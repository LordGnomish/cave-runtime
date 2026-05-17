// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Route matching engine — Kong-compatible scoring.
//!
//! Priority rules (higher = better match):
//!   1. Longer matching prefix / exact path beats shorter
//!   2. regex_priority field
//!   3. Specific host beats wildcard
//!   4. More methods/headers specified beats fewer

use crate::models::{PathHandling, Protocol, Route};
use regex::Regex;
use std::collections::HashMap;

/// Compiled, cached version of a Route for fast matching.
#[derive(Debug)]
pub struct CompiledRoute {
    pub route_id: uuid::Uuid,
    pub service_id: Option<uuid::Uuid>,
    pub regex_priority: i64,
    pub strip_path: bool,
    pub preserve_host: bool,
    pub path_handling: PathHandling,
    pub protocols: Vec<Protocol>,
    pub methods: Option<Vec<String>>,
    pub hosts: Option<Vec<HostPattern>>,
    pub paths: Option<Vec<PathPattern>>,
    pub headers: Option<HashMap<String, Vec<String>>>,
    pub snis: Option<Vec<String>>,
}

#[derive(Debug)]
pub enum HostPattern {
    Exact(String),
    Wildcard(String), // *.example.com
}

#[derive(Debug)]
pub enum PathPattern {
    Exact(String),
    Prefix(String),
    Regex(Regex, String), // (compiled, original)
}

impl HostPattern {
    pub fn matches(&self, host: &str) -> bool {
        match self {
            HostPattern::Exact(h) => h.eq_ignore_ascii_case(host),
            HostPattern::Wildcard(suffix) => {
                // suffix is ".example.com" (starts with .)
                host.ends_with(suffix.as_str()) && host.len() > suffix.len()
            }
        }
    }

    pub fn specificity(&self) -> i64 {
        match self {
            HostPattern::Exact(_) => 10,
            HostPattern::Wildcard(_) => 1,
        }
    }
}

impl PathPattern {
    pub fn matches(&self, path: &str) -> Option<usize> {
        // Returns matched length for scoring
        match self {
            PathPattern::Exact(p) => {
                if path == p.as_str() { Some(p.len()) } else { None }
            }
            PathPattern::Prefix(p) => {
                if path.starts_with(p.as_str()) { Some(p.len()) } else { None }
            }
            PathPattern::Regex(re, _) => {
                if re.is_match(path) { Some(0) } else { None }
            }
        }
    }
}

/// Parse a route into its compiled form.
pub fn compile_route(route: &Route) -> CompiledRoute {
    let hosts = route.hosts.as_ref().map(|hs| {
        hs.iter()
            .map(|h| {
                if h.starts_with("*.") {
                    HostPattern::Wildcard(h[1..].to_string())
                } else {
                    HostPattern::Exact(h.clone())
                }
            })
            .collect()
    });

    let paths = route.paths.as_ref().map(|ps| {
        ps.iter()
            .map(|p| {
                if p.starts_with('~') {
                    // Kong uses ~ prefix for regex paths
                    let pattern = &p[1..];
                    match Regex::new(pattern) {
                        Ok(re) => PathPattern::Regex(re, p.clone()),
                        Err(_) => PathPattern::Prefix(p.clone()),
                    }
                } else if p.ends_with('*') {
                    // treat trailing * as prefix
                    PathPattern::Prefix(p[..p.len() - 1].to_string())
                } else {
                    // Kong default: prefix match
                    PathPattern::Prefix(p.clone())
                }
            })
            .collect()
    });

    CompiledRoute {
        route_id: route.id,
        service_id: route.service_id,
        regex_priority: route.regex_priority,
        strip_path: route.strip_path,
        preserve_host: route.preserve_host,
        path_handling: route.path_handling.clone(),
        protocols: route.protocols.clone(),
        methods: route.methods.clone(),
        hosts,
        paths,
        headers: route.headers.clone(),
        snis: route.snis.clone(),
    }
}

