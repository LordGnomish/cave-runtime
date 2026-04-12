//! Route matching engine — host, path (prefix + regex), methods, headers, SNI.
//!
//! Priority order (highest first):
//!   1. `regex_priority` field (explicit user ordering)
//!   2. Regex paths beat prefix paths
//!   3. Longer (more specific) prefix paths
//!   4. Routes with more matcher criteria (host + method + headers)

use crate::models::Route;
use regex::Regex;
use std::collections::HashMap;

/// A single incoming request context for route matching.
#[derive(Debug, Clone)]
pub struct MatchContext<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub host: &'a str,
    pub sni: Option<&'a str>,
    pub headers: &'a HashMap<String, String>,
}

/// Determine whether a path pattern matches the given request path.
///
/// Regex patterns are prefixed with `~`.
/// Exact patterns are prefixed with `=`.
/// Everything else is a prefix match.
pub fn path_matches(pattern: &str, path: &str) -> bool {
    if let Some(regex_str) = pattern.strip_prefix('~') {
        match Regex::new(regex_str) {
            Ok(re) => re.is_match(path),
            Err(_) => false,
        }
    } else if let Some(exact) = pattern.strip_prefix('=') {
        path == exact
    } else {
        // Prefix match
        path == pattern
            || path.starts_with(&format!("{pattern}/"))
            || (pattern.ends_with('/') && path.starts_with(pattern.as_ref() as &str))
    }
}

/// Return `true` if the route matches the given context.
pub fn route_matches(route: &Route, ctx: &MatchContext) -> bool {
    // Method check (empty list = all methods allowed)
    if !route.methods.is_empty() {
        let method_upper = ctx.method.to_uppercase();
        if !route.methods.iter().any(|m| m.to_uppercase() == method_upper) {
            return false;
        }
    }

    // Host check (empty list = all hosts)
    if !route.hosts.is_empty() && !ctx.host.is_empty() {
        if !route.hosts.iter().any(|h| host_matches(h, ctx.host)) {
            return false;
        }
    }

    // SNI check (empty list = all SNIs)
    if !route.snis.is_empty() {
        match ctx.sni {
            None => return false,
            Some(sni) => {
                if !route.snis.iter().any(|s| s == sni) {
                    return false;
                }
            }
        }
    }

    // Header check
    for (header_name, expected_values) in &route.headers {
        let header_lower = header_name.to_lowercase();
        let actual = ctx
            .headers
            .iter()
            .find(|(k, _)| k.to_lowercase() == header_lower)
            .map(|(_, v)| v.as_str())
            .unwrap_or("");

        if !expected_values.iter().any(|ev| ev == actual) {
            return false;
        }
    }

    // Path check (empty list = all paths)
    if !route.paths.is_empty() {
        if !route.paths.iter().any(|p| path_matches(p, ctx.path)) {
            return false;
        }
    }

    true
}

/// Wildcard host matching: `*.example.com` matches `api.example.com`.
fn host_matches(pattern: &str, host: &str) -> bool {
    if let Some(suffix) = pattern.strip_prefix("*.") {
        host.ends_with(suffix) && host.len() > suffix.len()
    } else {
        pattern == host
    }
}

/// Score a route for priority sorting — higher score wins.
///
/// Scores are compared lexicographically as (regex_priority, is_regex, path_len, criteria_count).
pub fn route_score(route: &Route) -> (i32, i32, usize, usize) {
    let has_regex = route
        .paths
        .iter()
        .any(|p| p.starts_with('~') || p.starts_with('='));

    let is_regex_score = if has_regex { 1 } else { 0 };

    let max_path_len = route.paths.iter().map(|p| p.len()).max().unwrap_or(0);

    let criteria = route.methods.len()
        + route.hosts.len()
        + route.headers.len()
        + route.snis.len();

    (route.regex_priority, is_regex_score, max_path_len, criteria)
}

/// Find the best matching route from a slice of candidates.
pub fn find_best_match<'a>(
    routes: &'a [&'a Route],
    ctx: &MatchContext,
) -> Option<&'a Route> {
    let mut matched: Vec<&&Route> = routes
        .iter()
        .filter(|r| route_matches(r, ctx))
        .collect();

    if matched.is_empty() {
        return None;
    }

    // Sort descending by score
    matched.sort_by(|a, b| route_score(b).cmp(&route_score(a)));
    matched.first().copied().copied()
}

