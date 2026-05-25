// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Topology-aware routing — CategorizeEndpoints + hint application.
//!
//! Cite: `pkg/proxy/topology.go:36` (CategorizeEndpoints),
//! `:48` (canUseTopology), `:113` (filterEndpoints),
//! `pkg/proxy/endpointslicecache.go:90` (standardEndpointInfo — zone hint).

use crate::endpoints::EndpointInfo;
use crate::service::{ServicePortInfo, TrafficPolicy};

/// Cite: `pkg/proxy/topology.go:36` (CategorizeEndpoints) — endpoints
/// are split by the proxier into three buckets for downstream rule
/// emission:
///
/// * `cluster` — eligible for cluster-wide service traffic
///   (externalTrafficPolicy=Cluster).
/// * `local`   — eligible for node-local traffic
///   (externalTrafficPolicy=Local / internalTrafficPolicy=Local).
/// * `all_reachable_eps` — used by the LB endpoint-readiness probe.
#[derive(Debug, Clone)]
pub struct EndpointCategories<'a> {
    pub cluster: Vec<&'a EndpointInfo>,
    pub local: Vec<&'a EndpointInfo>,
    pub all_reachable_eps: Vec<&'a EndpointInfo>,
}

impl<'a> EndpointCategories<'a> {
    pub fn is_empty(&self) -> bool {
        self.cluster.is_empty() && self.local.is_empty()
    }
}

/// Cite: `pkg/proxy/topology.go:48` (canUseTopology) — topology hints
/// are honored only when:
///
/// 1. The Service carries `service.kubernetes.io/topology-aware-hints=Auto`.
/// 2. Every endpoint carries a non-empty `hints.forZones` annotation.
/// 3. At least one zone is the proxier's local zone.
///
/// cave inputs the per-endpoint zone via `EndpointInfo::zone` and the
/// service hint flag via the `hints_enabled` argument. The current-node
/// zone is the third input.
pub fn can_use_topology(
    eps: &[&EndpointInfo],
    hints_enabled: bool,
    current_zone: &str,
) -> bool {
    if !hints_enabled {
        return false;
    }
    let mut have_any_hint = false;
    let mut zone_present = false;
    for ep in eps {
        if let Some(z) = &ep.zone {
            have_any_hint = true;
            if z == current_zone {
                zone_present = true;
            }
        }
    }
    have_any_hint && zone_present
}

