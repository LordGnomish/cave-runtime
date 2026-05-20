// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Legacy `v1.Endpoints` reconciler.
//!
//! Cite: `pkg/controller/endpoint/endpoints_controller.go` (v1.36.0).
//!
//! cave normally writes only `discovery.k8s.io/v1.EndpointSlice` — the
//! modern shape that scales past 1 000 endpoints per service. v1.36 of
//! kubernetes still ships the legacy `v1.Endpoints` controller for the
//! benefit of older clients that haven't learned EndpointSlice yet
//! (kube-proxy in `iptables`-mode, third-party operators, …). KEP-572
//! documents the contract: the legacy object must continue to be
//! reconciled even though the new object is the source of truth.
//!
//! This module computes a `v1.Endpoints` object purely from the input
//! Service + Pod state. The function is idempotent — given the same
//! input it emits the same `Endpoints` payload, sorted deterministically
//! so cave-portal can diff two snapshots byte-for-byte.
//!
//! ```text
//!     Service.spec.selector + Service.spec.ports
//!                       │
//!     Pod.status.podIP  ▼
//!     Pod.status.phase  ▼  ───────►  v1.Endpoints { subsets[…] }
//!     Pod.spec.containers[].ports ▼
//! ```

use crate::types::Cite;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/endpoint/endpoints_controller.go",
    "EndpointsController.syncService",
);

/// Minimal projection of a `v1.Service` we need to reconcile endpoints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceSpec {
    pub namespace: String,
    pub name: String,
    /// Pod-label selector. Pods must match all labels to be eligible.
    pub selector: BTreeMap<String, String>,
    /// Port mapping. `name` is required when len > 1 (per v1.36
    /// validation); `target` is the per-pod port to forward to.
    pub ports: Vec<ServicePort>,
    /// `None` for headless services; `Some(_)` otherwise. We don't use
    /// it in v1.Endpoints reconciliation but track it for parity with
    /// the upstream `syncService` signature.
    pub cluster_ip: Option<String>,
    /// Headless services with `publishNotReadyAddresses=true` write
    /// not-ready endpoints into the *primary* `addresses[]` block
    /// instead of `notReadyAddresses[]` (this is the upstream "subset
    /// publish-not-ready" toggle).
    pub publish_not_ready_addresses: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServicePort {
    pub name: String,
    /// L4 protocol — "TCP" / "UDP" / "SCTP".
    pub protocol: String,
    /// Target port on each pod. v1 only supports numeric ports here
    /// (we resolve named ports against the pod spec ourselves).
    pub target: TargetPort,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetPort {
    Number(u16),
    /// Named port; resolved against the pod's containerPorts at sync
    /// time. Pods that don't expose a matching name are skipped.
    Name(String),
}

/// Minimal projection of a `v1.Pod` for endpoint computation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodView {
    pub namespace: String,
    pub name: String,
    pub labels: BTreeMap<String, String>,
    /// Pod IP — empty string ⇒ not yet assigned, pod is filtered out.
    pub pod_ip: String,
    pub node_name: Option<String>,
    pub uid: String,
    /// True when all containers report Ready (k8s readiness gate).
    pub ready: bool,
    pub terminating: bool,
    /// `(container_port_name → number)` lookup — used to resolve named
    /// `TargetPort::Name` references.
    pub named_ports: BTreeMap<String, u16>,
}

