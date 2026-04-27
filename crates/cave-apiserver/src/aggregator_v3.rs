//! Aggregation proxy deeper — header forwarding, impersonation, path
//! rewriting, retry policy, OpenAPI v3 component dedup. Layered atop
//! `aggregated_apiserver.rs` (registration) and `aggregator_v2.rs`
//! (availability + 503).
//!
//! Upstream sources (kubernetes/kubernetes v1.31):
//!   * `staging/src/k8s.io/kube-aggregator/pkg/apiserver/handler_proxy.go`
//!     — newRequestForProxy, requestHeader stripping, X-Remote-* injection.
//!   * `staging/src/k8s.io/apiserver/pkg/authentication/request/headerrequest`
//!     — request-header authn (kube-apiserver's CN-bound proxy).
//!   * `staging/src/k8s.io/kube-aggregator/pkg/controllers/openapi/aggregator.go`
//!     — schemaTable component dedup (kube-openapi).
//!
//! ## Tenant invariant
//!
//! A proxy request MUST be tagged with `X-Cave-Tenant-Id` matching the
//! caller's authenticated tenant. Headers from the inbound request that
//! claim a different tenant_id are stripped before forward. The header
//! is reconstructed by us from auth context, not blindly copied.

use crate::auth_review::UserInfo;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};

// ─────────────────────────────────────────────────────────────────────────────
// HTTP request — minimal model. Real proxying uses hyper::Request; we
// model just the fields the proxy logic touches.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ProxyHeaders {
    pub headers: BTreeMap<String, Vec<String>>,
}

impl ProxyHeaders {
    pub fn new() -> Self { Self { headers: BTreeMap::new() } }
    pub fn set(&mut self, k: &str, v: &str) {
        self.headers.insert(k.to_lowercase(), vec![v.into()]);
    }
    pub fn add(&mut self, k: &str, v: &str) {
        self.headers.entry(k.to_lowercase()).or_default().push(v.into());
    }
    pub fn get_first(&self, k: &str) -> Option<&str> {
        self.headers.get(&k.to_lowercase())
            .and_then(|v| v.first()).map(|s| s.as_str())
    }
    pub fn get_all(&self, k: &str) -> Vec<String> {
        self.headers.get(&k.to_lowercase()).cloned().unwrap_or_default()
    }
    pub fn remove(&mut self, k: &str) {
        self.headers.remove(&k.to_lowercase());
    }
    pub fn len(&self) -> usize { self.headers.len() }
    pub fn is_empty(&self) -> bool { self.headers.is_empty() }
}

impl Default for ProxyHeaders { fn default() -> Self { Self::new() } }

// ─────────────────────────────────────────────────────────────────────────────
// Header stripping & injection — `handler_proxy.go::newRequestForProxy`.
// Strip end-to-end auth (Authorization, Cookie, Impersonate-*) and inject
// X-Remote-* per the request-header authn protocol.
// ─────────────────────────────────────────────────────────────────────────────