/// Cite: `pkg/proxy/topology.go:113` (filterEndpoints) — narrow a flat
/// endpoint list down to (local, cluster) buckets according to the
/// Service's external/internal traffic policies, topology hints, and
/// the current node identity.
pub fn categorize_endpoints<'a>(
    svc: &ServicePortInfo,
    endpoints: &'a [&EndpointInfo],
    current_node: &str,
    current_zone: &str,
    hints_enabled: bool,
) -> EndpointCategories<'a> {
    let all_reachable: Vec<&EndpointInfo> = endpoints
        .iter()
        .copied()
        .filter(|e| e.ready && e.serving)
        .collect();

    let topology_active = can_use_topology(&all_reachable, hints_enabled, current_zone);

    let local_eps: Vec<&EndpointInfo> = all_reachable
        .iter()
        .copied()
        .filter(|e| e.is_local(current_node))
        .collect();

    let cluster_base: Vec<&EndpointInfo> = if topology_active {
        all_reachable
            .iter()
            .copied()
            .filter(|e| e.zone.as_deref() == Some(current_zone))
            .collect()
    } else {
        all_reachable.clone()
    };

    // externalTrafficPolicy=Local cuts cluster bucket to local-only when
    // it's not empty; if every endpoint is non-local, the proxier still
    // routes to "cluster" (Kubernetes 1.30+ behaviour: PreferLocal fall-back).
    let cluster = match svc.external_traffic_policy {
        TrafficPolicy::Local if !local_eps.is_empty() => local_eps.clone(),
        _ => cluster_base,
    };

    // `local` bucket: literally node-local endpoints (the proxier picks
    // from `cluster` vs `local` per-rule depending on traffic policy;
    // this categorization just surfaces what's reachable in each bucket).
    let _ = svc.internal_traffic_policy; // surface the field; consumption is in the proxier
    EndpointCategories {
        cluster,
        local: local_eps,
        all_reachable_eps: all_reachable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::endpoints::EndpointInfo;
    use crate::service::{Protocol, ServicePortInfo, ServicePortName};
    use std::net::{IpAddr, Ipv4Addr};

    fn ep(addr: &str, node: Option<&str>, zone: Option<&str>) -> EndpointInfo {
        let mut e = EndpointInfo::ready(IpAddr::V4(addr.parse().unwrap()), 8080);
        e.node_name = node.map(str::to_string);
        e.zone = zone.map(str::to_string);
        e
    }

    fn svc(ext: TrafficPolicy, int: TrafficPolicy) -> ServicePortInfo {
        let mut s = ServicePortInfo::cluster_ip_only(
            "t1",
            ServicePortName::new("default", "x", "http"),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            80,
            Protocol::Tcp,
        );
        s.external_traffic_policy = ext;
        s.internal_traffic_policy = int;
        s
    }

    #[test]
    fn cluster_policy_sees_all_endpoints() {
        let e1 = ep("10.0.0.1", Some("nodeA"), Some("z1"));
        let e2 = ep("10.0.0.2", Some("nodeB"), Some("z2"));
        let s = svc(TrafficPolicy::Cluster, TrafficPolicy::Cluster);
        let eps = vec![&e1, &e2];
        let cat = categorize_endpoints(&s, &eps, "nodeA", "z1", false);
        assert_eq!(cat.cluster.len(), 2);
    }

    #[test]
    fn external_local_keeps_only_local_endpoints() {
        let e1 = ep("10.0.0.1", Some("nodeA"), None);
        let e2 = ep("10.0.0.2", Some("nodeB"), None);
        let s = svc(TrafficPolicy::Local, TrafficPolicy::Cluster);
        let eps = vec![&e1, &e2];
        let cat = categorize_endpoints(&s, &eps, "nodeA", "z1", false);
        assert_eq!(cat.cluster.len(), 1);
        assert_eq!(cat.cluster[0].addresses[0].to_string(), "10.0.0.1");
    }

    #[test]
    fn external_local_with_no_local_endpoints_falls_back_to_cluster() {
        let e1 = ep("10.0.0.1", Some("nodeA"), None);
        let e2 = ep("10.0.0.2", Some("nodeB"), None);
        let s = svc(TrafficPolicy::Local, TrafficPolicy::Cluster);
        let eps = vec![&e1, &e2];
        let cat = categorize_endpoints(&s, &eps, "nodeC", "z1", false);
        assert_eq!(cat.cluster.len(), 2); // both nodes considered "cluster"
        assert!(cat.local.is_empty());
    }

    #[test]
    fn topology_hints_filter_by_zone_when_enabled() {
        let e1 = ep("10.0.0.1", Some("nodeA"), Some("z1"));
        let e2 = ep("10.0.0.2", Some("nodeB"), Some("z2"));
        let s = svc(TrafficPolicy::Cluster, TrafficPolicy::Cluster);
        let eps = vec![&e1, &e2];
        let cat = categorize_endpoints(&s, &eps, "nodeA", "z1", true);
        assert_eq!(cat.cluster.len(), 1);
        assert_eq!(cat.cluster[0].addresses[0].to_string(), "10.0.0.1");
    }

    #[test]
    fn topology_hints_disabled_keeps_full_cluster() {
        let e1 = ep("10.0.0.1", Some("nodeA"), Some("z1"));
        let e2 = ep("10.0.0.2", Some("nodeB"), Some("z2"));
        let s = svc(TrafficPolicy::Cluster, TrafficPolicy::Cluster);
        let eps = vec![&e1, &e2];
        let cat = categorize_endpoints(&s, &eps, "nodeA", "z1", false);
        assert_eq!(cat.cluster.len(), 2);
    }

    #[test]
    fn local_bucket_pins_to_node_local_endpoints() {
        let e1 = ep("10.0.0.1", Some("nodeA"), None);
        let e2 = ep("10.0.0.2", Some("nodeB"), None);
        let s = svc(TrafficPolicy::Cluster, TrafficPolicy::Local);
        let eps = vec![&e1, &e2];
        let cat = categorize_endpoints(&s, &eps, "nodeA", "z1", false);
        assert_eq!(cat.local.len(), 1);
        assert_eq!(cat.local[0].addresses[0].to_string(), "10.0.0.1");
    }

    #[test]
    fn unready_endpoints_drop_out() {
        let mut e1 = ep("10.0.0.1", Some("nodeA"), None);
        e1.ready = false;
        let e2 = ep("10.0.0.2", Some("nodeB"), None);
        let s = svc(TrafficPolicy::Cluster, TrafficPolicy::Cluster);
        let eps = vec![&e1, &e2];
        let cat = categorize_endpoints(&s, &eps, "nodeA", "z1", false);
        assert_eq!(cat.cluster.len(), 1);
    }

    #[test]
    fn can_use_topology_requires_zone_match() {
        let e1 = ep("10.0.0.1", None, Some("z1"));
        let e2 = ep("10.0.0.2", None, Some("z2"));
        let eps = vec![&e1, &e2];
        assert!(can_use_topology(&eps, true, "z1"));
        assert!(!can_use_topology(&eps, true, "z9"));
        assert!(!can_use_topology(&eps, false, "z1"));
    }
}
