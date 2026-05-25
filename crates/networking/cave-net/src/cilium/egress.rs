// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Egress gateway — `CiliumEgressGatewayPolicy` evaluator.
//!
//! Mirrors `pkg/egressgateway/manager.go` (per-policy state machine) and
//! the CRD shape from `pkg/k8s/apis/cilium.io/v2/ciliumegressgatewaypolicy_types.go`.
//!
//! Semantics (faithful to upstream):
//!
//! * A policy `selects` source endpoints via a label `EndpointSelector`
//!   (matches `pkg/policy/api/selector.go::EndpointSelector`).
//! * A policy declares one or more `destination_cidrs` and zero or more
//!   `excluded_cidrs`. Egress packets whose destination falls into a
//!   `destination_cidr` *and not* an `excluded_cidr` are SNAT'd to the
//!   policy's `egress_ip` and forwarded via the chosen gateway node.
//! * Multiple gateway nodes form an HA pool; selection is hash-based
//!   for stickiness and failover skips unhealthy nodes.
//! * If multiple policies match a (source, destination) pair, the
//!   first one whose order of insertion is earlier wins (mirrors the
//!   priority table in `pkg/maps/egressmap`).

use crate::cilium::policy::{EndpointSelector, PolicyError};
use crate::cilium::types::{Cite, TenantId};
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GatewayState {
    Healthy,
    Unhealthy,
    Draining,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayNode {
    pub name: String,
    pub node_ip: IpAddr,
    pub state: GatewayState,
}

impl GatewayNode {
    pub fn new(name: impl Into<String>, node_ip: IpAddr) -> Self {
        Self {
            name: name.into(),
            node_ip,
            state: GatewayState::Healthy,
        }
    }
    pub fn eligible(&self) -> bool {
        matches!(self.state, GatewayState::Healthy)
    }
}

/// CiliumEgressGatewayPolicy CRD shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EgressGatewayPolicy {
    pub name: String,
    pub tenant: TenantId,
    pub source_selector: EndpointSelector,
    pub source_namespace_selector: Option<EndpointSelector>,
    pub destination_cidrs: Vec<String>,
    pub excluded_cidrs: Vec<String>,
    pub egress_ip: IpAddr,
    pub gateway_nodes: Vec<GatewayNode>,
}