/// Compute the upstream path after applying `strip_path` rules.
pub fn compute_upstream_path(route: &Route, request_path: &str) -> String {
    if !route.strip_path {
        return request_path.to_string();
    }

    // Find the longest matching prefix and strip it
    let matched_prefix = route
        .paths
        .iter()
        .filter(|p| !p.starts_with('~') && !p.starts_with('='))
        .filter(|p| path_matches(p, request_path))
        .max_by_key(|p| p.len());

    match matched_prefix {
        Some(prefix) => {
            let stripped = request_path.strip_prefix(prefix.as_str()).unwrap_or("");
            if stripped.is_empty() {
                "/".to_string()
            } else if stripped.starts_with('/') {
                stripped.to_string()
            } else {
                format!("/{stripped}")
            }
        }
        None => request_path.to_string(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{PathHandling, Protocol, Route};
    use chrono::Utc;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn make_route(paths: Vec<&str>, methods: Vec<&str>) -> Route {
        Route {
            id: Uuid::new_v4(),
            name: None,
            service_id: Uuid::new_v4(),
            protocols: vec![Protocol::Http],
            methods: methods.iter().map(|s| s.to_string()).collect(),
            hosts: vec![],
            paths: paths.iter().map(|s| s.to_string()).collect(),
            headers: HashMap::new(),
            snis: vec![],
            strip_path: false,
            preserve_host: false,
            regex_priority: 0,
            path_handling: PathHandling::V0,
            tags: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn empty_headers() -> HashMap<String, String> {
        HashMap::new()
    }

    #[test]
    fn test_prefix_match_basic() {
        assert!(path_matches("/api", "/api/users"));
        assert!(path_matches("/api", "/api/users/123"));
        assert!(path_matches("/api", "/api"));
        assert!(!path_matches("/api", "/other"));
        assert!(!path_matches("/api", "/ap"));
    }

    #[test]
    fn test_regex_match() {
        assert!(path_matches("~/api/v[0-9]+/.*", "/api/v1/users"));
        assert!(path_matches("~/api/v[0-9]+/.*", "/api/v42/items"));
        assert!(!path_matches("~/api/v[0-9]+/.*", "/api/vX/users"));
    }

    #[test]
    fn test_exact_match() {
        assert!(path_matches("=/exact", "/exact"));
        assert!(!path_matches("=/exact", "/exact/more"));
        assert!(!path_matches("=/exact", "/other"));
    }

    #[test]
    fn test_method_filter() {
        let route = make_route(vec!["/api"], vec!["GET"]);
        let headers = empty_headers();

        let get_ctx = MatchContext {
            method: "GET",
            path: "/api/test",
            host: "",
            sni: None,
            headers: &headers,
        };
        assert!(route_matches(&route, &get_ctx));

        let post_ctx = MatchContext {
            method: "POST",
            path: "/api/test",
            host: "",
            sni: None,
            headers: &headers,
        };
        assert!(!route_matches(&route, &post_ctx));
    }

    #[test]
    fn test_host_match() {
        let mut route = make_route(vec!["/"], vec![]);
        route.hosts = vec!["api.example.com".to_string()];
        let headers = empty_headers();

        let ctx_match = MatchContext {
            method: "GET",
            path: "/test",
            host: "api.example.com",
            sni: None,
            headers: &headers,
        };
        assert!(route_matches(&route, &ctx_match));

        let ctx_no_match = MatchContext {
            method: "GET",
            path: "/test",
            host: "other.example.com",
            sni: None,
            headers: &headers,
        };
        assert!(!route_matches(&route, &ctx_no_match));
    }

    #[test]
    fn test_wildcard_host_match() {
        let mut route = make_route(vec!["/"], vec![]);
        route.hosts = vec!["*.example.com".to_string()];
        let headers = empty_headers();

        let ctx = MatchContext {
            method: "GET",
            path: "/",
            host: "api.example.com",
            sni: None,
            headers: &headers,
        };
        assert!(route_matches(&route, &ctx));

        let ctx_exact = MatchContext {
            method: "GET",
            path: "/",
            host: "example.com",
            sni: None,
            headers: &headers,
        };
        // "*.example.com" does not match "example.com" itself
        assert!(!route_matches(&route, &ctx_exact));
    }

    #[test]
    fn test_header_match() {
        let mut route = make_route(vec!["/"], vec![]);
        route.headers.insert(
            "X-Version".to_string(),
            vec!["v2".to_string(), "v3".to_string()],
        );
        let mut headers_match = HashMap::new();
        headers_match.insert("X-Version".to_string(), "v2".to_string());
        let mut headers_no_match = HashMap::new();
        headers_no_match.insert("X-Version".to_string(), "v1".to_string());

        let ctx_match = MatchContext {
            method: "GET",
            path: "/",
            host: "",
            sni: None,
            headers: &headers_match,
        };
        assert!(route_matches(&route, &ctx_match));

        let ctx_no = MatchContext {
            method: "GET",
            path: "/",
            host: "",
            sni: None,
            headers: &headers_no_match,
        };
        assert!(!route_matches(&route, &ctx_no));
    }

    #[test]
    fn test_sni_match() {
        let mut route = make_route(vec!["/"], vec![]);
        route.snis = vec!["secure.example.com".to_string()];
        let headers = empty_headers();

        let ctx_with_sni = MatchContext {
            method: "GET",
            path: "/",
            host: "",
            sni: Some("secure.example.com"),
            headers: &headers,
        };
        assert!(route_matches(&route, &ctx_with_sni));

        let ctx_no_sni = MatchContext {
            method: "GET",
            path: "/",
            host: "",
            sni: None,
            headers: &headers,
        };
        assert!(!route_matches(&route, &ctx_no_sni));
    }

    #[test]
    fn test_priority_ordering_longer_prefix_wins() {
        let short = make_route(vec!["/api"], vec![]);
        let long = make_route(vec!["/api/v2"], vec![]);
        let headers = empty_headers();

        let ctx = MatchContext {
            method: "GET",
            path: "/api/v2/users",
            host: "",
            sni: None,
            headers: &headers,
        };

        let routes = vec![&short, &long];
        let best = find_best_match(&routes, &ctx).unwrap();
        assert_eq!(best.paths, vec!["/api/v2"]);
    }

    #[test]
    fn test_strip_path() {
        let mut route = make_route(vec!["/api/v1"], vec![]);
        route.strip_path = true;

        assert_eq!(
            compute_upstream_path(&route, "/api/v1/users"),
            "/users"
        );
        assert_eq!(
            compute_upstream_path(&route, "/api/v1"),
            "/"
        );
    }
}
