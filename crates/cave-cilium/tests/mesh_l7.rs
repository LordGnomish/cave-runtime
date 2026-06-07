// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Headline acceptance test: no-sidecar L7 service-mesh routing (RED→GREEN).
//!
//! Builds an Envoy-style route configuration (the shape cilium synthesises
//! from CiliumEnvoyConfig / Gateway API) and asserts first-match-wins
//! routing by authority + path + method + header, plus L7 policy
//! enforcement at the proxy (allow → 200, deny → 403).

use cave_cilium::mesh::{
    HeaderMatch, HttpRequest, L7Policy, PathMatch, ProxyPorts, Route, RouteConfiguration,
    RouteMatch, VirtualHost,
};
use cave_cilium::policy::HttpRule;

fn config() -> RouteConfiguration {
    RouteConfiguration {
        virtual_hosts: vec![VirtualHost {
            domains: vec!["api.example.com".into()],
            routes: vec![
                // Most specific first — Envoy evaluates in order.
                Route {
                    name: "v1-canary".into(),
                    match_: RouteMatch {
                        path: PathMatch::Prefix("/api/v1".into()),
                        method: Some("GET".into()),
                        headers: vec![HeaderMatch {
                            name: "x-canary".into(),
                            value: Some("true".into()),
                        }],
                    },
                    cluster: "v1-canary".into(),
                    prefix_rewrite: None,
                },
                Route {
                    name: "v1".into(),
                    match_: RouteMatch {
                        path: PathMatch::Prefix("/api/v1".into()),
                        method: None,
                        headers: vec![],
                    },
                    cluster: "v1".into(),
                    prefix_rewrite: Some("/".into()),
                },
                Route {
                    name: "frontend".into(),
                    match_: RouteMatch {
                        path: PathMatch::Prefix("/".into()),
                        method: None,
                        headers: vec![],
                    },
                    cluster: "frontend".into(),
                    prefix_rewrite: None,
                },
            ],
        }],
    }
}

#[test]
fn routes_first_match_wins_by_path_method_header() {
    let cfg = config();

    // Header + method + path all match the canary route.
    let r = cfg
        .route(&HttpRequest::new(
            "GET",
            "api.example.com",
            "/api/v1/users",
            &[("x-canary", "true")],
        ))
        .unwrap();
    assert_eq!(r.cluster, "v1-canary");

    // Same path, no canary header → falls through to the v1 route.
    let r = cfg
        .route(&HttpRequest::new("GET", "api.example.com", "/api/v1/users", &[]))
        .unwrap();
    assert_eq!(r.cluster, "v1");
    assert_eq!(r.prefix_rewrite.as_deref(), Some("/"));

    // Unrelated path → the catch-all frontend route.
    let r = cfg
        .route(&HttpRequest::new("GET", "api.example.com", "/home", &[]))
        .unwrap();
    assert_eq!(r.cluster, "frontend");

    // Unknown authority → no virtual host matches.
    assert!(cfg
        .route(&HttpRequest::new("GET", "other.example.com", "/", &[]))
        .is_none());
}

#[test]
fn wildcard_domain_matches_any_authority() {
    let cfg = RouteConfiguration {
        virtual_hosts: vec![VirtualHost {
            domains: vec!["*".into()],
            routes: vec![Route {
                name: "catch".into(),
                match_: RouteMatch {
                    path: PathMatch::Prefix("/".into()),
                    method: None,
                    headers: vec![],
                },
                cluster: "default".into(),
                prefix_rewrite: None,
            }],
        }],
    };
    assert_eq!(
        cfg.route(&HttpRequest::new("GET", "whatever.svc", "/x", &[]))
            .unwrap()
            .cluster,
        "default"
    );
}

#[test]
fn exact_and_regex_path_matches() {
    let exact = PathMatch::Exact("/healthz".into());
    assert!(exact.matches("/healthz"));
    assert!(!exact.matches("/healthz/extra"));

    let re = PathMatch::Regex("/api/.*/status".into());
    assert!(re.matches("/api/v2/status"));
    assert!(re.matches("/api/anything/status"));
    assert!(!re.matches("/api/status"));
}

#[test]
fn l7_policy_enforces_http_rules_at_proxy() {
    // Allow only GET /api and any POST to /api/submit.
    let pol = L7Policy::new(vec![
        HttpRule {
            method: Some("GET".into()),
            path: Some("/api".into()),
            headers: vec![],
        },
        HttpRule {
            method: Some("POST".into()),
            path: Some("/api/submit".into()),
            headers: vec![],
        },
    ]);

    let allowed = pol.enforce(&HttpRequest::new("GET", "svc", "/api", &[]));
    assert!(allowed.allowed);
    assert_eq!(allowed.status, 200);

    // Method mismatch → 403.
    let denied = pol.enforce(&HttpRequest::new("POST", "svc", "/api", &[]));
    assert!(!denied.allowed);
    assert_eq!(denied.status, 403);

    // Path mismatch → 403.
    assert!(!pol.enforce(&HttpRequest::new("GET", "svc", "/secret", &[])).allowed);

    // POST /api/submit matches the second rule.
    assert!(pol.enforce(&HttpRequest::new("POST", "svc", "/api/submit", &[])).allowed);
}

#[test]
fn empty_l7_policy_allows_all() {
    let pol = L7Policy::new(vec![]);
    assert!(pol.enforce(&HttpRequest::new("DELETE", "svc", "/anything", &[])).allowed);
}

#[test]
fn header_rule_requires_presence_and_value() {
    let pol = L7Policy::new(vec![HttpRule {
        method: None,
        path: None,
        headers: vec!["x-token".into()],
    }]);
    assert!(pol
        .enforce(&HttpRequest::new("GET", "svc", "/", &[("x-token", "abc")]))
        .allowed);
    assert!(!pol.enforce(&HttpRequest::new("GET", "svc", "/", &[])).allowed);
}

#[test]
fn proxy_ports_are_stable_and_in_range() {
    let mut pp = ProxyPorts::default();
    let a = pp.allocate("cnp/default/web:80");
    let b = pp.allocate("cnp/default/web:80");
    assert_eq!(a, b, "same key → same listener port");
    let c = pp.allocate("cnp/default/db:5432");
    assert_ne!(a, c);
    for p in [a, c] {
        assert!((10000..20000).contains(&p), "proxy port {} in range", p);
    }
}
