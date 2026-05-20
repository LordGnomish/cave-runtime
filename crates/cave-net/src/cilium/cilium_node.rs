// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CiliumNode CRD + cluster-pool CIDR allocator.
//!
//! Mirrors `pkg/k8s/apis/cilium.io/v2/types.go::CiliumNode` and the
//! per-node CIDR allocator in `pkg/ipam/clusterpool/clusterpool.go`.
//!
//! The `CiliumNode` CRD carries each node's view: pod CIDRs allocated
//! from the cluster pool, encryption key index, addresses, and per-node
//! IPAM watermarks. The cluster-pool allocator lives in
//! cilium-operator and carves `/24`-sized subnets out of a configured
//! cluster CIDR for each registered node.

use crate::cilium::types::{Cite, TenantId};
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CiliumNodeSpec {
    pub name: String,
    pub tenant: TenantId,
    pub addresses: Vec<NodeAddress>,
    pub ipam: NodeIpamSpec,
    pub encryption_key: u8,
    pub cluster_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeAddress {
    pub ip: IpAddr,
    pub kind: AddressKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AddressKind {
    InternalIP,
    ExternalIP,
    Hostname,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeIpamSpec {
    pub pod_cidrs: Vec<String>,
    pub used_ipv4: u64,
    pub used_ipv6: u64,
    pub pre_allocate: u32,
    pub max_above_watermark: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CiliumNodeStatus {
    pub state: NodeState,
    pub last_heartbeat_ns: u64,
    pub allocated_v4: u64,
    pub allocated_v6: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeState {
    Joining,
    Ready,
    NotReady,
    Decommissioning,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NodeError {
    #[error("node `{0}` already registered")]
    Duplicate(String),
    #[error("node `{0}` not found")]
    NotFound(String),
    #[error("invalid CIDR `{0}`")]
    BadCidr(String),
    #[error("cluster pool exhausted (allocated {0} subnets)")]
    PoolExhausted(u32),
    #[error("node already has a CIDR assigned")]
    AlreadyAllocated,
    #[error("tenant {tenant} cannot mutate cilium-node store owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct CiliumNodeStore {
    pub tenant: TenantId,
    nodes: BTreeMap<String, (CiliumNodeSpec, CiliumNodeStatus)>,
    /// Cluster-pool config: parent CIDR, per-node mask length.
    pub cluster_cidr: Option<String>,
    pub per_node_mask: u8,
    /// Set of subnet CIDRs already issued to a node.
    issued_subnets: BTreeSet<String>,
    /// node_name → assigned subnet (so we can revoke on deregister).
    subnet_owner: BTreeMap<String, String>,
}

impl CiliumNodeStore {
    pub fn new(tenant: TenantId) -> Self {
        Self {
            tenant,
            nodes: BTreeMap::new(),
            cluster_cidr: None,
            per_node_mask: 24,
            issued_subnets: BTreeSet::new(),
            subnet_owner: BTreeMap::new(),
        }
    }

    pub fn configure_cluster_pool(
        &mut self,
        cidr: impl Into<String>,
        per_node_mask: u8,
    ) -> Result<(), NodeError> {
        let cidr = cidr.into();
        IpNet::from_str(&cidr).map_err(|_| NodeError::BadCidr(cidr.clone()))?;
        self.cluster_cidr = Some(cidr);
        self.per_node_mask = per_node_mask;
        Ok(())
    }

    pub fn register(&mut self, spec: CiliumNodeSpec) -> Result<(), NodeError> {
        if self.nodes.contains_key(&spec.name) {
            return Err(NodeError::Duplicate(spec.name.clone()));
        }
        let status = CiliumNodeStatus {
            state: NodeState::Joining,
            last_heartbeat_ns: 0,
            allocated_v4: 0,
            allocated_v6: 0,
        };
        self.nodes.insert(spec.name.clone(), (spec, status));
        Ok(())
    }

    pub fn deregister(&mut self, name: &str) -> Result<(), NodeError> {
        self.nodes
            .remove(name)
            .ok_or_else(|| NodeError::NotFound(name.to_string()))?;
        if let Some(subnet) = self.subnet_owner.remove(name) {
            self.issued_subnets.remove(&subnet);
        }
        Ok(())
    }

    pub fn lookup(&self, name: &str) -> Option<&(CiliumNodeSpec, CiliumNodeStatus)> {
        self.nodes.get(name)
    }

    pub fn count(&self) -> usize {
        self.nodes.len()
    }

    /// Allocate the next free `/{per_node_mask}` subnet from the cluster
    /// pool to `node_name`. Returns the assigned subnet.
    pub fn allocate_pod_cidr(&mut self, node_name: &str) -> Result<String, NodeError> {
        let cidr = self
            .cluster_cidr
            .clone()
            .ok_or(NodeError::PoolExhausted(0))?;
        if !self.nodes.contains_key(node_name) {
            return Err(NodeError::NotFound(node_name.to_string()));
        }
        if self.subnet_owner.contains_key(node_name) {
            return Err(NodeError::AlreadyAllocated);
        }
        let parent = IpNet::from_str(&cidr).map_err(|_| NodeError::BadCidr(cidr.clone()))?;
        let subnets = parent
            .subnets(self.per_node_mask)
            .map_err(|_| NodeError::BadCidr(cidr.clone()))?;
        for s in subnets {
            let sstr = s.to_string();
            if !self.issued_subnets.contains(&sstr) {
                self.issued_subnets.insert(sstr.clone());
                self.subnet_owner
                    .insert(node_name.to_string(), sstr.clone());
                if let Some((spec, _)) = self.nodes.get_mut(node_name) {
                    spec.ipam.pod_cidrs.push(sstr.clone());
                }
                return Ok(sstr);
            }
        }
        Err(NodeError::PoolExhausted(self.issued_subnets.len() as u32))
    }

    pub fn release_pod_cidr(&mut self, node_name: &str) -> bool {
        if let Some(subnet) = self.subnet_owner.remove(node_name) {
            self.issued_subnets.remove(&subnet);
            if let Some((spec, _)) = self.nodes.get_mut(node_name) {
                spec.ipam.pod_cidrs.retain(|c| c != &subnet);
            }
            true
        } else {
            false
        }
    }

    pub fn heartbeat(&mut self, name: &str, now_ns: u64) -> Result<(), NodeError> {
        let (_, status) = self
            .nodes
            .get_mut(name)
            .ok_or_else(|| NodeError::NotFound(name.to_string()))?;
        status.last_heartbeat_ns = now_ns;
        if matches!(status.state, NodeState::Joining) {
            status.state = NodeState::Ready;
        }
        Ok(())
    }

    pub fn set_state(&mut self, name: &str, to: NodeState) -> Result<(), NodeError> {
        let (_, status) = self
            .nodes
            .get_mut(name)
            .ok_or_else(|| NodeError::NotFound(name.to_string()))?;
        status.state = to;
        Ok(())
    }

    pub fn issued_subnets(&self) -> &BTreeSet<String> {
        &self.issued_subnets
    }

    pub fn subnet_for(&self, name: &str) -> Option<&String> {
        self.subnet_owner.get(name)
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/k8s/apis/cilium.io/v2/types.go", "CiliumNode");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    fn spec(name: &str, tenant: TenantId) -> CiliumNodeSpec {
        CiliumNodeSpec {
            name: name.into(),
            tenant,
            addresses: vec![NodeAddress {
                ip: ip(10, 0, 0, 1),
                kind: AddressKind::InternalIP,
            }],
            ipam: NodeIpamSpec {
                pod_cidrs: vec![],
                used_ipv4: 0,
                used_ipv6: 0,
                pre_allocate: 8,
                max_above_watermark: 16,
            },
            encryption_key: 0,
            cluster_id: 1,
        }
    }

    fn store(tenant: TenantId) -> CiliumNodeStore {
        CiliumNodeStore::new(tenant)
    }

    // ── Cluster pool config ─────────────────────────────────────────────────

    #[test]
    fn store_configure_cluster_pool() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ipam/clusterpool/clusterpool.go",
            "Configure",
            "tenant-cn-cfg"
        );
        let mut s = store(tenant);
        s.configure_cluster_pool("10.244.0.0/16", 24).unwrap();
        assert_eq!(s.cluster_cidr.as_deref(), Some("10.244.0.0/16"));
        assert_eq!(s.per_node_mask, 24);
    }

    #[test]
    fn store_configure_bad_cidr_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ipam/clusterpool/clusterpool.go",
            "Configure.BadCidr",
            "tenant-cn-cfgbad"
        );
        let mut s = store(tenant);
        let err = s.configure_cluster_pool("nope", 24).unwrap_err();
        assert_eq!(err, NodeError::BadCidr("nope".into()));
    }

    // ── Registration ────────────────────────────────────────────────────────

    #[test]
    fn store_register_succeeds() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/k8s/apis/cilium.io/v2/types.go",
            "CiliumNode.Register",
            "tenant-cn-reg"
        );
        let mut s = store(tenant.clone());
        s.register(spec("node-a", tenant)).unwrap();
        assert_eq!(s.count(), 1);
    }

    #[test]
    fn store_register_duplicate_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/k8s/apis/cilium.io/v2/types.go",
            "CiliumNode.Register.Dup",
            "tenant-cn-dup"
        );
        let mut s = store(tenant.clone());
        s.register(spec("node-a", tenant.clone())).unwrap();
        let err = s.register(spec("node-a", tenant)).unwrap_err();
        assert!(matches!(err, NodeError::Duplicate(_)));
    }

    #[test]
    fn store_lookup_returns_node() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/k8s/apis/cilium.io/v2/types.go",
            "CiliumNode.Lookup",
            "tenant-cn-lk"
        );
        let mut s = store(tenant.clone());
        s.register(spec("node-a", tenant)).unwrap();
        assert!(s.lookup("node-a").is_some());
    }

    #[test]
    fn store_lookup_unknown_returns_none() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/k8s/apis/cilium.io/v2/types.go",
            "CiliumNode.Lookup.NotFound",
            "tenant-cn-lknf"
        );
        let s = store(tenant);
        assert!(s.lookup("ghost").is_none());
    }

    #[test]
    fn store_deregister_drops_node_and_subnet() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/k8s/apis/cilium.io/v2/types.go",
            "CiliumNode.Deregister",
            "tenant-cn-rm"
        );
        let mut s = store(tenant.clone());
        s.configure_cluster_pool("10.244.0.0/16", 24).unwrap();
        s.register(spec("node-a", tenant)).unwrap();
        s.allocate_pod_cidr("node-a").unwrap();
        s.deregister("node-a").unwrap();
        assert_eq!(s.count(), 0);
        assert!(s.issued_subnets().is_empty());
    }

    #[test]
    fn store_deregister_unknown_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/k8s/apis/cilium.io/v2/types.go",
            "CiliumNode.Deregister.NotFound",
            "tenant-cn-rmnf"
        );
        let mut s = store(tenant);
        let err = s.deregister("ghost").unwrap_err();
        assert!(matches!(err, NodeError::NotFound(_)));
    }

    // ── CIDR allocation ─────────────────────────────────────────────────────

    #[test]
    fn allocate_first_subnet_starts_at_zero_offset() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ipam/clusterpool/clusterpool.go",
            "Allocate.First",
            "tenant-cn-af"
        );
        let mut s = store(tenant.clone());
        s.configure_cluster_pool("10.244.0.0/16", 24).unwrap();
        s.register(spec("node-a", tenant)).unwrap();
        let subnet = s.allocate_pod_cidr("node-a").unwrap();
        assert_eq!(subnet, "10.244.0.0/24");
    }

    #[test]
    fn allocate_subsequent_subnets_advance() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ipam/clusterpool/clusterpool.go",
            "Allocate.Sequential",
            "tenant-cn-as"
        );
        let mut s = store(tenant.clone());
        s.configure_cluster_pool("10.244.0.0/16", 24).unwrap();
        s.register(spec("node-a", tenant.clone())).unwrap();
        s.register(spec("node-b", tenant)).unwrap();
        let a = s.allocate_pod_cidr("node-a").unwrap();
        let b = s.allocate_pod_cidr("node-b").unwrap();
        assert_eq!(a, "10.244.0.0/24");
        assert_eq!(b, "10.244.1.0/24");
    }

    #[test]
    fn allocate_records_subnet_in_node_spec() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ipam/clusterpool/clusterpool.go",
            "Allocate.SpecRecord",
            "tenant-cn-asr"
        );
        let mut s = store(tenant.clone());
        s.configure_cluster_pool("10.244.0.0/16", 24).unwrap();
        s.register(spec("node-a", tenant)).unwrap();
        let subnet = s.allocate_pod_cidr("node-a").unwrap();
        let (sp, _) = s.lookup("node-a").unwrap();
        assert!(sp.ipam.pod_cidrs.contains(&subnet));
    }

    #[test]
    fn allocate_double_for_same_node_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ipam/clusterpool/clusterpool.go",
            "Allocate.AlreadyAllocated",
            "tenant-cn-aa"
        );
        let mut s = store(tenant.clone());
        s.configure_cluster_pool("10.244.0.0/16", 24).unwrap();
        s.register(spec("node-a", tenant)).unwrap();
        s.allocate_pod_cidr("node-a").unwrap();
        let err = s.allocate_pod_cidr("node-a").unwrap_err();
        assert_eq!(err, NodeError::AlreadyAllocated);
    }

    #[test]
    fn allocate_unknown_node_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ipam/clusterpool/clusterpool.go",
            "Allocate.UnknownNode",
            "tenant-cn-aun"
        );
        let mut s = store(tenant);
        s.configure_cluster_pool("10.244.0.0/16", 24).unwrap();
        let err = s.allocate_pod_cidr("ghost").unwrap_err();
        assert!(matches!(err, NodeError::NotFound(_)));
    }

    #[test]
    fn allocate_without_cluster_cidr_returns_pool_exhausted() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ipam/clusterpool/clusterpool.go",
            "Allocate.NoPool",
            "tenant-cn-nop"
        );
        let mut s = store(tenant.clone());
        s.register(spec("node-a", tenant)).unwrap();
        let err = s.allocate_pod_cidr("node-a").unwrap_err();
        assert!(matches!(err, NodeError::PoolExhausted(_)));
    }

    #[test]
    fn release_drops_subnet_and_returns_to_pool() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ipam/clusterpool/clusterpool.go",
            "Release",
            "tenant-cn-rel"
        );
        let mut s = store(tenant.clone());
        s.configure_cluster_pool("10.244.0.0/16", 24).unwrap();
        s.register(spec("node-a", tenant.clone())).unwrap();
        s.register(spec("node-b", tenant)).unwrap();
        s.allocate_pod_cidr("node-a").unwrap();
        let b1 = s.allocate_pod_cidr("node-b").unwrap();
        assert!(s.release_pod_cidr("node-a"));
        // Re-allocate to a fresh node — should reuse 10.244.0.0/24.
        let mut newspec = spec("node-c", s.tenant.clone());
        newspec.tenant = s.tenant.clone();
        s.register(newspec).unwrap();
        let c = s.allocate_pod_cidr("node-c").unwrap();
        assert_eq!(c, "10.244.0.0/24");
        // node-b's subnet is unchanged.
        assert_eq!(s.subnet_for("node-b"), Some(&b1));
    }

    #[test]
    fn release_unknown_returns_false() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ipam/clusterpool/clusterpool.go",
            "Release.NotFound",
            "tenant-cn-relnf"
        );
        let mut s = store(tenant);
        assert!(!s.release_pod_cidr("ghost"));
    }

    // ── State transitions ──────────────────────────────────────────────────

    #[test]
    fn node_initial_state_joining() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/k8s/apis/cilium.io/v2/types.go",
            "CiliumNode.State.Joining",
            "tenant-cn-stj"
        );
        let mut s = store(tenant.clone());
        s.register(spec("node-a", tenant)).unwrap();
        assert_eq!(s.lookup("node-a").unwrap().1.state, NodeState::Joining);
    }

    #[test]
    fn heartbeat_advances_joining_to_ready() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/k8s/apis/cilium.io/v2/types.go",
            "CiliumNode.Heartbeat",
            "tenant-cn-hbr"
        );
        let mut s = store(tenant.clone());
        s.register(spec("node-a", tenant)).unwrap();
        s.heartbeat("node-a", 100).unwrap();
        assert_eq!(s.lookup("node-a").unwrap().1.state, NodeState::Ready);
    }

    #[test]
    fn set_state_explicitly_changes_state() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/k8s/apis/cilium.io/v2/types.go",
            "CiliumNode.SetState",
            "tenant-cn-ss"
        );
        let mut s = store(tenant.clone());
        s.register(spec("node-a", tenant)).unwrap();
        s.set_state("node-a", NodeState::Decommissioning).unwrap();
        assert_eq!(
            s.lookup("node-a").unwrap().1.state,
            NodeState::Decommissioning
        );
    }

    #[test]
    fn heartbeat_unknown_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/k8s/apis/cilium.io/v2/types.go",
            "CiliumNode.Heartbeat.NotFound",
            "tenant-cn-hbnf"
        );
        let mut s = store(tenant);
        let err = s.heartbeat("ghost", 100).unwrap_err();
        assert!(matches!(err, NodeError::NotFound(_)));
    }

    // ── Multi-node ─────────────────────────────────────────────────────────

    #[test]
    fn multiple_nodes_get_distinct_subnets() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ipam/clusterpool/clusterpool.go",
            "Allocate.Distinct",
            "tenant-cn-md"
        );
        let mut s = store(tenant.clone());
        s.configure_cluster_pool("10.244.0.0/16", 24).unwrap();
        for i in 0..5u8 {
            s.register(spec(&format!("node-{i}"), tenant.clone()))
                .unwrap();
            s.allocate_pod_cidr(&format!("node-{i}")).unwrap();
        }
        assert_eq!(s.issued_subnets().len(), 5);
    }

    #[test]
    fn count_tracks_registrations() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/k8s/apis/cilium.io/v2/types.go",
            "CiliumNode.Count",
            "tenant-cn-cnt"
        );
        let mut s = store(tenant.clone());
        for i in 0..7u8 {
            s.register(spec(&format!("n-{i}"), tenant.clone())).unwrap();
        }
        assert_eq!(s.count(), 7);
    }

    // ── Serde ───────────────────────────────────────────────────────────────

    #[test]
    fn cilium_node_spec_serde_round_trip() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/k8s/apis/cilium.io/v2/types.go",
            "CiliumNode.Spec.Serde",
            "tenant-cn-sserde"
        );
        let s = spec("node-a", tenant);
        let json = serde_json::to_string(&s).unwrap();
        let back: CiliumNodeSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn node_state_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/k8s/apis/cilium.io/v2/types.go",
            "CiliumNode.State.Serde",
            "tenant-cn-stserde"
        );
        for st in [
            NodeState::Joining,
            NodeState::Ready,
            NodeState::NotReady,
            NodeState::Decommissioning,
        ] {
            let s = serde_json::to_string(&st).unwrap();
            let back: NodeState = serde_json::from_str(&s).unwrap();
            assert_eq!(back, st);
        }
    }
}
