// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! No-sidecar L7 service mesh — route matching + proxy L7 enforcement.
//!
//! Cilium's service mesh runs a per-node Envoy proxy rather than a sidecar.
//! This module ports the control-plane shapes it drives:
//!   * An Envoy-style [`RouteConfiguration`] (the config cilium synthesises
//!     from `CiliumEnvoyConfig` / Gateway API `HTTPRoute`): virtual hosts
//!     matched by `:authority`, then routes matched **first-wins** by path
//!     (prefix / exact / regex), method, and headers — exactly Envoy's
//!     `route_matcher` order.
//!   * [`L7Policy`], the HTTP allow-list cilium's proxy enforces for L7
//!     network policy: a request is allowed iff it matches some rule
//!     (`pkg/proxy` / `pkg/policy/api/rule_http`), otherwise 403.
//!   * [`ProxyPorts`], the per-listener proxy-port allocator
//!     (`pkg/proxy/proxyports`) in the 10000–20000 range.
//!
//! The path regex uses a faithful `.`/`*` full-match engine (the same
//! semantics Envoy's `safe_regex` anchors), not a substring hack.

use std::collections::HashMap;

use crate::policy::HttpRule;

/// A minimal HTTP request as seen by the proxy.
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: String,
    pub authority: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
}

impl HttpRequest {
    pub fn new(method: &str, authority: &str, path: &str, headers: &[(&str, &str)]) -> Self {
        HttpRequest {
            method: method.to_string(),
            authority: authority.to_string(),
            path: path.to_string(),
            headers: headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Path matcher (Envoy `route.match.path_specifier`).
#[derive(Debug, Clone)]
pub enum PathMatch {
    Prefix(String),
    Exact(String),
    Regex(String),
}

impl PathMatch {
    pub fn matches(&self, path: &str) -> bool {
        match self {
            PathMatch::Prefix(p) => path.starts_with(p.as_str()),
            PathMatch::Exact(p) => path == p,
            PathMatch::Regex(re) => regex_full_match(re, path),
        }
    }
}

/// Header matcher: presence (`value == None`) or exact value.
#[derive(Debug, Clone)]
pub struct HeaderMatch {
    pub name: String,
    pub value: Option<String>,
}

impl HeaderMatch {
    fn matches(&self, req: &HttpRequest) -> bool {
        match (&self.value, req.header(&self.name)) {
            (Some(want), Some(got)) => want == got,
            (None, Some(_)) => true,
            _ => false,
        }
    }
}

/// A route match condition.
#[derive(Debug, Clone)]
pub struct RouteMatch {
    pub path: PathMatch,
    pub method: Option<String>,
    pub headers: Vec<HeaderMatch>,
}

impl RouteMatch {
    fn matches(&self, req: &HttpRequest) -> bool {
        if !self.path.matches(&req.path) {
            return false;
        }
        if let Some(m) = &self.method {
            if !m.eq_ignore_ascii_case(&req.method) {
                return false;
            }
        }
        self.headers.iter().all(|h| h.matches(req))
    }
}

/// A route: match → upstream cluster, optional prefix rewrite.
#[derive(Debug, Clone)]
pub struct Route {
    pub name: String,
    pub match_: RouteMatch,
    pub cluster: String,
    pub prefix_rewrite: Option<String>,
}

/// A virtual host: domains + ordered routes.
#[derive(Debug, Clone)]
pub struct VirtualHost {
    pub domains: Vec<String>,
    pub routes: Vec<Route>,
}

impl VirtualHost {
    fn matches_authority(&self, authority: &str) -> bool {
        self.domains.iter().any(|d| domain_match(d, authority))
    }
}

/// Wildcard-aware domain match (`*`, `*.suffix`, or exact, case-insensitive).
fn domain_match(domain: &str, authority: &str) -> bool {
    if domain == "*" {
        return true;
    }
    if let Some(suffix) = domain.strip_prefix("*.") {
        return authority
            .to_ascii_lowercase()
            .ends_with(&format!(".{}", suffix.to_ascii_lowercase()))
            || authority.eq_ignore_ascii_case(suffix);
    }
    domain.eq_ignore_ascii_case(authority)
}

/// An Envoy-style route configuration.
#[derive(Debug, Clone, Default)]
pub struct RouteConfiguration {
    pub virtual_hosts: Vec<VirtualHost>,
}

impl RouteConfiguration {
    /// Resolve a request to a route: first matching virtual host (by
    /// authority), then the first matching route within it (Envoy order).
    pub fn route(&self, req: &HttpRequest) -> Option<&Route> {
        let vh = self
            .virtual_hosts
            .iter()
            .find(|vh| vh.matches_authority(&req.authority))?;
        vh.routes.iter().find(|r| r.match_.matches(req))
    }
}

/// The verdict the proxy returns for an L7 request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct L7Verdict {
    pub allowed: bool,
    pub status: u16,
}

/// The HTTP allow-list enforced by the proxy for L7 network policy.
#[derive(Debug, Clone, Default)]
pub struct L7Policy {
    rules: Vec<HttpRule>,
}

impl L7Policy {
    pub fn new(rules: Vec<HttpRule>) -> Self {
        L7Policy { rules }
    }

    /// Allow iff the request matches some rule. An empty rule list means
    /// "allow all L7" (the toPorts-without-rules case in cilium).
    pub fn enforce(&self, req: &HttpRequest) -> L7Verdict {
        if self.rules.is_empty() || self.rules.iter().any(|r| http_rule_matches(r, req)) {
            L7Verdict {
                allowed: true,
                status: 200,
            }
        } else {
            L7Verdict {
                allowed: false,
                status: 403,
            }
        }
    }
}

/// One HTTP rule matches a request: method (exact), path (anchored regex),
/// and every named header present.
fn http_rule_matches(rule: &HttpRule, req: &HttpRequest) -> bool {
    if let Some(m) = &rule.method {
        if !m.eq_ignore_ascii_case(&req.method) {
            return false;
        }
    }
    if let Some(p) = &rule.path {
        if !regex_full_match(p, &req.path) {
            return false;
        }
    }
    rule.headers.iter().all(|h| req.header(h).is_some())
}

/// Per-listener proxy-port allocator (`pkg/proxy/proxyports`, 10000–20000).
#[derive(Debug)]
pub struct ProxyPorts {
    next: u16,
    by_key: HashMap<String, u16>,
}

impl Default for ProxyPorts {
    fn default() -> Self {
        ProxyPorts {
            next: 10000,
            by_key: HashMap::new(),
        }
    }
}

impl ProxyPorts {
    /// Allocate (or return the existing) listener port for `key`.
    pub fn allocate(&mut self, key: &str) -> u16 {
        if let Some(p) = self.by_key.get(key) {
            return *p;
        }
        let p = self.next;
        self.next += 1;
        self.by_key.insert(key.to_string(), p);
        p
    }

    pub fn get(&self, key: &str) -> Option<u16> {
        self.by_key.get(key).copied()
    }
}

/// Anchored full-match regex supporting `.` (any char) and `*` (Kleene on
/// the preceding element) — Envoy `safe_regex` semantics for the subset
/// cilium emits. Classic recursive matcher; literal chars match themselves.
pub fn regex_full_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let s: Vec<char> = text.chars().collect();
    is_match(&p, &s)
}

fn is_match(p: &[char], s: &[char]) -> bool {
    if p.is_empty() {
        return s.is_empty();
    }
    let first = !s.is_empty() && (p[0] == s[0] || p[0] == '.');
    if p.len() >= 2 && p[1] == '*' {
        // Zero occurrences of p[0], or one more occurrence then retry.
        is_match(&p[2..], s) || (first && is_match(p, &s[1..]))
    } else if first {
        is_match(&p[1..], &s[1..])
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regex_dot_star_full_match() {
        assert!(regex_full_match("a.c", "abc"));
        assert!(regex_full_match("a.*c", "axyzc"));
        assert!(regex_full_match("a.*c", "ac"));
        assert!(!regex_full_match("a.c", "abbc"));
        assert!(!regex_full_match("abc", "abcd"));
    }

    #[test]
    fn suffix_wildcard_domain() {
        assert!(domain_match("*.example.com", "api.example.com"));
        assert!(domain_match("*.example.com", "example.com"));
        assert!(!domain_match("*.example.com", "example.org"));
    }
}