#[derive(Debug, Clone)]
pub struct MatchResult {
    pub route_id: uuid::Uuid,
    pub service_id: Option<uuid::Uuid>,
    pub strip_path: bool,
    pub preserve_host: bool,
    pub matched_path_len: usize,
    pub score: i64,
    pub path_handling: PathHandling,
}

/// Attempt to match an incoming request against compiled routes.
/// Returns the best-scored match, if any.
pub fn match_request(
    compiled: &[CompiledRoute],
    method: &str,
    host: &str,
    path: &str,
    headers: &HashMap<String, String>,
    protocol: &Protocol,
    sni: Option<&str>,
) -> Option<MatchResult> {
    let mut best: Option<MatchResult> = None;

    'outer: for cr in compiled {
        // Protocol must match
        if !cr.protocols.iter().any(|p| p == protocol) {
            continue;
        }

        // Method check
        if let Some(methods) = &cr.methods {
            if !methods.iter().any(|m| m.eq_ignore_ascii_case(method)) {
                continue 'outer;
            }
        }

        // SNI check
        if let Some(snis) = &cr.snis {
            let incoming_sni = sni.unwrap_or("");
            if !snis.iter().any(|s| s == incoming_sni) {
                continue 'outer;
            }
        }

        // Host check
        let mut host_score = 0i64;
        if let Some(host_patterns) = &cr.hosts {
            let mut matched = false;
            for hp in host_patterns {
                if hp.matches(host) {
                    host_score += hp.specificity();
                    matched = true;
                    break;
                }
            }
            if !matched {
                continue 'outer;
            }
        }

        // Header check
        if let Some(required_headers) = &cr.headers {
            for (hdr_name, allowed_values) in required_headers {
                let incoming = headers.get(&hdr_name.to_lowercase());
                let matches = match incoming {
                    Some(v) => allowed_values.iter().any(|av| {
                        // Kong supports regex values here
                        if av.starts_with("~*") {
                            Regex::new(&av[2..]).map(|re| re.is_match(v)).unwrap_or(false)
                        } else {
                            av == v
                        }
                    }),
                    None => false,
                };
                if !matches {
                    continue 'outer;
                }
            }
        }

        // Path check
        let mut path_len = 0usize;
        if let Some(path_patterns) = &cr.paths {
            let mut matched = false;
            for pp in path_patterns {
                if let Some(len) = pp.matches(path) {
                    path_len = path_len.max(len);
                    matched = true;
                }
            }
            if !matched {
                continue 'outer;
            }
        }

        // Score = regex_priority * 1000 + host_specificity * 100 + path_length
        let score =
            cr.regex_priority * 1_000 + host_score * 100 + path_len as i64;

        let result = MatchResult {
            route_id: cr.route_id,
            service_id: cr.service_id,
            strip_path: cr.strip_path,
            preserve_host: cr.preserve_host,
            matched_path_len: path_len,
            score,
            path_handling: cr.path_handling.clone(),
        };

        match &best {
            None => best = Some(result),
            Some(b) if score > b.score => best = Some(result),
            _ => {}
        }
    }

    best
}