impl EgressGatewayPolicy {
    pub fn new(
        name: impl Into<String>,
        tenant: TenantId,
        source_selector: EndpointSelector,
        egress_ip: IpAddr,
    ) -> Self {
        Self {
            name: name.into(),
            tenant,
            source_selector,
            source_namespace_selector: None,
            destination_cidrs: Vec::new(),
            excluded_cidrs: Vec::new(),
            egress_ip,
            gateway_nodes: Vec::new(),
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EgressError {
    #[error("invalid CIDR `{0}`")]
    BadCidr(String),
    #[error("policy `{0}` has no destination CIDRs")]
    NoDestinationCidrs(String),
    #[error("policy `{0}` has no gateway nodes")]
    NoGatewayNodes(String),
    #[error("tenant {tenant} cannot mutate egress policy owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

impl From<PolicyError> for EgressError {
    fn from(e: PolicyError) -> Self {
        match e {
            PolicyError::BadCidr(s) => EgressError::BadCidr(s),
            PolicyError::TenantDenied { tenant } => EgressError::TenantDenied { tenant },
            PolicyError::UnknownIdentity(_) => EgressError::BadCidr("unknown identity".into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EgressDecision {
    pub policy_name: String,
    pub egress_ip: IpAddr,
    pub gateway_node: String,
    pub gateway_node_ip: IpAddr,
}

#[derive(Debug, Default)]
pub struct EgressManager {
    policies: Vec<EgressGatewayPolicy>,
}

impl EgressManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert(&mut self, policy: EgressGatewayPolicy) -> Result<(), EgressError> {
        if policy.destination_cidrs.is_empty() {
            return Err(EgressError::NoDestinationCidrs(policy.name));
        }
        if policy.gateway_nodes.is_empty() {
            return Err(EgressError::NoGatewayNodes(policy.name));
        }
        for c in policy
            .destination_cidrs
            .iter()
            .chain(policy.excluded_cidrs.iter())
        {
            IpNet::from_str(c).map_err(|_| EgressError::BadCidr(c.clone()))?;
        }
        if let Some(idx) = self.policies.iter().position(|p| p.name == policy.name) {
            self.policies[idx] = policy;
        } else {
            self.policies.push(policy);
        }
        Ok(())
    }

    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.policies.len();
        self.policies.retain(|p| p.name != name);
        before != self.policies.len()
    }

    pub fn len(&self) -> usize {
        self.policies.len()
    }

    pub fn is_empty(&self) -> bool {
        self.policies.is_empty()
    }

    /// Apply egress policy to a packet from `src_labels` (the source pod's
    /// labels) heading to `dst_ip`. Returns the SNAT decision or `None` if
    /// no policy matches.
    pub fn evaluate(
        &self,
        src_labels: &crate::cilium::identity::LabelSet,
        src_namespace_labels: &crate::cilium::identity::LabelSet,
        dst_ip: IpAddr,
        flow_hash: u64,
    ) -> Result<Option<EgressDecision>, EgressError> {
        for p in &self.policies {
            if !p.source_selector.matches(src_labels) {
                continue;
            }
            if let Some(ns) = &p.source_namespace_selector {
                if !ns.matches(src_namespace_labels) {
                    continue;
                }
            }
            if cidr_matches(&p.excluded_cidrs, dst_ip)? {
                continue;
            }
            if !cidr_matches(&p.destination_cidrs, dst_ip)? {
                continue;
            }
            // Pick a healthy gateway via flow_hash for stickiness.
            let healthy: Vec<&GatewayNode> =
                p.gateway_nodes.iter().filter(|g| g.eligible()).collect();
            if healthy.is_empty() {
                return Ok(None);
            }
            let g = healthy[(flow_hash as usize) % healthy.len()];
            return Ok(Some(EgressDecision {
                policy_name: p.name.clone(),
                egress_ip: p.egress_ip,
                gateway_node: g.name.clone(),
                gateway_node_ip: g.node_ip,
            }));
        }
        Ok(None)
    }
}

fn cidr_matches(cidrs: &[String], ip: IpAddr) -> Result<bool, EgressError> {
    for c in cidrs {
        let net = IpNet::from_str(c).map_err(|_| EgressError::BadCidr(c.clone()))?;
        if net.contains(&ip) {
            return Ok(true);
        }
    }
    Ok(false)
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/egressgateway/manager.go", "Manager");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium::identity::LabelSet;
    use crate::cilium::policy::{EndpointSelector, MatchExpression, SelectorOp};
    use crate::cilium_test_ctx;
    use std::collections::HashMap;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    fn ls(pairs: &[(&str, &str)]) -> LabelSet {
        LabelSet::from_iter(pairs.iter().map(|(k, v)| (*k, *v)))
    }

    fn endpoint_sel(pairs: &[(&str, &str)]) -> EndpointSelector {
        EndpointSelector {
            match_labels: pairs
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect(),
            match_expressions: Vec::new(),
        }
    }

    fn make_policy(
        name: &str,
        tenant: TenantId,
        sel: EndpointSelector,
        dest: &str,
        egress: IpAddr,
        gw: IpAddr,
    ) -> EgressGatewayPolicy {
        EgressGatewayPolicy {
            name: name.into(),
            tenant,
            source_selector: sel,
            source_namespace_selector: None,
            destination_cidrs: vec![dest.into()],
            excluded_cidrs: vec![],
            egress_ip: egress,
            gateway_nodes: vec![GatewayNode::new("gw1", gw)],
        }
    }

    // ── Validation ───────────────────────────────────────────────────────────

    #[test]
    fn egw_upsert_with_no_destination_cidrs_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/egressgateway/manager.go",
            "Manager.Upsert.Validate",
            "tenant-egw-no-dest"
        );
        let mut mgr = EgressManager::new();
        let mut p = make_policy(
            "p",
            tenant,
            EndpointSelector::empty(),
            "0.0.0.0/0",
            ip(192, 0, 2, 1),
            ip(10, 0, 0, 1),
        );
        p.destination_cidrs.clear();
        let err = mgr.upsert(p).unwrap_err();
        assert_eq!(err, EgressError::NoDestinationCidrs("p".into()));
    }

    #[test]
    fn egw_upsert_with_no_gateway_nodes_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/egressgateway/manager.go",
            "Manager.Upsert.Validate",
            "tenant-egw-no-gw"
        );
        let mut mgr = EgressManager::new();
        let mut p = make_policy(
            "p",
            tenant,
            EndpointSelector::empty(),
            "0.0.0.0/0",
            ip(192, 0, 2, 1),
            ip(10, 0, 0, 1),
        );
        p.gateway_nodes.clear();
        let err = mgr.upsert(p).unwrap_err();
        assert_eq!(err, EgressError::NoGatewayNodes("p".into()));
    }

    #[test]
    fn egw_upsert_with_bad_cidr_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/egressgateway/manager.go",
            "Manager.Upsert.Validate",
            "tenant-egw-bad-cidr"
        );
        let mut mgr = EgressManager::new();
        let mut p = make_policy(
            "p",
            tenant,
            EndpointSelector::empty(),
            "not-a-cidr",
            ip(192, 0, 2, 1),
            ip(10, 0, 0, 1),
        );
        p.destination_cidrs = vec!["nope".into()];
        let err = mgr.upsert(p).unwrap_err();
        assert!(matches!(err, EgressError::BadCidr(_)));
    }

