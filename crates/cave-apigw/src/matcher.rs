// SPDX-License-Identifier: AGPL-3.0-or-later
//! Match predicates — path / host / method / header / regex.
//!
//! Kong's `kong.router.atc_compat` reformulates each predicate as a small
//! conjunction; the Rust analogue lives here.

use crate::models::HeaderMatch;
use regex::Regex;

/// Match the request method (case-insensitive). Empty `expected` matches any.
pub fn method_matches(expected: &[String], actual: &str) -> bool {
    if expected.is_empty() {
        return true;
    }
    expected.iter().any(|m| m.eq_ignore_ascii_case(actual))
}

/// Match the request host. Supports wildcard prefix (`*.example.com`).
/// Empty `expected` matches any.
pub fn host_matches(expected: &[String], actual: &str) -> bool {
    if expected.is_empty() {
        return true;
    }
    let actual = actual.to_lowercase();
    expected.iter().any(|h| {
        let h = h.to_lowercase();
        if let Some(suffix) = h.strip_prefix("*.") {
            actual.ends_with(suffix) && actual.len() > suffix.len()
        } else {
            h == actual
        }
    })
}

/// Path-match outcome — both whether the path matched, and what prefix to strip.
pub struct PathMatch {
    pub matched: bool,
    pub matched_prefix_len: usize,
}

/// Match a request path. Patterns starting with `~/` are treated as regex (Kong syntax).
/// Returns the longest matched prefix length so callers can strip it.
pub fn path_matches(expected: &[String], actual: &str) -> PathMatch {
    if expected.is_empty() {
        return PathMatch { matched: true, matched_prefix_len: 0 };
    }
    let mut best = 0usize;
    let mut hit = false;
    for p in expected {
        if let Some(re_src) = p.strip_prefix("~/") {
            if let Ok(re) = Regex::new(&format!("^{re_src}")) {
                if let Some(m) = re.find(actual) {
                    hit = true;
                    if m.end() > best {
                        best = m.end();
                    }
                }
            }
        } else if actual == p || actual.starts_with(&format!("{p}/")) || actual.starts_with(p) {
            // Kong allows non-anchored prefix match — but only at a `/` boundary or exact.
            if actual == p {
                hit = true;
                if p.len() > best { best = p.len(); }
            } else if actual.starts_with(&format!("{p}/")) {
                hit = true;
                if p.len() > best { best = p.len(); }
            }
        }
    }
    PathMatch { matched: hit, matched_prefix_len: best }
}

/// Match a set of header predicates against a multimap of incoming headers.
/// Header names are normalised to lower-case; values use case-insensitive equality.
pub fn header_matches(expected: &[HeaderMatch], actual: &[(String, String)]) -> bool {
    expected.iter().all(|hm| {
        let name = hm.name.to_lowercase();
        let present: Vec<&str> = actual
            .iter()
            .filter(|(n, _)| n.to_lowercase() == name)
            .map(|(_, v)| v.as_str())
            .collect();
        if hm.values.is_empty() {
            return !present.is_empty();
        }
        hm.values.iter().any(|want| {
            present
                .iter()
                .any(|got| got.eq_ignore_ascii_case(want))
        })
    })
}

/// Match a TLS Server Name Indication value.
pub fn sni_matches(expected: &[String], actual: &str) -> bool {
    if expected.is_empty() {
        return true;
    }
    let a = actual.to_lowercase();
    expected.iter().any(|s| s.to_lowercase() == a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn method_match_basic() {
        assert!(method_matches(&[], "GET"));
        assert!(method_matches(&["GET".into()], "get"));
        assert!(!method_matches(&["POST".into()], "GET"));
    }

    #[test]
    fn host_exact_and_wildcard() {
        assert!(host_matches(&["api.example.com".into()], "api.example.com"));
        assert!(host_matches(&["*.example.com".into()], "api.example.com"));
        assert!(!host_matches(&["*.example.com".into()], "example.com"));
        assert!(!host_matches(&["api.example.com".into()], "x.example.com"));
    }

    #[test]
    fn path_prefix_match() {
        let pm = path_matches(&["/api".into()], "/api/v1/users");
        assert!(pm.matched);
        assert_eq!(pm.matched_prefix_len, 4);
    }

    #[test]
    fn path_exact_match() {
        let pm = path_matches(&["/health".into()], "/health");
        assert!(pm.matched);
        assert_eq!(pm.matched_prefix_len, 7);
    }

    #[test]
    fn path_regex_match() {
        let pm = path_matches(&["~/^/v(\\d+)/.*".into()], "/v2/users");
        assert!(pm.matched);
        assert!(pm.matched_prefix_len > 0);
    }

    #[test]
    fn header_required_and_value_match() {
        let hm = HeaderMatch {
            name: "X-Tenant".into(),
            values: vec!["alpha".into()],
        };
        let actual = vec![("x-tenant".into(), "ALPHA".into())];
        assert!(header_matches(&[hm], &actual));
    }

    #[test]
    fn header_presence_only() {
        let hm = HeaderMatch {
            name: "X-Trace".into(),
            values: vec![],
        };
        let actual = vec![("x-trace".into(), "abc".into())];
        assert!(header_matches(&[hm], &actual));
    }

    #[test]
    fn sni_match() {
        assert!(sni_matches(&["api.example.com".into()], "API.example.com"));
        assert!(!sni_matches(&["api.example.com".into()], "other.com"));
    }

    #[test]
    fn empty_expected_always_matches() {
        assert!(method_matches(&[], "X"));
        assert!(host_matches(&[], "x"));
        assert!(sni_matches(&[], "x"));
        assert!(path_matches(&[], "/x").matched);
    }
}
