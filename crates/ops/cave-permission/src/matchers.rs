// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Casbin built-in matcher operators.
//!
//! Line-port of casbin v3.10.0 `util/builtin_operators.go` (Apache-2.0):
//! `KeyMatch` / `KeyMatch2` / `KeyMatch3` / `RegexMatch` / `IPMatch`.
//!
//! These pure functions are the primitives Casbin matchers are built from, e.g.
//! `m = keyMatch2(r.obj, p.obj) && r.act == p.act`. Upstream panics on malformed
//! input (invalid regex / non-IP); here we treat malformed input as a non-match
//! so a bad policy line can never crash the authorizer.

use std::net::IpAddr;

use ipnet::IpNet;
use regex::Regex;

/// `KeyMatch` — `key1` matches `key2`, where `key2` may end with a `*` wildcard
/// that matches any suffix. Upstream: `KeyMatch` in util/builtin_operators.go.
///
/// ```text
/// key_match("/foo/bar", "/foo/*") == true
/// key_match("/foo/bar", "/baz/*") == false
/// ```
pub fn key_match(key1: &str, key2: &str) -> bool {
    match key2.find('*') {
        None => key1 == key2,
        Some(i) => {
            if key1.len() > i {
                key1.as_bytes()[..i] == key2.as_bytes()[..i]
            } else {
                key1 == &key2[..i]
            }
        }
    }
}

/// `KeyMatch2` — like `KeyMatch` but `:name` segments match exactly one path
/// segment (`[^/]+`) and `/*` expands to `/.*`. Matched anchored end-to-end.
/// Upstream: `KeyMatch2` in util/builtin_operators.go.
pub fn key_match2(key1: &str, key2: &str) -> bool {
    let key2 = key2.replace("/*", "/.*");
    // `:name` => `[^/]+`
    let re = Regex::new(r":[^/]+").expect("static regex");
    let key2 = re.replace_all(&key2, "[^/]+");
    regex_match(key1, &format!("^{}$", key2))
}

/// `KeyMatch3` — like `KeyMatch2` but the named-parameter syntax is `{name}`.
/// Upstream: `KeyMatch3` in util/builtin_operators.go.
pub fn key_match3(key1: &str, key2: &str) -> bool {
    let key2 = key2.replace("/*", "/.*");
    // `{name}` => `[^/]+`
    let re = Regex::new(r"\{[^/]+\}").expect("static regex");
    let key2 = re.replace_all(&key2, "[^/]+");
    regex_match(key1, &format!("^{}$", key2))
}

/// `RegexMatch` — `key1` matches the regular expression `key2`.
/// Upstream: `RegexMatch` in util/builtin_operators.go. A malformed pattern
/// yields `false` (upstream panics).
pub fn regex_match(key1: &str, key2: &str) -> bool {
    match Regex::new(key2) {
        Ok(re) => re.is_match(key1),
        Err(_) => false,
    }
}

/// `IPMatch` — `ip1` matches `ip2`, where `ip2` is either a bare IP (exact
/// match) or a CIDR block (containment). Supports IPv4 and IPv6.
/// Upstream: `IPMatch` in util/builtin_operators.go. Malformed input yields
/// `false` (upstream panics).
pub fn ip_match(ip1: &str, ip2: &str) -> bool {
    let addr: IpAddr = match ip1.parse() {
        Ok(a) => a,
        Err(_) => return false,
    };
    // Try CIDR first; fall back to exact IP equality.
    if let Ok(net) = ip2.parse::<IpNet>() {
        net.contains(&addr)
    } else if let Ok(other) = ip2.parse::<IpAddr>() {
        addr == other
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_match_basics() {
        assert!(key_match("/foo/bar", "/foo/*"));
        assert!(!key_match("/foo/bar", "/baz/*"));
        assert!(key_match("/foo", "/foo"));
    }

    #[test]
    fn key_match2_single_segment() {
        assert!(key_match2("/foo/bar", "/foo/:id"));
        assert!(!key_match2("/foo/bar/baz", "/foo/:id"));
    }

    #[test]
    fn ip_match_cidr() {
        assert!(ip_match("192.168.2.123", "192.168.2.0/24"));
        assert!(!ip_match("192.168.3.1", "192.168.2.0/24"));
    }
}