    // ── Selection ────────────────────────────────────────────────────────────

    #[test]
    fn egw_policy_matches_source_pod_by_labels() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/egressgateway/manager.go",
            "Manager.Lookup",
            "tenant-egw-match"
        );
        let mut mgr = EgressManager::new();
        mgr.upsert(make_policy(
            "tenant-A",
            tenant,
            endpoint_sel(&[("app", "billing")]),
            "1.0.0.0/8",
            ip(192, 0, 2, 100),
            ip(10, 0, 0, 1),
        ))
        .unwrap();
        let dec = mgr
            .evaluate(&ls(&[("app", "billing")]), &ls(&[]), ip(1, 1, 1, 1), 0)
            .unwrap()
            .unwrap();
        assert_eq!(dec.policy_name, "tenant-A");
        assert_eq!(dec.egress_ip, ip(192, 0, 2, 100));
    }

    #[test]
    fn egw_policy_does_not_match_unrelated_pod() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/egressgateway/manager.go",
            "Manager.Lookup.NoMatch",
            "tenant-egw-nomatch"
        );
        let mut mgr = EgressManager::new();
        mgr.upsert(make_policy(
            "tenant-A",
            tenant,
            endpoint_sel(&[("app", "billing")]),
            "1.0.0.0/8",
            ip(192, 0, 2, 100),
            ip(10, 0, 0, 1),
        ))
        .unwrap();
        let dec = mgr
            .evaluate(&ls(&[("app", "metrics")]), &ls(&[]), ip(1, 1, 1, 1), 0)
            .unwrap();
        assert!(dec.is_none());
    }

    #[test]
    fn egw_excluded_cidr_overrides_destination_cidr() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/egressgateway/manager.go",
            "Manager.Lookup.Excluded",
            "tenant-egw-excl"
        );
        let mut mgr = EgressManager::new();
        let mut p = make_policy(
            "egw",
            tenant,
            endpoint_sel(&[("app", "x")]),
            "1.0.0.0/8",
            ip(192, 0, 2, 1),
            ip(10, 0, 0, 1),
        );
        p.excluded_cidrs = vec!["1.10.0.0/16".into()];
        mgr.upsert(p).unwrap();
        // Inside destination but inside excluded → no SNAT.
        let dec = mgr
            .evaluate(&ls(&[("app", "x")]), &ls(&[]), ip(1, 10, 0, 5), 0)
            .unwrap();
        assert!(dec.is_none());
        // Inside destination but outside excluded → match.
        let dec = mgr
            .evaluate(&ls(&[("app", "x")]), &ls(&[]), ip(1, 11, 0, 5), 0)
            .unwrap()
            .unwrap();
        assert_eq!(dec.policy_name, "egw");
    }

    #[test]
    fn egw_destination_cidr_outside_returns_none() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/egressgateway/manager.go",
            "Manager.Lookup.NoCidr",
            "tenant-egw-nocidr"
        );
        let mut mgr = EgressManager::new();
        mgr.upsert(make_policy(
            "egw",
            tenant,
            endpoint_sel(&[("app", "x")]),
            "10.0.0.0/8",
            ip(192, 0, 2, 1),
            ip(10, 0, 0, 1),
        ))
        .unwrap();
        let dec = mgr
            .evaluate(&ls(&[("app", "x")]), &ls(&[]), ip(8, 8, 8, 8), 0)
            .unwrap();
        assert!(dec.is_none());
    }

    #[test]
    fn egw_first_matching_policy_wins() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/egressgateway/manager.go",
            "Manager.Lookup.Order",
            "tenant-egw-order"
        );
        let mut mgr = EgressManager::new();
        mgr.upsert(make_policy(
            "first",
            tenant.clone(),
            endpoint_sel(&[("app", "x")]),
            "1.0.0.0/8",
            ip(192, 0, 2, 1),
            ip(10, 0, 0, 1),
        ))
        .unwrap();
        mgr.upsert(make_policy(
            "second",
            tenant,
            endpoint_sel(&[("app", "x")]),
            "1.0.0.0/8",
            ip(192, 0, 2, 2),
            ip(10, 0, 0, 2),
        ))
        .unwrap();
        let dec = mgr
            .evaluate(&ls(&[("app", "x")]), &ls(&[]), ip(1, 1, 1, 1), 0)
            .unwrap()
            .unwrap();
        assert_eq!(dec.policy_name, "first");
    }

    #[test]
    fn egw_namespace_selector_filters_pods() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/egressgateway/manager.go",
            "Manager.Lookup.Namespace",
            "tenant-egw-ns"
        );
        let mut mgr = EgressManager::new();
        let mut p = make_policy(
            "egw",
            tenant,
            endpoint_sel(&[("app", "x")]),
            "1.0.0.0/8",
            ip(192, 0, 2, 1),
            ip(10, 0, 0, 1),
        );
        p.source_namespace_selector =
            Some(endpoint_sel(&[("kubernetes.io/metadata.name", "prod")]));
        mgr.upsert(p).unwrap();
        // Pod in `prod` namespace → match.
        let dec = mgr
            .evaluate(
                &ls(&[("app", "x")]),
                &ls(&[("kubernetes.io/metadata.name", "prod")]),
                ip(1, 1, 1, 1),
                0,
            )
            .unwrap();
        assert!(dec.is_some());
        // Pod in `dev` namespace → no match.
        let dec = mgr
            .evaluate(
                &ls(&[("app", "x")]),
                &ls(&[("kubernetes.io/metadata.name", "dev")]),
                ip(1, 1, 1, 1),
                0,
            )
            .unwrap();
        assert!(dec.is_none());
    }

    // ── HA gateway pool ──────────────────────────────────────────────────────

    #[test]
    fn egw_ha_uses_hash_to_pick_gateway() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/egressgateway/manager.go",
            "Manager.SelectGateway",
            "tenant-egw-ha"
        );
        let mut mgr = EgressManager::new();
        let mut p = make_policy(
            "egw",
            tenant,
            endpoint_sel(&[("app", "x")]),
            "1.0.0.0/8",
            ip(192, 0, 2, 1),
            ip(10, 0, 0, 1),
        );
        p.gateway_nodes = vec![
            GatewayNode::new("gw1", ip(10, 0, 0, 1)),
            GatewayNode::new("gw2", ip(10, 0, 0, 2)),
            GatewayNode::new("gw3", ip(10, 0, 0, 3)),
        ];
        mgr.upsert(p).unwrap();
        let mut hits = std::collections::HashSet::new();
        for h in 0..30u64 {
            let dec = mgr
                .evaluate(&ls(&[("app", "x")]), &ls(&[]), ip(1, 1, 1, 1), h)
                .unwrap()
                .unwrap();
            hits.insert(dec.gateway_node);
        }
        assert!(hits.len() >= 2);
    }

    #[test]
    fn egw_ha_failover_skips_unhealthy_gateway() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/egressgateway/manager.go",
            "Manager.SelectGateway.Failover",
            "tenant-egw-fover"
        );
        let mut mgr = EgressManager::new();
        let mut p = make_policy(
            "egw",
            tenant,
            endpoint_sel(&[("app", "x")]),
            "1.0.0.0/8",
            ip(192, 0, 2, 1),
            ip(10, 0, 0, 1),
        );
        p.gateway_nodes = vec![
            GatewayNode {
                name: "gw1".into(),
                node_ip: ip(10, 0, 0, 1),
                state: GatewayState::Unhealthy,
            },
            GatewayNode::new("gw2", ip(10, 0, 0, 2)),
        ];
        mgr.upsert(p).unwrap();
        for h in 0..10u64 {
            let dec = mgr
                .evaluate(&ls(&[("app", "x")]), &ls(&[]), ip(1, 1, 1, 1), h)
                .unwrap()
                .unwrap();
            assert_eq!(dec.gateway_node, "gw2");
        }
    }

    #[test]
    fn egw_ha_all_unhealthy_returns_none() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/egressgateway/manager.go",
            "Manager.SelectGateway.NoHealthy",
            "tenant-egw-nh"
        );
        let mut mgr = EgressManager::new();
        let mut p = make_policy(
            "egw",
            tenant,
            endpoint_sel(&[("app", "x")]),
            "1.0.0.0/8",
            ip(192, 0, 2, 1),
            ip(10, 0, 0, 1),
        );
        p.gateway_nodes = vec![
            GatewayNode {
                name: "gw1".into(),
                node_ip: ip(10, 0, 0, 1),
                state: GatewayState::Unhealthy,
            },
            GatewayNode {
                name: "gw2".into(),
                node_ip: ip(10, 0, 0, 2),
                state: GatewayState::Draining,
            },
        ];
        mgr.upsert(p).unwrap();
        let dec = mgr
            .evaluate(&ls(&[("app", "x")]), &ls(&[]), ip(1, 1, 1, 1), 0)
            .unwrap();
        assert!(dec.is_none());
    }

    #[test]
    fn egw_ha_draining_excluded_from_pool() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/egressgateway/manager.go",
            "Manager.SelectGateway.Draining",
            "tenant-egw-drain"
        );
        let mut mgr = EgressManager::new();
        let mut p = make_policy(
            "egw",
            tenant,
            endpoint_sel(&[("app", "x")]),
            "1.0.0.0/8",
            ip(192, 0, 2, 1),
            ip(10, 0, 0, 1),
        );
        p.gateway_nodes = vec![
            GatewayNode {
                name: "gw1".into(),
                node_ip: ip(10, 0, 0, 1),
                state: GatewayState::Draining,
            },
            GatewayNode::new("gw2", ip(10, 0, 0, 2)),
        ];
        mgr.upsert(p).unwrap();
        for h in 0..10u64 {
            let dec = mgr
                .evaluate(&ls(&[("app", "x")]), &ls(&[]), ip(1, 1, 1, 1), h)
                .unwrap()
                .unwrap();
            assert_eq!(dec.gateway_node, "gw2");
        }
    }

    // ── Lifecycle ────────────────────────────────────────────────────────────

    #[test]
    fn egw_remove_policy_drops_route() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/egressgateway/manager.go",
            "Manager.Remove",
            "tenant-egw-rm"
        );
        let mut mgr = EgressManager::new();
        mgr.upsert(make_policy(
            "p",
            tenant,
            endpoint_sel(&[("app", "x")]),
            "1.0.0.0/8",
            ip(192, 0, 2, 1),
            ip(10, 0, 0, 1),
        ))
        .unwrap();
        assert!(mgr.remove("p"));
        assert!(mgr.is_empty());
        let dec = mgr
            .evaluate(&ls(&[("app", "x")]), &ls(&[]), ip(1, 1, 1, 1), 0)
            .unwrap();
        assert!(dec.is_none());
    }

    #[test]
    fn egw_remove_unknown_returns_false() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/egressgateway/manager.go",
            "Manager.Remove.NotFound",
            "tenant-egw-rm-nf"
        );
        let mut mgr = EgressManager::new();
        assert!(!mgr.remove("nope"));
    }

    #[test]
    fn egw_upsert_replaces_existing_policy_in_place() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/egressgateway/manager.go",
            "Manager.Upsert.Replace",
            "tenant-egw-upd"
        );
        let mut mgr = EgressManager::new();
        mgr.upsert(make_policy(
            "p",
            tenant.clone(),
            endpoint_sel(&[("app", "x")]),
            "1.0.0.0/8",
            ip(192, 0, 2, 1),
            ip(10, 0, 0, 1),
        ))
        .unwrap();
        mgr.upsert(make_policy(
            "p",
            tenant,
            endpoint_sel(&[("app", "x")]),
            "1.0.0.0/8",
            ip(192, 0, 2, 99),
            ip(10, 0, 0, 1),
        ))
        .unwrap();
        assert_eq!(mgr.len(), 1);
        let dec = mgr
            .evaluate(&ls(&[("app", "x")]), &ls(&[]), ip(1, 1, 1, 1), 0)
            .unwrap()
            .unwrap();
        assert_eq!(dec.egress_ip, ip(192, 0, 2, 99));
    }

    // ── Selector flavours ────────────────────────────────────────────────────

    #[test]
    fn egw_source_selector_uses_match_expressions() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/egressgateway/manager.go",
            "Manager.SourceSelector.Expr",
            "tenant-egw-expr"
        );
        let mut mgr = EgressManager::new();
        let sel = EndpointSelector {
            match_labels: HashMap::new(),
            match_expressions: vec![MatchExpression {
                key: "tier".into(),
                op: SelectorOp::In,
                values: vec!["frontend".into(), "edge".into()],
            }],
        };
        mgr.upsert(EgressGatewayPolicy {
            name: "p".into(),
            tenant,
            source_selector: sel,
            source_namespace_selector: None,
            destination_cidrs: vec!["1.0.0.0/8".into()],
            excluded_cidrs: vec![],
            egress_ip: ip(192, 0, 2, 1),
            gateway_nodes: vec![GatewayNode::new("gw", ip(10, 0, 0, 1))],
        })
        .unwrap();
        assert!(mgr
            .evaluate(&ls(&[("tier", "frontend")]), &ls(&[]), ip(1, 1, 1, 1), 0)
            .unwrap()
            .is_some());
        assert!(mgr
            .evaluate(&ls(&[("tier", "backend")]), &ls(&[]), ip(1, 1, 1, 1), 0)
            .unwrap()
            .is_none());
    }

    // ── IPv6 ─────────────────────────────────────────────────────────────────

    #[test]
    fn egw_destination_cidr_v6_match() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/egressgateway/manager.go",
            "Manager.Lookup.V6",
            "tenant-egw-v6"
        );
        let mut mgr = EgressManager::new();
        let p = EgressGatewayPolicy {
            name: "v6".into(),
            tenant,
            source_selector: endpoint_sel(&[("app", "x")]),
            source_namespace_selector: None,
            destination_cidrs: vec!["2001:db8::/32".into()],
            excluded_cidrs: vec![],
            egress_ip: "2001:db8:1::1".parse().unwrap(),
            gateway_nodes: vec![GatewayNode::new("gw", "2001:db8:1::ff".parse().unwrap())],
        };
        mgr.upsert(p).unwrap();
        let dst: IpAddr = "2001:db8:abcd::1".parse().unwrap();
        let dec = mgr
            .evaluate(&ls(&[("app", "x")]), &ls(&[]), dst, 0)
            .unwrap()
            .unwrap();
        assert_eq!(dec.policy_name, "v6");
    }

    // ── Multiple destination CIDRs ───────────────────────────────────────────

    #[test]
    fn egw_multiple_destination_cidrs_first_inside_match_wins() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/egressgateway/manager.go",
            "Manager.Lookup.MultiDest",
            "tenant-egw-multi"
        );
        let mut mgr = EgressManager::new();
        let mut p = make_policy(
            "p",
            tenant,
            endpoint_sel(&[("app", "x")]),
            "1.0.0.0/8",
            ip(192, 0, 2, 1),
            ip(10, 0, 0, 1),
        );
        p.destination_cidrs = vec!["1.0.0.0/8".into(), "2.0.0.0/8".into(), "3.0.0.0/8".into()];
        mgr.upsert(p).unwrap();
        for octet in [1u8, 2, 3] {
            let dec = mgr
                .evaluate(&ls(&[("app", "x")]), &ls(&[]), ip(octet, 0, 0, 1), 0)
                .unwrap();
            assert!(dec.is_some(), "octet {octet}");
        }
        assert!(mgr
            .evaluate(&ls(&[("app", "x")]), &ls(&[]), ip(4, 0, 0, 1), 0)
            .unwrap()
            .is_none());
    }

    // ── Serde round-trip ─────────────────────────────────────────────────────

    #[test]
    fn egw_policy_round_trips_through_serde() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/k8s/apis/cilium.io/v2/ciliumegressgatewaypolicy_types.go",
            "CiliumEgressGatewayPolicy",
            "tenant-egw-serde"
        );
        let p = make_policy(
            "p",
            tenant,
            endpoint_sel(&[("app", "x")]),
            "1.0.0.0/8",
            ip(192, 0, 2, 1),
            ip(10, 0, 0, 1),
        );
        let json = serde_json::to_string(&p).unwrap();
        let back: EgressGatewayPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn egw_evaluate_empty_manager_returns_none() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/egressgateway/manager.go",
            "Manager.Lookup.Empty",
            "tenant-egw-empty"
        );
        let mgr = EgressManager::new();
        let dec = mgr
            .evaluate(&ls(&[("app", "x")]), &ls(&[]), ip(1, 1, 1, 1), 0)
            .unwrap();
        assert!(dec.is_none());
    }

    #[test]
    fn egw_len_tracks_upserts() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/egressgateway/manager.go",
            "Manager.Len",
            "tenant-egw-len"
        );
        let mut mgr = EgressManager::new();
        for i in 0..5 {
            let p = make_policy(
                &format!("p-{i}"),
                tenant.clone(),
                endpoint_sel(&[("app", "x")]),
                "1.0.0.0/8",
                ip(192, 0, 2, 1 + i),
                ip(10, 0, 0, 1),
            );
            mgr.upsert(p).unwrap();
        }
        assert_eq!(mgr.len(), 5);
    }
}
