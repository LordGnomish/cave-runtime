// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Local Redirect Policy (CiliumLocalRedirectPolicy) + node-local DNS
//! cache integration.
//!
//! Mirrors `pkg/redirectpolicy/manager.go` plus the LRP CRD shape from
//! `pkg/k8s/apis/cilium.io/v2/ciliumlocalredirectpolicy_types.go`.
//!
//! Semantics (faithful to upstream):
//!
//! * An LRP can redirect traffic by **service matcher** (intercept
//!   `ClusterIP` traffic for a specific service) or by **address
//!   matcher** (intercept any traffic to one or more `(ip, port)`
//!   tuples).
//! * The redirect target is a **local-only backend pod set** selected
//!   by labels — only pods on the same node as the source endpoint
//!   are considered.
//! * If no local backend exists, traffic falls through to the original
//!   destination (unless the policy is marked `skip_redirect_no_match`).
//! * Common use: node-local DNS cache. The LRP captures `kube-dns` →
//!   `node-local-dns` for UDP/53 and TCP/53.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum L4Proto {
    TCP,
    UDP,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortMatcher {
    pub port: u16,
    pub protocol: L4Proto,
    /// Optional renaming — the redirect maps the original port to this
    /// new target port. `None` keeps the same port.
    pub target_port: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LrpFrontend {
    /// Match a Kubernetes Service by namespace/name.
    Service {
        namespace: String,
        name: String,
        ports: Vec<PortMatcher>,
    },
    /// Match traffic to specific (ip, port, proto) tuples.
    Address { ip: IpAddr, ports: Vec<PortMatcher> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalBackendSelector {
    /// Match pods by these labels.
    pub match_labels: Vec<(String, String)>,
    /// Required namespace.
    pub namespace: String,
    pub ports: Vec<PortMatcher>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalRedirectPolicy {
    pub name: String,
    pub namespace: String,
    pub tenant: TenantId,
    pub frontend: LrpFrontend,
    pub backend: LocalBackendSelector,
    /// If true and no local backend matches, drop the packet rather
    /// than passing it through.
    pub skip_redirect_no_match: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalBackend {
    pub pod_name: String,
    pub pod_namespace: String,
    pub node_name: String,
    pub pod_ip: IpAddr,
    pub labels: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedirectDecision {
    pub policy_name: String,
    pub backend_ip: IpAddr,
    pub backend_pod: String,
    pub target_port: u16,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LrpError {
    #[error("policy `{0}` not found")]
    PolicyNotFound(String),
    #[error("policy `{0}` already exists")]
    Duplicate(String),
    #[error("frontend has no port matchers")]
    EmptyFrontendPorts,
    #[error("backend selector has no labels and no namespace")]
    EmptyBackendSelector,
    #[error("tenant {tenant} cannot mutate LRP store owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug, Default)]
pub struct LrpManager {
    policies: HashMap<String, LocalRedirectPolicy>,
    /// Per-node backend roster.
    backends: Vec<LocalBackend>,
}

impl LrpManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert_policy(&mut self, p: LocalRedirectPolicy) -> Result<(), LrpError> {
        match &p.frontend {
            LrpFrontend::Service { ports, .. } | LrpFrontend::Address { ports, .. } => {
                if ports.is_empty() {
                    return Err(LrpError::EmptyFrontendPorts);
                }
            }
        }
        if p.backend.match_labels.is_empty() && p.backend.namespace.is_empty() {
            return Err(LrpError::EmptyBackendSelector);
        }
        self.policies.insert(p.key(), p);
        Ok(())
    }

    pub fn remove_policy(&mut self, key: &str) -> Result<(), LrpError> {
        self.policies
            .remove(key)
            .ok_or_else(|| LrpError::PolicyNotFound(key.to_string()))?;
        Ok(())
    }

    pub fn policy_count(&self) -> usize {
        self.policies.len()
    }

    pub fn policy(&self, key: &str) -> Option<&LocalRedirectPolicy> {
        self.policies.get(key)
    }

    pub fn upsert_backend(&mut self, b: LocalBackend) {
        // Replace if pod already known.
        let key = format!("{}/{}", b.pod_namespace, b.pod_name);
        self.backends
            .retain(|x| format!("{}/{}", x.pod_namespace, x.pod_name) != key);
        self.backends.push(b);
    }

    pub fn remove_backend(&mut self, namespace: &str, name: &str) -> bool {
        let key = format!("{namespace}/{name}");
        let before = self.backends.len();
        self.backends
            .retain(|x| format!("{}/{}", x.pod_namespace, x.pod_name) != key);
        before != self.backends.len()
    }

    pub fn backend_count(&self) -> usize {
        self.backends.len()
    }

    /// Resolve a redirect for a packet from `node` heading to
    /// `(dst_ip, dst_port, proto)`. Returns the redirect decision or
    /// `None` if no policy applies (passthrough). If a policy applies
    /// but no local backend matches, returns `None` unless
    /// `skip_redirect_no_match` is true (in which case a `Denied`
    /// decision would be modelled — we return `None` here to keep the
    /// API a single Option, callers can check `is_blocked`).
    pub fn resolve(
        &self,
        node: &str,
        dst_ip: IpAddr,
        dst_port: u16,
        proto: L4Proto,
        service_lookup: Option<(&str, &str)>, // (namespace, service-name) if dst_ip resolves to a known ClusterIP
    ) -> Option<RedirectDecision> {
        for p in self.policies.values() {
            // Match the frontend.
            let target_port = match &p.frontend {
                LrpFrontend::Address { ip, ports } => {
                    if *ip != dst_ip {
                        continue;
                    }
                    match port_match(ports, dst_port, proto) {
                        Some(tp) => tp,
                        None => continue,
                    }
                }
                LrpFrontend::Service {
                    namespace,
                    name,
                    ports,
                } => {
                    let (svc_ns, svc_name) = match service_lookup {
                        Some(x) => x,
                        None => continue,
                    };
                    if svc_ns != namespace || svc_name != name {
                        continue;
                    }
                    match port_match(ports, dst_port, proto) {
                        Some(tp) => tp,
                        None => continue,
                    }
                }
            };
            // Find a local backend.
            if let Some(b) = self.pick_local_backend(node, &p.backend) {
                return Some(RedirectDecision {
                    policy_name: p.key(),
                    backend_ip: b.pod_ip,
                    backend_pod: format!("{}/{}", b.pod_namespace, b.pod_name),
                    target_port,
                });
            }
        }
        None
    }

    fn pick_local_backend<'a>(
        &'a self,
        node: &str,
        sel: &LocalBackendSelector,
    ) -> Option<&'a LocalBackend> {
        self.backends.iter().find(|b| {
            b.node_name == node
                && b.pod_namespace == sel.namespace
                && sel
                    .match_labels
                    .iter()
                    .all(|(k, v)| b.labels.iter().any(|(bk, bv)| bk == k && bv == v))
        })
    }
}

fn port_match(matchers: &[PortMatcher], port: u16, proto: L4Proto) -> Option<u16> {
    for m in matchers {
        if m.port == port && m.protocol == proto {
            return Some(m.target_port.unwrap_or(port));
        }
    }
    None
}

impl LocalRedirectPolicy {
    pub fn key(&self) -> String {
        format!("{}/{}", self.namespace, self.name)
    }
}

/// Convenience constructor for the canonical node-local DNS cache LRP:
/// captures `kube-system/kube-dns` UDP+TCP/53 and redirects to a local
/// `node-local-dns` pod with label `k8s-app=node-local-dns`.
pub fn make_node_local_dns_lrp(tenant: TenantId) -> LocalRedirectPolicy {
    LocalRedirectPolicy {
        name: "nodelocaldns".into(),
        namespace: "kube-system".into(),
        tenant,
        frontend: LrpFrontend::Service {
            namespace: "kube-system".into(),
            name: "kube-dns".into(),
            ports: vec![
                PortMatcher {
                    port: 53,
                    protocol: L4Proto::UDP,
                    target_port: Some(53),
                },
                PortMatcher {
                    port: 53,
                    protocol: L4Proto::TCP,
                    target_port: Some(53),
                },
            ],
        },
        backend: LocalBackendSelector {
            match_labels: vec![("k8s-app".into(), "node-local-dns".into())],
            namespace: "kube-system".into(),
            ports: vec![
                PortMatcher {
                    port: 53,
                    protocol: L4Proto::UDP,
                    target_port: None,
                },
                PortMatcher {
                    port: 53,
                    protocol: L4Proto::TCP,
                    target_port: None,
                },
            ],
        },
        skip_redirect_no_match: false,
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/redirectpolicy/manager.go", "Manager");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    fn dns_backend(node: &str) -> LocalBackend {
        LocalBackend {
            pod_name: format!("nldns-{node}"),
            pod_namespace: "kube-system".into(),
            node_name: node.into(),
            pod_ip: ip(10, 0, 1, 9),
            labels: vec![("k8s-app".into(), "node-local-dns".into())],
        }
    }

    // ── Validation ───────────────────────────────────────────────────────────

    #[test]
    fn lrp_upsert_with_empty_frontend_ports_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/redirectpolicy/manager.go",
            "Validate.EmptyPorts",
            "tenant-lrp-empports"
        );
        let mut m = LrpManager::new();
        let p = LocalRedirectPolicy {
            name: "x".into(),
            namespace: "ns".into(),
            tenant,
            frontend: LrpFrontend::Address {
                ip: ip(10, 0, 0, 1),
                ports: vec![],
            },
            backend: LocalBackendSelector {
                match_labels: vec![("a".into(), "b".into())],
                namespace: "ns".into(),
                ports: vec![],
            },
            skip_redirect_no_match: false,
        };
        let err = m.upsert_policy(p).unwrap_err();
        assert_eq!(err, LrpError::EmptyFrontendPorts);
    }

    #[test]
    fn lrp_upsert_with_empty_backend_selector_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/redirectpolicy/manager.go",
            "Validate.EmptyBackend",
            "tenant-lrp-empback"
        );
        let mut m = LrpManager::new();
        let p = LocalRedirectPolicy {
            name: "x".into(),
            namespace: "ns".into(),
            tenant,
            frontend: LrpFrontend::Address {
                ip: ip(10, 0, 0, 1),
                ports: vec![PortMatcher {
                    port: 80,
                    protocol: L4Proto::TCP,
                    target_port: None,
                }],
            },
            backend: LocalBackendSelector {
                match_labels: vec![],
                namespace: "".into(),
                ports: vec![],
            },
            skip_redirect_no_match: false,
        };
        let err = m.upsert_policy(p).unwrap_err();
        assert_eq!(err, LrpError::EmptyBackendSelector);
    }

    #[test]
    fn lrp_remove_unknown_returns_not_found() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/redirectpolicy/manager.go",
            "Remove.NotFound",
            "tenant-lrp-rmnf"
        );
        let mut m = LrpManager::new();
        let err = m.remove_policy("ns/x").unwrap_err();
        assert_eq!(err, LrpError::PolicyNotFound("ns/x".into()));
    }

    // ── Address matcher ──────────────────────────────────────────────────────

    #[test]
    fn lrp_address_matcher_redirects_to_local_backend() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/redirectpolicy/manager.go",
            "Resolve.AddressMatcher",
            "tenant-lrp-addr"
        );
        let mut m = LrpManager::new();
        let p = LocalRedirectPolicy {
            name: "to-local".into(),
            namespace: "ns".into(),
            tenant,
            frontend: LrpFrontend::Address {
                ip: ip(10, 96, 0, 53),
                ports: vec![PortMatcher {
                    port: 53,
                    protocol: L4Proto::UDP,
                    target_port: None,
                }],
            },
            backend: LocalBackendSelector {
                match_labels: vec![("k8s-app".into(), "node-local-dns".into())],
                namespace: "kube-system".into(),
                ports: vec![],
            },
            skip_redirect_no_match: false,
        };
        m.upsert_policy(p).unwrap();
        m.upsert_backend(dns_backend("node-a"));
        let r = m
            .resolve("node-a", ip(10, 96, 0, 53), 53, L4Proto::UDP, None)
            .unwrap();
        assert_eq!(r.backend_pod, "kube-system/nldns-node-a");
    }

    #[test]
    fn lrp_address_matcher_other_ip_passthrough() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/redirectpolicy/manager.go",
            "Resolve.NoMatch",
            "tenant-lrp-pt"
        );
        let mut m = LrpManager::new();
        let p = LocalRedirectPolicy {
            name: "to-local".into(),
            namespace: "ns".into(),
            tenant,
            frontend: LrpFrontend::Address {
                ip: ip(10, 96, 0, 53),
                ports: vec![PortMatcher {
                    port: 53,
                    protocol: L4Proto::UDP,
                    target_port: None,
                }],
            },
            backend: LocalBackendSelector {
                match_labels: vec![("k8s-app".into(), "x".into())],
                namespace: "ns".into(),
                ports: vec![],
            },
            skip_redirect_no_match: false,
        };
        m.upsert_policy(p).unwrap();
        let r = m.resolve("node-a", ip(8, 8, 8, 8), 53, L4Proto::UDP, None);
        assert!(r.is_none());
    }

    #[test]
    fn lrp_address_matcher_other_port_passthrough() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/redirectpolicy/manager.go",
            "Resolve.WrongPort",
            "tenant-lrp-wp"
        );
        let mut m = LrpManager::new();
        let p = LocalRedirectPolicy {
            name: "to-local".into(),
            namespace: "ns".into(),
            tenant,
            frontend: LrpFrontend::Address {
                ip: ip(10, 96, 0, 53),
                ports: vec![PortMatcher {
                    port: 53,
                    protocol: L4Proto::UDP,
                    target_port: None,
                }],
            },
            backend: LocalBackendSelector {
                match_labels: vec![("k8s-app".into(), "x".into())],
                namespace: "ns".into(),
                ports: vec![],
            },
            skip_redirect_no_match: false,
        };
        m.upsert_policy(p).unwrap();
        let r = m.resolve("node-a", ip(10, 96, 0, 53), 80, L4Proto::TCP, None);
        assert!(r.is_none());
    }

    #[test]
    fn lrp_address_matcher_proto_must_match() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/redirectpolicy/manager.go",
            "Resolve.WrongProto",
            "tenant-lrp-wproto"
        );
        let mut m = LrpManager::new();
        let p = LocalRedirectPolicy {
            name: "to-local".into(),
            namespace: "ns".into(),
            tenant,
            frontend: LrpFrontend::Address {
                ip: ip(10, 96, 0, 53),
                ports: vec![PortMatcher {
                    port: 53,
                    protocol: L4Proto::UDP,
                    target_port: None,
                }],
            },
            backend: LocalBackendSelector {
                match_labels: vec![("k8s-app".into(), "node-local-dns".into())],
                namespace: "kube-system".into(),
                ports: vec![],
            },
            skip_redirect_no_match: false,
        };
        m.upsert_policy(p).unwrap();
        m.upsert_backend(dns_backend("node-a"));
        let r = m.resolve("node-a", ip(10, 96, 0, 53), 53, L4Proto::TCP, None);
        assert!(r.is_none());
    }

    // ── Service matcher ──────────────────────────────────────────────────────

    #[test]
    fn lrp_service_matcher_redirects_for_matching_service() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/redirectpolicy/manager.go",
            "Resolve.ServiceMatcher",
            "tenant-lrp-svc"
        );
        let mut m = LrpManager::new();
        m.upsert_policy(make_node_local_dns_lrp(tenant)).unwrap();
        m.upsert_backend(dns_backend("node-a"));
        let r = m.resolve(
            "node-a",
            ip(10, 96, 0, 10),
            53,
            L4Proto::UDP,
            Some(("kube-system", "kube-dns")),
        );
        assert!(r.is_some());
    }

    #[test]
    fn lrp_service_matcher_wrong_service_passthrough() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/redirectpolicy/manager.go",
            "Resolve.ServiceMismatch",
            "tenant-lrp-svcm"
        );
        let mut m = LrpManager::new();
        m.upsert_policy(make_node_local_dns_lrp(tenant)).unwrap();
        m.upsert_backend(dns_backend("node-a"));
        let r = m.resolve(
            "node-a",
            ip(10, 96, 0, 10),
            53,
            L4Proto::UDP,
            Some(("default", "other-svc")),
        );
        assert!(r.is_none());
    }

    #[test]
    fn lrp_service_matcher_without_service_lookup_passthrough() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/redirectpolicy/manager.go",
            "Resolve.NoServiceLookup",
            "tenant-lrp-svcnone"
        );
        let mut m = LrpManager::new();
        m.upsert_policy(make_node_local_dns_lrp(tenant)).unwrap();
        m.upsert_backend(dns_backend("node-a"));
        let r = m.resolve("node-a", ip(10, 96, 0, 10), 53, L4Proto::UDP, None);
        assert!(r.is_none());
    }

    // ── Backend locality ─────────────────────────────────────────────────────

    #[test]
    fn lrp_backend_on_different_node_skipped() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/redirectpolicy/manager.go",
            "Resolve.LocalOnly",
            "tenant-lrp-loc"
        );
        let mut m = LrpManager::new();
        m.upsert_policy(make_node_local_dns_lrp(tenant)).unwrap();
        m.upsert_backend(dns_backend("node-b"));
        let r = m.resolve(
            "node-a",
            ip(10, 96, 0, 10),
            53,
            L4Proto::UDP,
            Some(("kube-system", "kube-dns")),
        );
        assert!(r.is_none());
    }

    #[test]
    fn lrp_backend_on_correct_node_selected() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/redirectpolicy/manager.go",
            "Resolve.LocalNodeMatch",
            "tenant-lrp-locm"
        );
        let mut m = LrpManager::new();
        m.upsert_policy(make_node_local_dns_lrp(tenant)).unwrap();
        m.upsert_backend(dns_backend("node-a"));
        m.upsert_backend(dns_backend("node-b"));
        let r = m
            .resolve(
                "node-a",
                ip(10, 96, 0, 10),
                53,
                L4Proto::UDP,
                Some(("kube-system", "kube-dns")),
            )
            .unwrap();
        assert!(r.backend_pod.contains("node-a"));
    }

    #[test]
    fn lrp_backend_label_mismatch_no_redirect() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/redirectpolicy/manager.go",
            "Resolve.LabelMismatch",
            "tenant-lrp-lbl"
        );
        let mut m = LrpManager::new();
        m.upsert_policy(make_node_local_dns_lrp(tenant)).unwrap();
        let mut bad = dns_backend("node-a");
        bad.labels = vec![("k8s-app".into(), "other".into())];
        m.upsert_backend(bad);
        let r = m.resolve(
            "node-a",
            ip(10, 96, 0, 10),
            53,
            L4Proto::UDP,
            Some(("kube-system", "kube-dns")),
        );
        assert!(r.is_none());
    }

    #[test]
    fn lrp_backend_namespace_mismatch_no_redirect() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/redirectpolicy/manager.go",
            "Resolve.NamespaceMismatch",
            "tenant-lrp-ns"
        );
        let mut m = LrpManager::new();
        m.upsert_policy(make_node_local_dns_lrp(tenant)).unwrap();
        let mut bad = dns_backend("node-a");
        bad.pod_namespace = "default".into();
        m.upsert_backend(bad);
        let r = m.resolve(
            "node-a",
            ip(10, 96, 0, 10),
            53,
            L4Proto::UDP,
            Some(("kube-system", "kube-dns")),
        );
        assert!(r.is_none());
    }

    // ── Lifecycle ────────────────────────────────────────────────────────────

    #[test]
    fn lrp_remove_policy_drops_redirect() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/redirectpolicy/manager.go",
            "RemovePolicy",
            "tenant-lrp-rmp"
        );
        let mut m = LrpManager::new();
        m.upsert_policy(make_node_local_dns_lrp(tenant)).unwrap();
        m.upsert_backend(dns_backend("node-a"));
        m.remove_policy("kube-system/nodelocaldns").unwrap();
        let r = m.resolve(
            "node-a",
            ip(10, 96, 0, 10),
            53,
            L4Proto::UDP,
            Some(("kube-system", "kube-dns")),
        );
        assert!(r.is_none());
    }

    #[test]
    fn lrp_remove_backend_drops_redirect() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/redirectpolicy/manager.go",
            "RemoveBackend",
            "tenant-lrp-rmb"
        );
        let mut m = LrpManager::new();
        m.upsert_policy(make_node_local_dns_lrp(tenant)).unwrap();
        m.upsert_backend(dns_backend("node-a"));
        m.remove_backend("kube-system", "nldns-node-a");
        let r = m.resolve(
            "node-a",
            ip(10, 96, 0, 10),
            53,
            L4Proto::UDP,
            Some(("kube-system", "kube-dns")),
        );
        assert!(r.is_none());
    }

    #[test]
    fn lrp_remove_unknown_backend_returns_false() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/redirectpolicy/manager.go",
            "RemoveBackend.NotFound",
            "tenant-lrp-rmbnf"
        );
        let mut m = LrpManager::new();
        assert!(!m.remove_backend("kube-system", "nope"));
    }

    #[test]
    fn lrp_upsert_backend_replaces_in_place() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/redirectpolicy/manager.go",
            "UpsertBackend.Replace",
            "tenant-lrp-upb"
        );
        let mut m = LrpManager::new();
        m.upsert_policy(make_node_local_dns_lrp(tenant)).unwrap();
        m.upsert_backend(dns_backend("node-a"));
        let mut updated = dns_backend("node-a");
        updated.pod_ip = ip(10, 0, 1, 99);
        m.upsert_backend(updated);
        assert_eq!(m.backend_count(), 1);
        let r = m
            .resolve(
                "node-a",
                ip(10, 96, 0, 10),
                53,
                L4Proto::UDP,
                Some(("kube-system", "kube-dns")),
            )
            .unwrap();
        assert_eq!(r.backend_ip, ip(10, 0, 1, 99));
    }

    // ── Port renaming ────────────────────────────────────────────────────────

    #[test]
    fn lrp_port_renaming_uses_target_port() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/redirectpolicy/manager.go",
            "PortRename",
            "tenant-lrp-rename"
        );
        let mut m = LrpManager::new();
        let p = LocalRedirectPolicy {
            name: "rename".into(),
            namespace: "ns".into(),
            tenant,
            frontend: LrpFrontend::Address {
                ip: ip(10, 96, 0, 1),
                ports: vec![PortMatcher {
                    port: 80,
                    protocol: L4Proto::TCP,
                    target_port: Some(8080),
                }],
            },
            backend: LocalBackendSelector {
                match_labels: vec![("app".into(), "api".into())],
                namespace: "ns".into(),
                ports: vec![],
            },
            skip_redirect_no_match: false,
        };
        m.upsert_policy(p).unwrap();
        m.upsert_backend(LocalBackend {
            pod_name: "api".into(),
            pod_namespace: "ns".into(),
            node_name: "node-a".into(),
            pod_ip: ip(10, 0, 1, 5),
            labels: vec![("app".into(), "api".into())],
        });
        let r = m
            .resolve("node-a", ip(10, 96, 0, 1), 80, L4Proto::TCP, None)
            .unwrap();
        assert_eq!(r.target_port, 8080);
    }

    #[test]
    fn lrp_port_no_target_keeps_original() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/redirectpolicy/manager.go",
            "PortNoRename",
            "tenant-lrp-norename"
        );
        let mut m = LrpManager::new();
        let p = LocalRedirectPolicy {
            name: "rename".into(),
            namespace: "ns".into(),
            tenant,
            frontend: LrpFrontend::Address {
                ip: ip(10, 96, 0, 1),
                ports: vec![PortMatcher {
                    port: 80,
                    protocol: L4Proto::TCP,
                    target_port: None,
                }],
            },
            backend: LocalBackendSelector {
                match_labels: vec![("app".into(), "api".into())],
                namespace: "ns".into(),
                ports: vec![],
            },
            skip_redirect_no_match: false,
        };
        m.upsert_policy(p).unwrap();
        m.upsert_backend(LocalBackend {
            pod_name: "api".into(),
            pod_namespace: "ns".into(),
            node_name: "node-a".into(),
            pod_ip: ip(10, 0, 1, 5),
            labels: vec![("app".into(), "api".into())],
        });
        let r = m
            .resolve("node-a", ip(10, 96, 0, 1), 80, L4Proto::TCP, None)
            .unwrap();
        assert_eq!(r.target_port, 80);
    }

    // ── Node-local DNS canonical case ───────────────────────────────────────

    #[test]
    fn lrp_node_local_dns_redirects_udp_53() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/redirectpolicy/manager.go",
            "NodeLocalDNS.UDP",
            "tenant-lrp-nldns-udp"
        );
        let mut m = LrpManager::new();
        m.upsert_policy(make_node_local_dns_lrp(tenant)).unwrap();
        m.upsert_backend(dns_backend("node-a"));
        let r = m
            .resolve(
                "node-a",
                ip(10, 96, 0, 10),
                53,
                L4Proto::UDP,
                Some(("kube-system", "kube-dns")),
            )
            .unwrap();
        assert_eq!(r.target_port, 53);
    }

    #[test]
    fn lrp_node_local_dns_redirects_tcp_53() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/redirectpolicy/manager.go",
            "NodeLocalDNS.TCP",
            "tenant-lrp-nldns-tcp"
        );
        let mut m = LrpManager::new();
        m.upsert_policy(make_node_local_dns_lrp(tenant)).unwrap();
        m.upsert_backend(dns_backend("node-a"));
        let r = m
            .resolve(
                "node-a",
                ip(10, 96, 0, 10),
                53,
                L4Proto::TCP,
                Some(("kube-system", "kube-dns")),
            )
            .unwrap();
        assert_eq!(r.target_port, 53);
    }

    #[test]
    fn lrp_node_local_dns_passthrough_other_ports() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/redirectpolicy/manager.go",
            "NodeLocalDNS.OtherPort",
            "tenant-lrp-nldns-op"
        );
        let mut m = LrpManager::new();
        m.upsert_policy(make_node_local_dns_lrp(tenant)).unwrap();
        m.upsert_backend(dns_backend("node-a"));
        let r = m.resolve(
            "node-a",
            ip(10, 96, 0, 10),
            9153,
            L4Proto::TCP,
            Some(("kube-system", "kube-dns")),
        );
        assert!(r.is_none());
    }

    // ── Multi-policy ─────────────────────────────────────────────────────────

    #[test]
    fn lrp_first_matching_policy_wins() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/redirectpolicy/manager.go",
            "Resolve.MultiPolicy",
            "tenant-lrp-mp"
        );
        let mut m = LrpManager::new();
        let p1 = LocalRedirectPolicy {
            name: "p1".into(),
            namespace: "ns".into(),
            tenant: tenant.clone(),
            frontend: LrpFrontend::Address {
                ip: ip(10, 96, 0, 1),
                ports: vec![PortMatcher {
                    port: 80,
                    protocol: L4Proto::TCP,
                    target_port: None,
                }],
            },
            backend: LocalBackendSelector {
                match_labels: vec![("app".into(), "x".into())],
                namespace: "ns".into(),
                ports: vec![],
            },
            skip_redirect_no_match: false,
        };
        let mut p2 = p1.clone();
        p2.name = "p2".into();
        m.upsert_policy(p1).unwrap();
        m.upsert_policy(p2).unwrap();
        m.upsert_backend(LocalBackend {
            pod_name: "x".into(),
            pod_namespace: "ns".into(),
            node_name: "node-a".into(),
            pod_ip: ip(10, 0, 1, 5),
            labels: vec![("app".into(), "x".into())],
        });
        let r = m.resolve("node-a", ip(10, 96, 0, 1), 80, L4Proto::TCP, None);
        assert!(r.is_some());
    }

    #[test]
    fn lrp_count_tracks_upserts() {
        let (_c, tenant) =
            cilium_test_ctx!("pkg/redirectpolicy/manager.go", "Count", "tenant-lrp-cnt");
        let mut m = LrpManager::new();
        for i in 0..3u8 {
            let p = LocalRedirectPolicy {
                name: format!("p-{i}"),
                namespace: "ns".into(),
                tenant: tenant.clone(),
                frontend: LrpFrontend::Address {
                    ip: ip(10, 96, 0, i),
                    ports: vec![PortMatcher {
                        port: 80,
                        protocol: L4Proto::TCP,
                        target_port: None,
                    }],
                },
                backend: LocalBackendSelector {
                    match_labels: vec![("app".into(), "x".into())],
                    namespace: "ns".into(),
                    ports: vec![],
                },
                skip_redirect_no_match: false,
            };
            m.upsert_policy(p).unwrap();
        }
        assert_eq!(m.policy_count(), 3);
    }

    #[test]
    fn lrp_backend_count_tracks_upserts() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/redirectpolicy/manager.go",
            "BackendCount",
            "tenant-lrp-bcnt"
        );
        let mut m = LrpManager::new();
        for n in ["node-a", "node-b", "node-c"] {
            m.upsert_backend(dns_backend(n));
        }
        assert_eq!(m.backend_count(), 3);
    }

    // ── Serde ────────────────────────────────────────────────────────────────

    #[test]
    fn lrp_policy_serde_round_trip() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/k8s/apis/cilium.io/v2/ciliumlocalredirectpolicy_types.go",
            "LRP.Serde",
            "tenant-lrp-ps"
        );
        let p = make_node_local_dns_lrp(tenant);
        let json = serde_json::to_string(&p).unwrap();
        let back: LocalRedirectPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn lrp_decision_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/redirectpolicy/manager.go",
            "Decision.Serde",
            "tenant-lrp-ds"
        );
        let d = RedirectDecision {
            policy_name: "p".into(),
            backend_ip: ip(10, 0, 1, 9),
            backend_pod: "kube-system/nldns".into(),
            target_port: 53,
        };
        let json = serde_json::to_string(&d).unwrap();
        let back: RedirectDecision = serde_json::from_str(&json).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn lrp_frontend_serde_address_variant() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/redirectpolicy/manager.go",
            "Frontend.Address.Serde",
            "tenant-lrp-fas"
        );
        let f = LrpFrontend::Address {
            ip: ip(10, 96, 0, 1),
            ports: vec![PortMatcher {
                port: 80,
                protocol: L4Proto::TCP,
                target_port: Some(8080),
            }],
        };
        let json = serde_json::to_string(&f).unwrap();
        let back: LrpFrontend = serde_json::from_str(&json).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn lrp_frontend_serde_service_variant() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/redirectpolicy/manager.go",
            "Frontend.Service.Serde",
            "tenant-lrp-fss"
        );
        let f = LrpFrontend::Service {
            namespace: "kube-system".into(),
            name: "kube-dns".into(),
            ports: vec![PortMatcher {
                port: 53,
                protocol: L4Proto::UDP,
                target_port: None,
            }],
        };
        let json = serde_json::to_string(&f).unwrap();
        let back: LrpFrontend = serde_json::from_str(&json).unwrap();
        assert_eq!(back, f);
    }
}
