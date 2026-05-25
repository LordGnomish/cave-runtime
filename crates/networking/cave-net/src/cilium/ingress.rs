// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Ingress controller — Cilium's Envoy-backed L7 gateway.
//!
//! Mirrors `pkg/ingress/ingress.go` (cilium-ingress-controller) plus the
//! Gateway-API translation in `pkg/gateway-api/translation`.
//!
//! Two CRDs are supported (faithful to upstream):
//!
//! * Core `networking.k8s.io/v1.Ingress` — host + path rules, default
//!   backend, optional TLS secret per host.
//! * `gateway.networking.k8s.io/v1.HTTPRoute` + `v1alpha2.TLSRoute` —
//!   richer matching (headers, query params), weighted backends, SNI.
//!
//! LB modes (mirrors `cilium.io/lb-ipam-ips` annotation):
//!
//! * [`LbMode::Shared`] — every ingress shares one LB IP (the cluster
//!   default).
//! * [`LbMode::Dedicated`] — each ingress gets its own LB IP from the
//!   IPAM pool.
//!
//! Path types follow upstream `Ingress`:
//!
//! * `Exact` — full path equality.
//! * `Prefix` — `/foo` matches `/foo`, `/foo/`, `/foo/bar`; longest-prefix
//!   match wins when multiple paths could match.
//! * `ImplementationSpecific` — Cilium treats it as `Prefix`.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LbMode {
    Shared,
    Dedicated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PathType {
    Exact,
    Prefix,
    ImplementationSpecific,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendRef {
    pub service: String,
    pub namespace: String,
    pub port: u16,
    pub weight: u32,
}

impl BackendRef {
    pub fn new(ns: impl Into<String>, svc: impl Into<String>, port: u16) -> Self {
        Self {
            namespace: ns.into(),
            service: svc.into(),
            port,
            weight: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngressPath {
    pub path: String,
    pub path_type: PathType,
    pub backend: BackendRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngressRule {
    /// Host header to match. `None` means *any host*.
    pub host: Option<String>,
    pub paths: Vec<IngressPath>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TlsConfig {
    pub hosts: Vec<String>,
    pub secret_name: String,
    pub secret_namespace: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ingress {
    pub name: String,
    pub namespace: String,
    pub tenant: TenantId,
    pub class: String,
    pub lb_mode: LbMode,
    pub rules: Vec<IngressRule>,
    pub default_backend: Option<BackendRef>,
    pub tls: Vec<TlsConfig>,
}

impl Ingress {
    pub fn new(name: impl Into<String>, ns: impl Into<String>, tenant: TenantId) -> Self {
        Self {
            name: name.into(),
            namespace: ns.into(),
            tenant,
            class: "cilium".into(),
            lb_mode: LbMode::Shared,
            rules: Vec::new(),
            default_backend: None,
            tls: Vec::new(),
        }
    }
    pub fn key(&self) -> String {
        format!("{}/{}", self.namespace, self.name)
    }
}

// ── HTTPRoute (Gateway API) ──────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeaderMatch {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryParamMatch {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpRouteMatch {
    pub path_prefix: Option<String>,
    pub path_exact: Option<String>,
    pub headers: Vec<HeaderMatch>,
    pub query_params: Vec<QueryParamMatch>,
}

impl HttpRouteMatch {
    pub fn matches(
        &self,
        host: &str,
        path: &str,
        headers: &[(String, String)],
        query: &[(String, String)],
    ) -> bool {
        let _ = host;
        if let Some(p) = &self.path_exact {
            if p != path {
                return false;
            }
        }
        if let Some(p) = &self.path_prefix {
            if !path.starts_with(p.as_str()) {
                return false;
            }
        }
        for h in &self.headers {
            if !headers
                .iter()
                .any(|(k, v)| k.eq_ignore_ascii_case(&h.name) && v == &h.value)
            {
                return false;
            }
        }
        for q in &self.query_params {
            if !query.iter().any(|(k, v)| k == &q.name && v == &q.value) {
                return false;
            }
        }
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpRouteRule {
    pub matches: Vec<HttpRouteMatch>,
    pub backends: Vec<BackendRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpRoute {
    pub name: String,
    pub namespace: String,
    pub tenant: TenantId,
    pub hostnames: Vec<String>,
    pub rules: Vec<HttpRouteRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TlsRoute {
    pub name: String,
    pub namespace: String,
    pub tenant: TenantId,
    pub sni_hostnames: Vec<String>,
    pub backends: Vec<BackendRef>,
}

// ── Manager ──────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IngressError {
    #[error("ingress `{0}` has no rules and no default backend")]
    EmptyIngress(String),
    #[error("path `{0}` is not absolute (must start with `/`)")]
    BadPath(String),
    #[error("ingress `{0}` not found")]
    NotFound(String),
    #[error("tenant {tenant} cannot mutate ingress owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug, Default)]
pub struct IngressManager {
    ingresses: HashMap<String, Ingress>,
    http_routes: HashMap<String, HttpRoute>,
    tls_routes: HashMap<String, TlsRoute>,
    /// Per-ingress LB IP (Dedicated mode).
    dedicated_lb_ips: HashMap<String, IpAddr>,
    /// Cluster-wide shared LB IP.
    pub shared_lb_ip: Option<IpAddr>,
}

impl IngressManager {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn len(&self) -> usize {
        self.ingresses.len()
    }
    pub fn is_empty(&self) -> bool {
        self.ingresses.is_empty() && self.http_routes.is_empty() && self.tls_routes.is_empty()
    }

    pub fn upsert_ingress(&mut self, ing: Ingress) -> Result<(), IngressError> {
        if ing.rules.is_empty() && ing.default_backend.is_none() {
            return Err(IngressError::EmptyIngress(ing.key()));
        }
        for r in &ing.rules {
            for p in &r.paths {
                if !p.path.starts_with('/') {
                    return Err(IngressError::BadPath(p.path.clone()));
                }
            }
        }
        let key = ing.key();
        if matches!(ing.lb_mode, LbMode::Dedicated) && !self.dedicated_lb_ips.contains_key(&key) {
            // Allocate a dedicated IP from a fictitious 192.0.2.0/24 pool.
            let next = 100 + self.dedicated_lb_ips.len() as u8;
            self.dedicated_lb_ips.insert(
                key.clone(),
                IpAddr::V4(std::net::Ipv4Addr::new(192, 0, 2, next)),
            );
        }
        self.ingresses.insert(key, ing);
        Ok(())
    }

    pub fn remove_ingress(&mut self, key: &str) -> Result<(), IngressError> {
        if self.ingresses.remove(key).is_none() {
            return Err(IngressError::NotFound(key.to_string()));
        }
        self.dedicated_lb_ips.remove(key);
        Ok(())
    }

    pub fn lb_ip_for(&self, key: &str) -> Option<IpAddr> {
        let ing = self.ingresses.get(key)?;
        match ing.lb_mode {
            LbMode::Shared => self.shared_lb_ip,
            LbMode::Dedicated => self.dedicated_lb_ips.get(key).copied(),
        }
    }

    /// Route a request through the registered Ingresses. Filters by class
    /// (only `cilium`-class ingresses are evaluated).
    pub fn route(&self, host: &str, path: &str) -> Option<&BackendRef> {
        let mut best: Option<(usize, &BackendRef)> = None;
        let mut default: Option<&BackendRef> = None;

        for ing in self.ingresses.values() {
            if ing.class != "cilium" {
                continue;
            }
            if let Some(db) = &ing.default_backend {
                default.get_or_insert(db);
            }
            for rule in &ing.rules {
                if let Some(h) = &rule.host {
                    if h != host {
                        continue;
                    }
                }
                for p in &rule.paths {
                    let m = match p.path_type {
                        PathType::Exact => p.path == path,
                        PathType::Prefix | PathType::ImplementationSpecific => {
                            path == p.path
                                || path.starts_with(&format!("{}/", p.path.trim_end_matches('/')))
                                || (p.path == "/" && !path.is_empty())
                        }
                    };
                    if m {
                        let len = p.path.len();
                        match best {
                            Some((blen, _)) if len <= blen => {}
                            _ => best = Some((len, &p.backend)),
                        }
                    }
                }
            }
        }
        best.map(|(_, b)| b).or(default)
    }

    /// Look up the TLS secret for a hostname (mirrors envoy SDS lookup).
    pub fn tls_secret_for(&self, host: &str) -> Option<(&str, &str)> {
        for ing in self.ingresses.values() {
            for tls in &ing.tls {
                if tls.hosts.iter().any(|h| h == host) {
                    return Some((&tls.secret_namespace, &tls.secret_name));
                }
            }
        }
        None
    }

    // ── Gateway API ─────────────────────────────────────────────────────────

    pub fn upsert_http_route(&mut self, r: HttpRoute) {
        let key = format!("{}/{}", r.namespace, r.name);
        self.http_routes.insert(key, r);
    }

    pub fn upsert_tls_route(&mut self, r: TlsRoute) {
        let key = format!("{}/{}", r.namespace, r.name);
        self.tls_routes.insert(key, r);
    }

    pub fn route_http(
        &self,
        host: &str,
        path: &str,
        headers: &[(String, String)],
        query: &[(String, String)],
    ) -> Option<&BackendRef> {
        for r in self.http_routes.values() {
            if !r.hostnames.is_empty() && !r.hostnames.iter().any(|h| h == host) {
                continue;
            }
            for rule in &r.rules {
                let any_match = rule.matches.is_empty()
                    || rule
                        .matches
                        .iter()
                        .any(|m| m.matches(host, path, headers, query));
                if any_match {
                    // Pick highest-weight backend (deterministic).
                    return rule.backends.iter().max_by_key(|b| b.weight);
                }
            }
        }
        None
    }

    pub fn route_tls_sni(&self, sni: &str) -> Option<&BackendRef> {
        for r in self.tls_routes.values() {
            if r.sni_hostnames.iter().any(|h| h == sni) {
                return r.backends.first();
            }
        }
        None
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/ingress/ingress.go", "Manager");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn basic_ingress(tenant: TenantId) -> Ingress {
        let mut ing = Ingress::new("api", "default", tenant);
        ing.rules.push(IngressRule {
            host: Some("api.example.com".into()),
            paths: vec![IngressPath {
                path: "/v1".into(),
                path_type: PathType::Prefix,
                backend: BackendRef::new("default", "api-svc", 8080),
            }],
        });
        ing
    }

    // ── Validation ───────────────────────────────────────────────────────────

    #[test]
    fn ing_upsert_with_no_rules_and_no_default_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ingress/ingress.go",
            "Manager.Validate",
            "tenant-ing-empty"
        );
        let mut mgr = IngressManager::new();
        let ing = Ingress::new("e", "default", tenant);
        let err = mgr.upsert_ingress(ing).unwrap_err();
        assert_eq!(err, IngressError::EmptyIngress("default/e".into()));
    }

    #[test]
    fn ing_upsert_with_relative_path_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ingress/ingress.go",
            "Manager.Validate.Path",
            "tenant-ing-relpath"
        );
        let mut mgr = IngressManager::new();
        let mut ing = basic_ingress(tenant);
        ing.rules[0].paths[0].path = "v1".into();
        let err = mgr.upsert_ingress(ing).unwrap_err();
        assert_eq!(err, IngressError::BadPath("v1".into()));
    }

    // ── Path-type routing ────────────────────────────────────────────────────

    #[test]
    fn ing_route_prefix_matches_subpaths() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ingress/ingress.go",
            "Manager.Route.Prefix",
            "tenant-ing-pref"
        );
        let mut mgr = IngressManager::new();
        mgr.upsert_ingress(basic_ingress(tenant)).unwrap();
        let b = mgr.route("api.example.com", "/v1/users").unwrap();
        assert_eq!(b.service, "api-svc");
    }

    #[test]
    fn ing_route_prefix_does_not_match_other_subpath() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ingress/ingress.go",
            "Manager.Route.PrefixBoundary",
            "tenant-ing-prefb"
        );
        let mut mgr = IngressManager::new();
        mgr.upsert_ingress(basic_ingress(tenant)).unwrap();
        let b = mgr.route("api.example.com", "/v2/users");
        assert!(b.is_none());
    }

    #[test]
    fn ing_route_prefix_distinguishes_v1_and_v10() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ingress/ingress.go",
            "Manager.Route.PrefixDistinct",
            "tenant-ing-prefd"
        );
        let mut mgr = IngressManager::new();
        mgr.upsert_ingress(basic_ingress(tenant)).unwrap();
        // /v1 prefix should NOT match /v10 (Cilium / k8s-ingress is segment-aware).
        let b = mgr.route("api.example.com", "/v10/users");
        assert!(b.is_none());
    }

    #[test]
    fn ing_route_exact_path_match() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ingress/ingress.go",
            "Manager.Route.Exact",
            "tenant-ing-exact"
        );
        let mut mgr = IngressManager::new();
        let mut ing = basic_ingress(tenant);
        ing.rules[0].paths[0].path = "/health".into();
        ing.rules[0].paths[0].path_type = PathType::Exact;
        mgr.upsert_ingress(ing).unwrap();
        assert!(mgr.route("api.example.com", "/health").is_some());
        assert!(mgr.route("api.example.com", "/health/x").is_none());
    }

    #[test]
    fn ing_route_longest_prefix_wins() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ingress/ingress.go",
            "Manager.Route.LongestPrefix",
            "tenant-ing-lp"
        );
        let mut mgr = IngressManager::new();
        let mut ing = basic_ingress(tenant);
        ing.rules[0].paths.push(IngressPath {
            path: "/v1/users".into(),
            path_type: PathType::Prefix,
            backend: BackendRef::new("default", "users-svc", 7000),
        });
        mgr.upsert_ingress(ing).unwrap();
        let b = mgr.route("api.example.com", "/v1/users/42").unwrap();
        assert_eq!(b.service, "users-svc");
    }

    #[test]
    fn ing_route_implementation_specific_treated_as_prefix() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ingress/ingress.go",
            "Manager.Route.ImplSpec",
            "tenant-ing-impl"
        );
        let mut mgr = IngressManager::new();
        let mut ing = basic_ingress(tenant);
        ing.rules[0].paths[0].path_type = PathType::ImplementationSpecific;
        mgr.upsert_ingress(ing).unwrap();
        assert!(mgr.route("api.example.com", "/v1/foo").is_some());
    }

    // ── Host header ──────────────────────────────────────────────────────────

    #[test]
    fn ing_route_filters_by_host_header() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ingress/ingress.go",
            "Manager.Route.Host",
            "tenant-ing-host"
        );
        let mut mgr = IngressManager::new();
        mgr.upsert_ingress(basic_ingress(tenant)).unwrap();
        assert!(mgr.route("api.example.com", "/v1/x").is_some());
        assert!(mgr.route("other.example.com", "/v1/x").is_none());
    }

    #[test]
    fn ing_route_rule_with_no_host_matches_any() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ingress/ingress.go",
            "Manager.Route.Host.AnyHost",
            "tenant-ing-anyhost"
        );
        let mut mgr = IngressManager::new();
        let mut ing = basic_ingress(tenant);
        ing.rules[0].host = None;
        mgr.upsert_ingress(ing).unwrap();
        assert!(mgr.route("any.example.com", "/v1/x").is_some());
    }

    // ── Default backend ──────────────────────────────────────────────────────

    #[test]
    fn ing_route_default_backend_for_unmatched_request() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ingress/ingress.go",
            "Manager.Route.DefaultBackend",
            "tenant-ing-def"
        );
        let mut mgr = IngressManager::new();
        let mut ing = basic_ingress(tenant);
        ing.default_backend = Some(BackendRef::new("default", "fallback", 8081));
        mgr.upsert_ingress(ing).unwrap();
        let b = mgr.route("other.example.com", "/anything").unwrap();
        assert_eq!(b.service, "fallback");
    }

    #[test]
    fn ing_route_no_match_no_default_returns_none() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ingress/ingress.go",
            "Manager.Route.NoMatch",
            "tenant-ing-nomatch"
        );
        let mut mgr = IngressManager::new();
        mgr.upsert_ingress(basic_ingress(tenant)).unwrap();
        assert!(mgr.route("other.example.com", "/anything").is_none());
    }

    // ── TLS ──────────────────────────────────────────────────────────────────

    #[test]
    fn ing_tls_secret_for_host() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ingress/ingress.go",
            "Manager.TLSSecret",
            "tenant-ing-tls"
        );
        let mut mgr = IngressManager::new();
        let mut ing = basic_ingress(tenant);
        ing.tls.push(TlsConfig {
            hosts: vec!["api.example.com".into()],
            secret_name: "api-cert".into(),
            secret_namespace: "default".into(),
        });
        mgr.upsert_ingress(ing).unwrap();
        let (ns, name) = mgr.tls_secret_for("api.example.com").unwrap();
        assert_eq!((ns, name), ("default", "api-cert"));
    }

    #[test]
    fn ing_tls_secret_unknown_host_returns_none() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ingress/ingress.go",
            "Manager.TLSSecret.NotFound",
            "tenant-ing-tlsnf"
        );
        let mut mgr = IngressManager::new();
        mgr.upsert_ingress(basic_ingress(tenant)).unwrap();
        assert!(mgr.tls_secret_for("api.example.com").is_none());
    }

    // ── LB modes ─────────────────────────────────────────────────────────────

    #[test]
    fn ing_lb_mode_shared_returns_cluster_default() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ingress/ingress.go",
            "Manager.LBMode.Shared",
            "tenant-ing-lbsh"
        );
        let mut mgr = IngressManager::new();
        mgr.shared_lb_ip = Some("203.0.113.10".parse().unwrap());
        mgr.upsert_ingress(basic_ingress(tenant)).unwrap();
        let lb = mgr.lb_ip_for("default/api").unwrap();
        assert_eq!(lb.to_string(), "203.0.113.10");
    }

    #[test]
    fn ing_lb_mode_dedicated_assigns_unique_ip_per_ingress() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ingress/ingress.go",
            "Manager.LBMode.Dedicated",
            "tenant-ing-lbded"
        );
        let mut mgr = IngressManager::new();
        let mut a = basic_ingress(tenant.clone());
        a.lb_mode = LbMode::Dedicated;
        mgr.upsert_ingress(a).unwrap();

        let mut b = Ingress::new("api2", "default", tenant);
        b.lb_mode = LbMode::Dedicated;
        b.rules.push(IngressRule {
            host: Some("api2.example.com".into()),
            paths: vec![IngressPath {
                path: "/".into(),
                path_type: PathType::Prefix,
                backend: BackendRef::new("default", "api2-svc", 8082),
            }],
        });
        mgr.upsert_ingress(b).unwrap();

        let lb_a = mgr.lb_ip_for("default/api").unwrap();
        let lb_b = mgr.lb_ip_for("default/api2").unwrap();
        assert_ne!(lb_a, lb_b);
    }

    // ── Class filter ─────────────────────────────────────────────────────────

    #[test]
    fn ing_route_skips_non_cilium_class() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ingress/ingress.go",
            "Manager.Route.Class",
            "tenant-ing-class"
        );
        let mut mgr = IngressManager::new();
        let mut ing = basic_ingress(tenant);
        ing.class = "nginx".into();
        mgr.upsert_ingress(ing).unwrap();
        assert!(mgr.route("api.example.com", "/v1/x").is_none());
    }

    // ── Lifecycle ────────────────────────────────────────────────────────────

    #[test]
    fn ing_remove_drops_routes() {
        let (_c, tenant) =
            cilium_test_ctx!("pkg/ingress/ingress.go", "Manager.Delete", "tenant-ing-rm");
        let mut mgr = IngressManager::new();
        mgr.upsert_ingress(basic_ingress(tenant)).unwrap();
        mgr.remove_ingress("default/api").unwrap();
        assert!(mgr.is_empty());
        assert!(mgr.route("api.example.com", "/v1/x").is_none());
    }

    #[test]
    fn ing_remove_unknown_returns_not_found() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/ingress/ingress.go",
            "Manager.Delete.NotFound",
            "tenant-ing-rmnf"
        );
        let mut mgr = IngressManager::new();
        let err = mgr.remove_ingress("default/missing").unwrap_err();
        assert_eq!(err, IngressError::NotFound("default/missing".into()));
    }

    // ── HTTPRoute (Gateway API) ──────────────────────────────────────────────

    #[test]
    fn ing_httproute_path_prefix_match() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/gateway-api/translation/translation.go",
            "HTTPRoute",
            "tenant-ing-httpr"
        );
        let mut mgr = IngressManager::new();
        mgr.upsert_http_route(HttpRoute {
            name: "r".into(),
            namespace: "default".into(),
            tenant,
            hostnames: vec!["api.example.com".into()],
            rules: vec![HttpRouteRule {
                matches: vec![HttpRouteMatch {
                    path_prefix: Some("/v1".into()),
                    path_exact: None,
                    headers: vec![],
                    query_params: vec![],
                }],
                backends: vec![BackendRef::new("default", "v1-svc", 8080)],
            }],
        });
        let b = mgr
            .route_http("api.example.com", "/v1/users", &[], &[])
            .unwrap();
        assert_eq!(b.service, "v1-svc");
    }

    #[test]
    fn ing_httproute_picks_highest_weight_backend() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/gateway-api/translation/translation.go",
            "HTTPRoute.Weights",
            "tenant-ing-httpw"
        );
        let mut mgr = IngressManager::new();
        mgr.upsert_http_route(HttpRoute {
            name: "r".into(),
            namespace: "default".into(),
            tenant,
            hostnames: vec!["api.example.com".into()],
            rules: vec![HttpRouteRule {
                matches: vec![HttpRouteMatch {
                    path_prefix: Some("/v1".into()),
                    path_exact: None,
                    headers: vec![],
                    query_params: vec![],
                }],
                backends: vec![
                    BackendRef {
                        service: "a".into(),
                        namespace: "default".into(),
                        port: 80,
                        weight: 1,
                    },
                    BackendRef {
                        service: "b".into(),
                        namespace: "default".into(),
                        port: 80,
                        weight: 9,
                    },
                ],
            }],
        });
        let b = mgr
            .route_http("api.example.com", "/v1/u", &[], &[])
            .unwrap();
        assert_eq!(b.service, "b");
    }

    #[test]
    fn ing_httproute_header_match_required() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/gateway-api/translation/translation.go",
            "HTTPRoute.Headers",
            "tenant-ing-httph"
        );
        let mut mgr = IngressManager::new();
        mgr.upsert_http_route(HttpRoute {
            name: "r".into(),
            namespace: "default".into(),
            tenant,
            hostnames: vec!["api.example.com".into()],
            rules: vec![HttpRouteRule {
                matches: vec![HttpRouteMatch {
                    path_prefix: Some("/".into()),
                    path_exact: None,
                    headers: vec![HeaderMatch {
                        name: "x-tenant".into(),
                        value: "acme".into(),
                    }],
                    query_params: vec![],
                }],
                backends: vec![BackendRef::new("default", "tenant-svc", 8080)],
            }],
        });
        assert!(mgr
            .route_http(
                "api.example.com",
                "/",
                &[("x-tenant".into(), "acme".into())],
                &[]
            )
            .is_some());
        assert!(mgr
            .route_http(
                "api.example.com",
                "/",
                &[("x-tenant".into(), "other".into())],
                &[]
            )
            .is_none());
    }

    #[test]
    fn ing_httproute_query_param_match_required() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/gateway-api/translation/translation.go",
            "HTTPRoute.Query",
            "tenant-ing-httpq"
        );
        let mut mgr = IngressManager::new();
        mgr.upsert_http_route(HttpRoute {
            name: "r".into(),
            namespace: "default".into(),
            tenant,
            hostnames: vec!["api.example.com".into()],
            rules: vec![HttpRouteRule {
                matches: vec![HttpRouteMatch {
                    path_prefix: Some("/".into()),
                    path_exact: None,
                    headers: vec![],
                    query_params: vec![QueryParamMatch {
                        name: "v".into(),
                        value: "2".into(),
                    }],
                }],
                backends: vec![BackendRef::new("default", "v2-svc", 8080)],
            }],
        });
        assert!(mgr
            .route_http("api.example.com", "/", &[], &[("v".into(), "2".into())])
            .is_some());
        assert!(mgr
            .route_http("api.example.com", "/", &[], &[("v".into(), "1".into())])
            .is_none());
    }

    // ── TLSRoute ─────────────────────────────────────────────────────────────

    #[test]
    fn ing_tls_route_sni_match() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/gateway-api/translation/translation.go",
            "TLSRoute",
            "tenant-ing-tlsr"
        );
        let mut mgr = IngressManager::new();
        mgr.upsert_tls_route(TlsRoute {
            name: "r".into(),
            namespace: "default".into(),
            tenant,
            sni_hostnames: vec!["secure.example.com".into()],
            backends: vec![BackendRef::new("default", "secure-svc", 8443)],
        });
        let b = mgr.route_tls_sni("secure.example.com").unwrap();
        assert_eq!(b.service, "secure-svc");
    }

    #[test]
    fn ing_tls_route_sni_mismatch_returns_none() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/gateway-api/translation/translation.go",
            "TLSRoute.NoMatch",
            "tenant-ing-tlsrn"
        );
        let mut mgr = IngressManager::new();
        mgr.upsert_tls_route(TlsRoute {
            name: "r".into(),
            namespace: "default".into(),
            tenant,
            sni_hostnames: vec!["secure.example.com".into()],
            backends: vec![BackendRef::new("default", "secure-svc", 8443)],
        });
        assert!(mgr.route_tls_sni("other.example.com").is_none());
    }

    // ── Serde ────────────────────────────────────────────────────────────────

    #[test]
    fn ing_round_trips_serde() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/k8s/apis/networking.k8s.io/v1.Ingress",
            "Spec",
            "tenant-ing-serde"
        );
        let ing = basic_ingress(tenant);
        let json = serde_json::to_string(&ing).unwrap();
        let back: Ingress = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ing);
    }
}