/// Compute the upstream path after optional strip_path.
pub fn upstream_path(
    route: &MatchResult,
    request_path: &str,
    service_path: Option<&str>,
) -> String {
    let base = if route.strip_path && route.matched_path_len > 0 {
        &request_path[route.matched_path_len..]
    } else {
        request_path
    };

    let base = if base.is_empty() { "/" } else { base };

    match service_path {
        None | Some("/") | Some("") => base.to_string(),
        Some(sp) => {
            if sp.ends_with('/') && base.starts_with('/') {
                format!("{}{}", &sp[..sp.len() - 1], base)
            } else {
                format!("{}{}", sp, base)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{PathHandling, Protocol, Route};
    use uuid::Uuid;

    fn make_route(paths: Vec<&str>, methods: Option<Vec<&str>>, hosts: Option<Vec<&str>>) -> CompiledRoute {
        let mut r = Route::new(Uuid::new_v4());
        r.paths = Some(paths.into_iter().map(String::from).collect());
        r.methods = methods.map(|ms| ms.into_iter().map(String::from).collect());
        r.hosts = hosts.map(|hs| hs.into_iter().map(String::from).collect());
        compile_route(&r)
    }

    #[test]
    fn test_prefix_match() {
        let compiled = vec![make_route(vec!["/api"], None, None)];
        let hdrs = HashMap::new();
        let res = match_request(&compiled, "GET", "example.com", "/api/users", &hdrs, &Protocol::Http, None);
        assert!(res.is_some());
        assert_eq!(res.unwrap().matched_path_len, 4);
    }

    #[test]
    fn test_no_match_wrong_path() {
        let compiled = vec![make_route(vec!["/api"], None, None)];
        let hdrs = HashMap::new();
        let res = match_request(&compiled, "GET", "example.com", "/other", &hdrs, &Protocol::Http, None);
        assert!(res.is_none());
    }

    #[test]
    fn test_host_match() {
        let compiled = vec![make_route(vec!["/"], None, Some(vec!["api.example.com"]))];
        let hdrs = HashMap::new();
        let res = match_request(&compiled, "GET", "api.example.com", "/foo", &hdrs, &Protocol::Http, None);
        assert!(res.is_some());
    }

    #[test]
    fn test_wildcard_host() {
        let compiled = vec![make_route(vec!["/"], None, Some(vec!["*.example.com"]))];
        let hdrs = HashMap::new();
        let res = match_request(&compiled, "GET", "api.example.com", "/foo", &hdrs, &Protocol::Http, None);
        assert!(res.is_some());
        let res2 = match_request(&compiled, "GET", "other.io", "/foo", &hdrs, &Protocol::Http, None);
        assert!(res2.is_none());
    }

    #[test]
    fn test_method_filter() {
        let compiled = vec![make_route(vec!["/"], Some(vec!["POST"]), None)];
        let hdrs = HashMap::new();
        let res = match_request(&compiled, "GET", "example.com", "/", &hdrs, &Protocol::Http, None);
        assert!(res.is_none());
        let res2 = match_request(&compiled, "POST", "example.com", "/", &hdrs, &Protocol::Http, None);
        assert!(res2.is_some());
    }

    #[test]
    fn test_longer_prefix_wins() {
        let mut r1 = Route::new(Uuid::new_v4());
        r1.paths = Some(vec!["/api".to_string()]);
        r1.regex_priority = 0;
        let mut r2 = Route::new(Uuid::new_v4());
        r2.paths = Some(vec!["/api/v1".to_string()]);
        r2.regex_priority = 0;

        let compiled = vec![compile_route(&r1), compile_route(&r2)];
        let hdrs = HashMap::new();
        let res = match_request(&compiled, "GET", "x", "/api/v1/users", &hdrs, &Protocol::Http, None)
            .unwrap();
        assert_eq!(res.route_id, r2.id);
    }

    #[test]
    fn test_upstream_path_strip() {
        let m = MatchResult {
            route_id: Uuid::new_v4(),
            service_id: None,
            strip_path: true,
            preserve_host: false,
            matched_path_len: 4,
            score: 0,
            path_handling: PathHandling::V0,
        };
        assert_eq!(upstream_path(&m, "/api/users", None), "/users");
    }

    #[test]
    fn test_upstream_path_no_strip() {
        let m = MatchResult {
            route_id: Uuid::new_v4(),
            service_id: None,
            strip_path: false,
            preserve_host: false,
            matched_path_len: 4,
            score: 0,
            path_handling: PathHandling::V0,
        };
        assert_eq!(upstream_path(&m, "/api/users", Some("/v2")), "/v2/api/users");
    }
}