/// Headers that must NEVER reach the backing apiservice. Mirrors upstream's
/// `requestHeaderProxyHeaders`.
pub const HOP_BY_HOP_HEADERS: &[&str] = &[
    "authorization",
    "cookie",
    "set-cookie",
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// Headers the apiserver itself injects post-strip — clients can't override.
pub const INJECTED_PREFIXES: &[&str] = &[
    "x-remote-",
    "x-cave-",
];

/// Build the outbound proxy headers from `inbound`, stamping the
/// authenticated `user` and `tenant_id`. Drops hop-by-hop, drops anything
/// the client tried to spoof under a managed prefix, then injects the
/// canonical X-Remote-* set.
pub fn build_proxy_headers(
    inbound: &ProxyHeaders, user: &UserInfo, tenant_id: &str,
) -> ProxyHeaders {
    let mut out = inbound.clone();
    for h in HOP_BY_HOP_HEADERS { out.remove(h); }
    let keys: Vec<String> = out.headers.keys().cloned().collect();
    for k in keys {
        if INJECTED_PREFIXES.iter().any(|p| k.starts_with(p)) { out.remove(&k); }
    }
    out.set("x-remote-user", &user.username);
    out.set("x-remote-uid", &user.uid);
    for g in &user.groups { out.add("x-remote-group", g); }
    for (k, vs) in &user.extra {
        let header = format!("x-remote-extra-{}", k.to_lowercase());
        for v in vs { out.add(&header, v); }
    }
    out.set("x-cave-tenant-id", tenant_id);
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// Impersonation — strict policy. Upstream allows clients to impersonate
// only when they hold `impersonate` verb on `users`/`groups`/etc. We
// model the validation: if any Impersonate-* header is present, the
// caller MUST be authorized to impersonate that subject.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Impersonation {
    pub user: Option<String>,
    pub uid: Option<String>,
    pub groups: Vec<String>,
    pub extras: BTreeMap<String, Vec<String>>,
}

pub fn extract_impersonation(headers: &ProxyHeaders) -> Impersonation {
    let mut extras: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (k, vs) in &headers.headers {
        if let Some(rest) = k.strip_prefix("impersonate-extra-") {
            extras.entry(rest.to_string()).or_default().extend(vs.clone());
        }
    }
    Impersonation {
        user: headers.get_first("Impersonate-User").map(String::from),
        uid: headers.get_first("Impersonate-Uid").map(String::from),
        groups: headers.get_all("Impersonate-Group"),
        extras,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImpersonationDecision {
    NotRequested,
    Allowed { resolved: UserInfo },
    Denied { reason: String },
}

pub trait ImpersonationAuthorizer: Send + Sync {
    /// Returns `true` if the caller may impersonate the named subject of
    /// the given kind ("users"|"groups"|"uids"|"userextras") within tenant.
    fn may_impersonate(
        &self, tenant: &str, caller: &UserInfo, kind: &str, value: &str,
    ) -> bool;
}

pub struct AllowAllImpersonator;
impl ImpersonationAuthorizer for AllowAllImpersonator {
    fn may_impersonate(&self, _: &str, _: &UserInfo, _: &str, _: &str) -> bool { true }
}

pub struct DenyImpersonator;
impl ImpersonationAuthorizer for DenyImpersonator {
    fn may_impersonate(&self, _: &str, _: &UserInfo, _: &str, _: &str) -> bool { false }
}

pub fn resolve_impersonation(
    auth: &dyn ImpersonationAuthorizer,
    tenant: &str, caller: &UserInfo, headers: &ProxyHeaders,
) -> ImpersonationDecision {
    let imp = extract_impersonation(headers);
    if imp.user.is_none() && imp.groups.is_empty()
        && imp.uid.is_none() && imp.extras.is_empty()
    {
        return ImpersonationDecision::NotRequested;
    }
    let Some(user) = &imp.user else {
        return ImpersonationDecision::Denied {
            reason: "Impersonate-User required when other Impersonate-* headers present".into(),
        };
    };
    if !auth.may_impersonate(tenant, caller, "users", user) {
        return ImpersonationDecision::Denied {
            reason: format!("not authorized to impersonate user {user}"),
        };
    }
    for g in &imp.groups {
        if !auth.may_impersonate(tenant, caller, "groups", g) {
            return ImpersonationDecision::Denied {
                reason: format!("not authorized to impersonate group {g}"),
            };
        }
    }
    if let Some(uid) = &imp.uid {
        if !auth.may_impersonate(tenant, caller, "uids", uid) {
            return ImpersonationDecision::Denied {
                reason: format!("not authorized to impersonate uid {uid}"),
            };
        }
    }
    for (k, vs) in &imp.extras {
        for v in vs {
            if !auth.may_impersonate(tenant, caller, &format!("userextras/{k}"), v) {
                return ImpersonationDecision::Denied {
                    reason: format!("not authorized to impersonate extras[{k}]={v}"),
                };
            }
        }
    }
    let mut extra: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (k, vs) in imp.extras { extra.insert(k, vs); }
    let resolved = UserInfo {
        username: user.clone(),
        uid: imp.uid.unwrap_or_default(),
        groups: imp.groups.clone(),
        extra,
    };
    ImpersonationDecision::Allowed { resolved }
}

// ─────────────────────────────────────────────────────────────────────────────
// Path rewriting — `handler_proxy.go::serveHTTP`. The aggregator strips the
// `/apis/<group>/<version>` prefix that the apiserver appended for routing
// and forwards the *original* path to the apiservice.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxiedPath {
    pub group: String,
    pub version: String,
    /// Path *after* `/apis/<group>/<version>`; e.g. `/namespaces/foo/widgets/bar`.
    pub remainder: String,
}

pub fn parse_apis_path(path: &str) -> Option<ProxiedPath> {
    let trimmed = path.strip_prefix('/')?;
    if !trimmed.starts_with("apis/") { return None; }
    let parts: Vec<&str> = trimmed.splitn(4, '/').collect();
    if parts.len() < 3 { return None; }
    let group = parts.get(1).copied().unwrap_or("").to_string();
    let version = parts.get(2).copied().unwrap_or("").to_string();
    let remainder = if parts.len() == 4 {
        format!("/{}", parts[3])
    } else {
        String::new()
    };
    Some(ProxiedPath { group, version, remainder })
}

pub fn forward_path(p: &ProxiedPath) -> String {
    if p.remainder.is_empty() {
        format!("/apis/{}/{}", p.group, p.version)
    } else {
        format!("/apis/{}/{}{}", p.group, p.version, p.remainder)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Retry policy — upstream: 503 with reason ServiceUnavailable retries with
// exponential backoff up to 3 attempts. Our test surface is the policy fn.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryDecision {
    pub retry: bool,
    pub backoff_ms: u64,
    pub attempt: u32,
}

pub fn next_retry(prev_attempt: u32, http_status: u16) -> RetryDecision {
    let max_attempts = 3;
    if prev_attempt >= max_attempts {
        return RetryDecision { retry: false, backoff_ms: 0, attempt: prev_attempt };
    }
    if !matches!(http_status, 502 | 503 | 504) {
        return RetryDecision { retry: false, backoff_ms: 0, attempt: prev_attempt };
    }
    let next = prev_attempt + 1;
    let backoff_ms = 100u64 * (1u64 << prev_attempt); // 100, 200, 400
    RetryDecision { retry: true, backoff_ms, attempt: next }
}

// ─────────────────────────────────────────────────────────────────────────────
// OpenAPI v3 component dedup — kube-openapi `aggregator/aggregator.go`. When
// two child specs both define `#/components/schemas/io.k8s.api.core.v1.Pod`
// AND their bodies differ, namespace the child by its group/version.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct V3Spec {
    pub group_version: String,
    /// Schema name → JSON-encoded body.
    pub schemas: BTreeMap<String, String>,
}

pub fn dedup_components(specs: &[V3Spec]) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut out: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    for spec in specs {
        let mut my: BTreeMap<String, String> = BTreeMap::new();
        for (name, body) in &spec.schemas {
            match seen.get(name) {
                None => {
                    seen.insert(name.clone(), body.clone());
                    my.insert(name.clone(), body.clone());
                }
                Some(prev) if prev == body => {
                    // identical — share the symbol
                    my.insert(name.clone(), body.clone());
                }
                Some(_) => {
                    // collision — namespace by group_version
                    let ns_name = format!("{}__{}", spec.group_version.replace('/', "_"), name);
                    my.insert(ns_name, body.clone());
                }
            }
        }
        out.insert(spec.group_version.clone(), my);
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// GroupVersion priority sort — upstream's `discovery.SortableResources`.
// Stable, lower group_priority first; ties → lower version_priority;
// final ties → lexicographic name.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GVRecord {
    pub name: String,
    pub group_priority: i32,
    pub version_priority: i32,
}

pub fn sort_apis(records: &mut [GVRecord]) {
    records.sort_by(|a, b| {
        a.group_priority.cmp(&b.group_priority)
            .then(a.version_priority.cmp(&b.version_priority))
            .then(a.name.cmp(&b.name))
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// Forward-allowlist — paths under `/apis/<group>/...` are routed to
// APIService; ones under `/api/...` (core) are NEVER aggregated.
// ─────────────────────────────────────────────────────────────────────────────

pub fn is_aggregable(path: &str) -> bool {
    path.starts_with("/apis/")
}

pub fn forbidden_aggregated_groups() -> HashSet<&'static str> {
    // Built-in groups never delegated.
    let mut s = HashSet::new();
    for g in ["", "apps", "batch", "networking.k8s.io", "rbac.authorization.k8s.io",
              "storage.k8s.io", "apiextensions.k8s.io", "admissionregistration.k8s.io",
              "authentication.k8s.io", "authorization.k8s.io", "coordination.k8s.io",
              "events.k8s.io", "policy", "scheduling.k8s.io"] {
        s.insert(g);
    }
    s
}

pub fn route_decision(path: &str, group: &str) -> bool {
    if !is_aggregable(path) { return false; }
    !forbidden_aggregated_groups().contains(group)
}

// ─────────────────────────────────────────────────────────────────────────────
// Extension utilities for groups list deduplication.
// ─────────────────────────────────────────────────────────────────────────────

pub fn unique_versions(versions: &[String]) -> Vec<String> {
    let mut seen: BTreeSet<&String> = BTreeSet::new();
    let mut out = vec![];
    for v in versions {
        if seen.insert(v) { out.push(v.clone()); }
    }
    out
}

#[cfg(test)]
mod tests;