/// One `v1.Endpoints` subset block — a tuple of addresses + ports
/// that travel together. Pods that expose the same set of (resolved)
/// service-ports share a subset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct EndpointSubset {
    pub addresses: Vec<EndpointAddress>,
    pub not_ready_addresses: Vec<EndpointAddress>,
    pub ports: Vec<EndpointPort>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EndpointAddress {
    pub ip: String,
    pub node_name: Option<String>,
    /// `(ns, name, uid)` triple of the source pod.
    pub target_ref: PodRef,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PodRef {
    pub namespace: String,
    pub name: String,
    pub uid: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EndpointPort {
    pub name: String,
    pub port: u16,
    pub protocol: String,
}

/// Computed `v1.Endpoints` object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Endpoints {
    pub namespace: String,
    pub name: String,
    pub subsets: Vec<EndpointSubset>,
}

/// Compute the `v1.Endpoints` object for `service` given the current
/// set of pods. Idempotent + deterministic — addresses inside each
/// subset are sorted by IP, subsets are sorted by their port-set
/// signature, and ports inside each subset are sorted by (name, port).
pub fn reconcile(service: &ServiceSpec, pods: &[PodView]) -> Endpoints {
    // Step 1: pre-filter pods — must match selector + same namespace +
    // have a pod IP and not be terminating.
    let eligible: Vec<&PodView> = pods
        .iter()
        .filter(|p| {
            p.namespace == service.namespace
                && !p.pod_ip.is_empty()
                && !p.terminating
                && matches_selector(&p.labels, &service.selector)
        })
        .collect();

    // Step 2: resolve service ports against each pod. Pods whose named
    // ports don't resolve are skipped *for that service-port only*.
    // Resulting subsets are keyed by the sorted (resolved-port-set).
    let mut buckets: BTreeMap<Vec<EndpointPort>, EndpointSubset> = BTreeMap::new();
    for pod in &eligible {
        let mut resolved: Vec<EndpointPort> = Vec::new();
        for sp in &service.ports {
            let n = match &sp.target {
                TargetPort::Number(n) => Some(*n),
                TargetPort::Name(nm) => pod.named_ports.get(nm).copied(),
            };
            if let Some(n) = n {
                resolved.push(EndpointPort {
                    name: sp.name.clone(),
                    port: n,
                    protocol: sp.protocol.clone(),
                });
            }
        }
        if resolved.is_empty() {
            // Pod resolves no service-ports at all — skip outright.
            continue;
        }
        resolved.sort();
        let subset = buckets.entry(resolved.clone()).or_insert_with(|| EndpointSubset {
            addresses: Vec::new(),
            not_ready_addresses: Vec::new(),
            ports: resolved.clone(),
        });
        let addr = EndpointAddress {
            ip: pod.pod_ip.clone(),
            node_name: pod.node_name.clone(),
            target_ref: PodRef {
                namespace: pod.namespace.clone(),
                name: pod.name.clone(),
                uid: pod.uid.clone(),
            },
        };
        if pod.ready || service.publish_not_ready_addresses {
            subset.addresses.push(addr);
        } else {
            subset.not_ready_addresses.push(addr);
        }
    }

    // Step 3: deterministic sort + emit.
    let mut subsets: Vec<EndpointSubset> = buckets.into_values().collect();
    for s in subsets.iter_mut() {
        s.addresses.sort();
        s.not_ready_addresses.sort();
    }
    subsets.sort_by(|a, b| a.ports.cmp(&b.ports));

    Endpoints {
        namespace: service.namespace.clone(),
        name: service.name.clone(),
        subsets,
    }
}

fn matches_selector(labels: &BTreeMap<String, String>, sel: &BTreeMap<String, String>) -> bool {
    if sel.is_empty() {
        // upstream: empty selector ⇒ never match (vs. EndpointSlice
        // which has the same rule via syncService early-return).
        return false;
    }
    sel.iter().all(|(k, v)| labels.get(k).is_some_and(|lv| lv == v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn svc() -> ServiceSpec {
        let mut sel = BTreeMap::new();
        sel.insert("app".to_string(), "web".to_string());
        ServiceSpec {
            namespace: "default".to_string(),
            name: "web".to_string(),
            selector: sel,
            ports: vec![ServicePort {
                name: "http".to_string(),
                protocol: "TCP".to_string(),
                target: TargetPort::Number(8080),
            }],
            cluster_ip: Some("10.0.0.1".to_string()),
            publish_not_ready_addresses: false,
        }
    }

    fn pod(name: &str, ip: &str, ready: bool) -> PodView {
        let mut labels = BTreeMap::new();
        labels.insert("app".to_string(), "web".to_string());
        PodView {
            namespace: "default".to_string(),
            name: name.to_string(),
            labels,
            pod_ip: ip.to_string(),
            node_name: Some("node-1".to_string()),
            uid: format!("uid-{name}"),
            ready,
            terminating: false,
            named_ports: BTreeMap::new(),
        }
    }

    #[test]
    fn selector_matching_filters_non_matching_pods() {
        let (_c, _t) = test_ctx!(
            "pkg/controller/endpoint/endpoints_controller.go",
            "syncService:selector",
            "tenant-ep-selector"
        );
        let s = svc();
        let mut other = pod("other", "10.1.0.5", true);
        other.labels.insert("app".to_string(), "db".to_string());
        let pods = vec![pod("a", "10.1.0.1", true), other];
        let ep = reconcile(&s, &pods);
        // Only "a" survives the selector.
        let addrs: Vec<&str> = ep.subsets[0]
            .addresses
            .iter()
            .map(|a| a.ip.as_str())
            .collect();
        assert_eq!(addrs, vec!["10.1.0.1"]);
    }

    #[test]
    fn ready_pods_go_to_addresses_unready_to_not_ready() {
        let s = svc();
        let pods = vec![pod("a", "10.1.0.1", true), pod("b", "10.1.0.2", false)];
        let ep = reconcile(&s, &pods);
        assert_eq!(ep.subsets.len(), 1);
        let sub = &ep.subsets[0];
        assert_eq!(sub.addresses.len(), 1);
        assert_eq!(sub.addresses[0].ip, "10.1.0.1");
        assert_eq!(sub.not_ready_addresses.len(), 1);
        assert_eq!(sub.not_ready_addresses[0].ip, "10.1.0.2");
    }

    #[test]
    fn publish_not_ready_addresses_promotes_unready_into_primary_block() {
        let mut s = svc();
        s.publish_not_ready_addresses = true;
        let pods = vec![pod("a", "10.1.0.1", false), pod("b", "10.1.0.2", false)];
        let ep = reconcile(&s, &pods);
        assert_eq!(ep.subsets.len(), 1);
        let sub = &ep.subsets[0];
        assert_eq!(sub.addresses.len(), 2);
        assert!(sub.not_ready_addresses.is_empty());
    }

    #[test]
    fn terminating_pods_are_excluded() {
        let s = svc();
        let mut p = pod("a", "10.1.0.1", true);
        p.terminating = true;
        let ep = reconcile(&s, &vec![p, pod("b", "10.1.0.2", true)]);
        let ips: Vec<&str> = ep.subsets[0]
            .addresses
            .iter()
            .map(|a| a.ip.as_str())
            .collect();
        assert_eq!(ips, vec!["10.1.0.2"]);
    }

    #[test]
    fn pods_without_ip_are_excluded() {
        let s = svc();
        let pods = vec![pod("a", "", true), pod("b", "10.1.0.2", true)];
        let ep = reconcile(&s, &pods);
        assert_eq!(ep.subsets.len(), 1);
        assert_eq!(ep.subsets[0].addresses.len(), 1);
        assert_eq!(ep.subsets[0].addresses[0].ip, "10.1.0.2");
    }

    #[test]
    fn output_is_idempotent_under_repeated_calls() {
        let s = svc();
        let pods = vec![pod("a", "10.1.0.1", true), pod("b", "10.1.0.2", true)];
        let first = reconcile(&s, &pods);
        let second = reconcile(&s, &pods);
        assert_eq!(first, second);
    }

    #[test]
    fn addresses_within_subset_are_sorted_deterministically() {
        let s = svc();
        // Insert intentionally out-of-order.
        let pods = vec![
            pod("c", "10.1.0.3", true),
            pod("a", "10.1.0.1", true),
            pod("b", "10.1.0.2", true),
        ];
        let ep = reconcile(&s, &pods);
        let ips: Vec<&str> = ep.subsets[0]
            .addresses
            .iter()
            .map(|a| a.ip.as_str())
            .collect();
        assert_eq!(ips, vec!["10.1.0.1", "10.1.0.2", "10.1.0.3"]);
    }

    #[test]
    fn named_target_port_resolves_via_pod_container_ports() {
        let mut s = svc();
        s.ports[0].target = TargetPort::Name("http".to_string());
        let mut p1 = pod("a", "10.1.0.1", true);
        p1.named_ports.insert("http".to_string(), 8080);
        // p2 doesn't expose the named port — it falls out of the
        // subset entirely.
        let p2 = pod("b", "10.1.0.2", true);
        let ep = reconcile(&s, &vec![p1, p2]);
        assert_eq!(ep.subsets.len(), 1);
        assert_eq!(ep.subsets[0].addresses.len(), 1);
        assert_eq!(ep.subsets[0].addresses[0].ip, "10.1.0.1");
        assert_eq!(ep.subsets[0].ports[0].port, 8080);
    }

    #[test]
    fn empty_selector_emits_no_endpoints() {
        let mut s = svc();
        s.selector.clear();
        let pods = vec![pod("a", "10.1.0.1", true)];
        let ep = reconcile(&s, &pods);
        assert!(ep.subsets.is_empty());
    }

    #[test]
    fn cross_namespace_pods_are_ignored() {
        let s = svc();
        let mut p = pod("a", "10.1.0.1", true);
        p.namespace = "other-ns".to_string();
        let ep = reconcile(&s, &vec![p, pod("b", "10.1.0.2", true)]);
        assert_eq!(ep.subsets[0].addresses.len(), 1);
        assert_eq!(ep.subsets[0].addresses[0].ip, "10.1.0.2");
    }

    #[test]
    fn endpoints_object_serialises_via_serde_json() {
        let s = svc();
        let ep = reconcile(&s, &vec![pod("a", "10.1.0.1", true)]);
        let v = serde_json::to_value(&ep).unwrap();
        assert_eq!(v["name"], "web");
        assert_eq!(v["namespace"], "default");
        assert!(v["subsets"].is_array());
    }
}
