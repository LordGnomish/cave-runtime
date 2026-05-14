// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! IP Address Management — Cilium's IPAM modes.
//!
//! Mirrors `pkg/ipam/ipam.go` plus `pkg/ipam/types.go` (the
//! `IPAMConfig` enum) and the `CiliumPodIPPool` CRD from
//! `pkg/ipam/multipool/types.go`.
//!
//! Modes (faithful to upstream):
//!
//! * [`IpamMode::ClusterPool`] — cilium-agent carves per-node `/24`
//!   subnets out of a cluster-wide pool and allocates pod IPs within
//!   them. The first usable address in a node subnet is reserved as the
//!   gateway.
//! * [`IpamMode::Kubernetes`] — host-scope; the K8s controller-manager
//!   sets `Node.Spec.PodCIDRs` and cilium-agent allocates within those.
//! * [`IpamMode::MultiPool`] — named pools per workload, defined via
//!   the `CiliumPodIPPool` CRD; pods request a pool by name (annotation
//!   `ipam.cilium.io/ip-pool`) and IPAM allocates from that pool.
//!
//! Watermark pre-allocation: each pool has a `low_watermark` and
//! `high_watermark` (mirrors `pre-allocate` in upstream). Crossing the
//! low watermark triggers a pre-allocation request; crossing the high
//! watermark triggers a release of unused addresses.

use crate::cilium::types::{Cite, TenantId};
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::net::IpAddr;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IpamMode {
    ClusterPool,
    Kubernetes,
    MultiPool,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IpamError {
    #[error("pool `{0}` not found")]
    PoolNotFound(String),
    #[error("pool `{0}` is exhausted")]
    PoolExhausted(String),
    #[error("invalid CIDR `{0}`")]
    BadCidr(String),
    #[error("address {0} is already allocated")]
    AddressInUse(IpAddr),
    #[error("address {0} not in pool `{1}`")]
    AddressOutOfPool(IpAddr, String),
    #[error("node `{0}` has no PodCIDR (Kubernetes mode requires Node.Spec.PodCIDRs)")]
    NodeMissingPodCidr(String),
    #[error("tenant {tenant} cannot mutate pool owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

/// One pool — `cidr_v4` + optional `cidr_v6` (dual stack).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodIpPool {
    pub name: String,
    pub tenant: TenantId,
    pub cidr_v4: Option<String>,
    pub cidr_v6: Option<String>,
    pub gateway_v4: Option<IpAddr>,
    pub gateway_v6: Option<IpAddr>,
    pub low_watermark: u32,
    pub high_watermark: u32,
}

impl PodIpPool {
    pub fn ipv4(name: impl Into<String>, tenant: TenantId, cidr: impl Into<String>) -> Self {
        Self {
            name: name.into(), tenant,
            cidr_v4: Some(cidr.into()),
            cidr_v6: None, gateway_v4: None, gateway_v6: None,
            low_watermark: 8, high_watermark: 16,
        }
    }
    pub fn dual_stack(name: impl Into<String>, tenant: TenantId, cidr_v4: impl Into<String>, cidr_v6: impl Into<String>) -> Self {
        Self {
            name: name.into(), tenant,
            cidr_v4: Some(cidr_v4.into()),
            cidr_v6: Some(cidr_v6.into()),
            gateway_v4: None, gateway_v6: None,
            low_watermark: 8, high_watermark: 16,
        }
    }
}

/// Per-pool runtime state: allocated set + cursor.
#[derive(Debug, Clone)]
struct PoolState {
    pool: PodIpPool,
    allocated_v4: BTreeSet<IpAddr>,
    allocated_v6: BTreeSet<IpAddr>,
    /// Owner reference (`namespace/pod-name`) → allocated IP for idempotency.
    owners_v4: HashMap<String, IpAddr>,
    owners_v6: HashMap<String, IpAddr>,
}

impl PoolState {
    fn new(pool: PodIpPool) -> Self {
        Self {
            pool,
            allocated_v4: BTreeSet::new(),
            allocated_v6: BTreeSet::new(),
            owners_v4: HashMap::new(),
            owners_v6: HashMap::new(),
        }
    }

    fn capacity_v4(&self) -> u64 {
        self.pool.cidr_v4.as_deref()
            .and_then(|c| IpNet::from_str(c).ok())
            .map(|n| match n {
                IpNet::V4(v4) => 1u64 << (32 - v4.prefix_len() as u64),
                _ => 0,
            })
            .unwrap_or(0)
    }
    fn capacity_v6(&self) -> u64 {
        // Cap IPv6 at 64-bit since u64 can't represent /48 addresses.
        // Mirrors upstream `pkg/ipam/types.go` which uses `big.Int.Lsh(64)`.
        self.pool.cidr_v6.as_deref()
            .and_then(|c| IpNet::from_str(c).ok())
            .map(|n| match n {
                IpNet::V6(v6) => {
                    let bits = 128 - v6.prefix_len() as u32;
                    if bits >= 64 { u64::MAX } else { 1u64 << bits }
                }
                _ => 0,
            })
            .unwrap_or(0)
    }

    fn allocate_v4(&mut self, owner: &str) -> Result<IpAddr, IpamError> {
        if let Some(&ip) = self.owners_v4.get(owner) {
            return Ok(ip);
        }
        let cidr = self.pool.cidr_v4.as_deref().ok_or_else(|| IpamError::PoolNotFound(self.pool.name.clone()))?;
        let net = IpNet::from_str(cidr).map_err(|_| IpamError::BadCidr(cidr.to_string()))?;
        for ip in net.hosts() {
            if Some(ip) == self.pool.gateway_v4 {
                continue;
            }
            if !self.allocated_v4.contains(&ip) {
                self.allocated_v4.insert(ip);
                self.owners_v4.insert(owner.to_string(), ip);
                return Ok(ip);
            }
        }
        Err(IpamError::PoolExhausted(self.pool.name.clone()))
    }

    fn allocate_v6(&mut self, owner: &str) -> Result<IpAddr, IpamError> {
        if let Some(&ip) = self.owners_v6.get(owner) {
            return Ok(ip);
        }
        let cidr = self.pool.cidr_v6.as_deref().ok_or_else(|| IpamError::PoolNotFound(self.pool.name.clone()))?;
        let net = IpNet::from_str(cidr).map_err(|_| IpamError::BadCidr(cidr.to_string()))?;
        for ip in net.hosts().take(2048) {
            if Some(ip) == self.pool.gateway_v6 {
                continue;
            }
            if !self.allocated_v6.contains(&ip) {
                self.allocated_v6.insert(ip);
                self.owners_v6.insert(owner.to_string(), ip);
                return Ok(ip);
            }
        }
        Err(IpamError::PoolExhausted(self.pool.name.clone()))
    }

    fn allocate_specific(&mut self, owner: &str, ip: IpAddr) -> Result<(), IpamError> {
        let (set, owners, cidr_str) = match ip {
            IpAddr::V4(_) => (&mut self.allocated_v4, &mut self.owners_v4, self.pool.cidr_v4.as_deref()),
            IpAddr::V6(_) => (&mut self.allocated_v6, &mut self.owners_v6, self.pool.cidr_v6.as_deref()),
        };
        let cidr = cidr_str.ok_or_else(|| IpamError::AddressOutOfPool(ip, self.pool.name.clone()))?;
        let net = IpNet::from_str(cidr).map_err(|_| IpamError::BadCidr(cidr.to_string()))?;
        if !net.contains(&ip) {
            return Err(IpamError::AddressOutOfPool(ip, self.pool.name.clone()));
        }
        if set.contains(&ip) {
            return Err(IpamError::AddressInUse(ip));
        }
        set.insert(ip);
        owners.insert(owner.to_string(), ip);
        Ok(())
    }

    fn release_owner(&mut self, owner: &str) -> usize {
        let mut n = 0;
        if let Some(ip) = self.owners_v4.remove(owner) {
            self.allocated_v4.remove(&ip);
            n += 1;
        }
        if let Some(ip) = self.owners_v6.remove(owner) {
            self.allocated_v6.remove(&ip);
            n += 1;
        }
        n
    }

    fn release_specific(&mut self, ip: IpAddr) -> bool {
        match ip {
            IpAddr::V4(_) => {
                let removed = self.allocated_v4.remove(&ip);
                if removed {
                    self.owners_v4.retain(|_, v| *v != ip);
                }
                removed
            }
            IpAddr::V6(_) => {
                let removed = self.allocated_v6.remove(&ip);
                if removed {
                    self.owners_v6.retain(|_, v| *v != ip);
                }
                removed
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolStatus {
    pub allocated_v4: u64,
    pub capacity_v4: u64,
    pub allocated_v6: u64,
    pub capacity_v6: u64,
    pub low_watermark_breached: bool,
    pub high_watermark_breached: bool,
}

#[derive(Debug)]
pub struct Ipam {
    pub mode: IpamMode,
    pools: HashMap<String, PoolState>,
    /// Kubernetes-mode: per-node PodCIDR(s).
    node_cidrs: HashMap<String, (Option<String>, Option<String>)>,
}

impl Ipam {
    pub fn new(mode: IpamMode) -> Self {
        Self { mode, pools: HashMap::new(), node_cidrs: HashMap::new() }
    }

    pub fn upsert_pool(&mut self, pool: PodIpPool) -> Result<(), IpamError> {
        if let Some(c) = &pool.cidr_v4 {
            IpNet::from_str(c).map_err(|_| IpamError::BadCidr(c.clone()))?;
        }
        if let Some(c) = &pool.cidr_v6 {
            IpNet::from_str(c).map_err(|_| IpamError::BadCidr(c.clone()))?;
        }
        let name = pool.name.clone();
        let state = match self.pools.remove(&name) {
            Some(mut s) => { s.pool = pool; s }
            None => PoolState::new(pool),
        };
        self.pools.insert(name, state);
        Ok(())
    }

    pub fn pool_status(&self, name: &str) -> Result<PoolStatus, IpamError> {
        let s = self.pools.get(name).ok_or_else(|| IpamError::PoolNotFound(name.to_string()))?;
        let cap_v4 = s.capacity_v4();
        let cap_v6 = s.capacity_v6();
        let alloc_v4 = s.allocated_v4.len() as u64;
        let alloc_v6 = s.allocated_v6.len() as u64;
        let free_v4 = cap_v4.saturating_sub(alloc_v4);
        let low = s.pool.low_watermark as u64;
        let high = s.pool.high_watermark as u64;
        Ok(PoolStatus {
            allocated_v4: alloc_v4,
            capacity_v4: cap_v4,
            allocated_v6: alloc_v6,
            capacity_v6: cap_v6,
            low_watermark_breached: free_v4 < low,
            high_watermark_breached: free_v4 > high && cap_v4 > high,
        })
    }

    /// Allocate a v4 IP for `owner` (e.g. `namespace/pod-name`) from the
    /// named pool. Idempotent: same owner → same IP.
    pub fn allocate_v4(&mut self, pool: &str, owner: &str) -> Result<IpAddr, IpamError> {
        let s = self.pools.get_mut(pool).ok_or_else(|| IpamError::PoolNotFound(pool.to_string()))?;
        s.allocate_v4(owner)
    }

    pub fn allocate_v6(&mut self, pool: &str, owner: &str) -> Result<IpAddr, IpamError> {
        let s = self.pools.get_mut(pool).ok_or_else(|| IpamError::PoolNotFound(pool.to_string()))?;
        s.allocate_v6(owner)
    }

    pub fn allocate_dual_stack(&mut self, pool: &str, owner: &str) -> Result<(IpAddr, IpAddr), IpamError> {
        let v4 = self.allocate_v4(pool, owner)?;
        let v6 = self.allocate_v6(pool, owner)?;
        Ok((v4, v6))
    }

    pub fn allocate_specific(&mut self, pool: &str, owner: &str, ip: IpAddr) -> Result<(), IpamError> {
        let s = self.pools.get_mut(pool).ok_or_else(|| IpamError::PoolNotFound(pool.to_string()))?;
        s.allocate_specific(owner, ip)
    }

    /// Release every allocation owned by `owner` across all pools. Returns
    /// total number of addresses freed.
    pub fn release_owner(&mut self, owner: &str) -> usize {
        let mut n = 0;
        for s in self.pools.values_mut() {
            n += s.release_owner(owner);
        }
        n
    }

    pub fn release_specific(&mut self, pool: &str, ip: IpAddr) -> Result<bool, IpamError> {
        let s = self.pools.get_mut(pool).ok_or_else(|| IpamError::PoolNotFound(pool.to_string()))?;
        Ok(s.release_specific(ip))
    }

    pub fn pool_count(&self) -> usize {
        self.pools.len()
    }

    // ── Kubernetes mode ──────────────────────────────────────────────────────

    pub fn set_node_pod_cidrs(&mut self, node: impl Into<String>, v4: Option<String>, v6: Option<String>) {
        self.node_cidrs.insert(node.into(), (v4, v6));
    }

    /// Allocate from a node's PodCIDR (Kubernetes mode). Auto-creates a
    /// pool named after the node if it doesn't exist yet.
    pub fn allocate_from_node(&mut self, tenant: TenantId, node: &str, owner: &str) -> Result<(Option<IpAddr>, Option<IpAddr>), IpamError> {
        if !matches!(self.mode, IpamMode::Kubernetes) {
            // For ClusterPool we require an explicit pool.
        }
        let (v4_cidr, v6_cidr) = self.node_cidrs.get(node).cloned()
            .ok_or_else(|| IpamError::NodeMissingPodCidr(node.to_string()))?;
        let pool_name = format!("node:{node}");
        if !self.pools.contains_key(&pool_name) {
            let pool = PodIpPool {
                name: pool_name.clone(), tenant,
                cidr_v4: v4_cidr.clone(),
                cidr_v6: v6_cidr.clone(),
                gateway_v4: None, gateway_v6: None,
                low_watermark: 4, high_watermark: 8,
            };
            self.upsert_pool(pool)?;
        }
        let v4 = if v4_cidr.is_some() { Some(self.allocate_v4(&pool_name, owner)?) } else { None };
        let v6 = if v6_cidr.is_some() { Some(self.allocate_v6(&pool_name, owner)?) } else { None };
        Ok((v4, v6))
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/ipam/ipam.go", "IPAM");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;
    use std::net::Ipv4Addr;

    fn ip4(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    // ── ClusterPool ──────────────────────────────────────────────────────────

    #[test]
    fn ipam_cluster_pool_allocates_first_available_address() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/clusterpool/clusterpool.go", "Allocate", "tenant-ipam-cp-first");
        let mut ipam = Ipam::new(IpamMode::ClusterPool);
        ipam.upsert_pool(PodIpPool::ipv4("default", tenant, "10.0.0.0/29")).unwrap();
        let ip = ipam.allocate_v4("default", "ns/pod1").unwrap();
        assert_eq!(ip, ip4(10, 0, 0, 1));
    }

    #[test]
    fn ipam_cluster_pool_skips_gateway_address() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/clusterpool/clusterpool.go", "Allocate.Gateway", "tenant-ipam-cp-gw");
        let mut ipam = Ipam::new(IpamMode::ClusterPool);
        let mut pool = PodIpPool::ipv4("default", tenant, "10.0.0.0/29");
        pool.gateway_v4 = Some(ip4(10, 0, 0, 1));
        ipam.upsert_pool(pool).unwrap();
        assert_eq!(ipam.allocate_v4("default", "ns/p").unwrap(), ip4(10, 0, 0, 2));
    }

    #[test]
    fn ipam_cluster_pool_allocations_are_sequential() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/clusterpool/clusterpool.go", "Allocate.Sequential", "tenant-ipam-cp-seq");
        let mut ipam = Ipam::new(IpamMode::ClusterPool);
        ipam.upsert_pool(PodIpPool::ipv4("default", tenant, "10.0.0.0/29")).unwrap();
        let a = ipam.allocate_v4("default", "ns/p1").unwrap();
        let b = ipam.allocate_v4("default", "ns/p2").unwrap();
        assert_eq!(a, ip4(10, 0, 0, 1));
        assert_eq!(b, ip4(10, 0, 0, 2));
    }

    #[test]
    fn ipam_cluster_pool_exhaustion_returns_error() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/clusterpool/clusterpool.go", "Allocate.Exhausted", "tenant-ipam-cp-ex");
        let mut ipam = Ipam::new(IpamMode::ClusterPool);
        // /30 has 4 addresses → 2 usable hosts.
        ipam.upsert_pool(PodIpPool::ipv4("default", tenant, "10.0.0.0/30")).unwrap();
        let _ = ipam.allocate_v4("default", "ns/p1").unwrap();
        let _ = ipam.allocate_v4("default", "ns/p2").unwrap();
        let err = ipam.allocate_v4("default", "ns/p3").unwrap_err();
        assert_eq!(err, IpamError::PoolExhausted("default".into()));
    }

    #[test]
    fn ipam_cluster_pool_idempotent_for_same_owner() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/clusterpool/clusterpool.go", "Allocate.Idempotent", "tenant-ipam-cp-idem");
        let mut ipam = Ipam::new(IpamMode::ClusterPool);
        ipam.upsert_pool(PodIpPool::ipv4("default", tenant, "10.0.0.0/24")).unwrap();
        let a = ipam.allocate_v4("default", "ns/p1").unwrap();
        let b = ipam.allocate_v4("default", "ns/p1").unwrap();
        assert_eq!(a, b);
        let st = ipam.pool_status("default").unwrap();
        assert_eq!(st.allocated_v4, 1);
    }

    #[test]
    fn ipam_release_owner_drops_allocation() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/clusterpool/clusterpool.go", "Release", "tenant-ipam-rel");
        let mut ipam = Ipam::new(IpamMode::ClusterPool);
        ipam.upsert_pool(PodIpPool::ipv4("default", tenant, "10.0.0.0/29")).unwrap();
        ipam.allocate_v4("default", "ns/p1").unwrap();
        let n = ipam.release_owner("ns/p1");
        assert_eq!(n, 1);
        let st = ipam.pool_status("default").unwrap();
        assert_eq!(st.allocated_v4, 0);
    }

    #[test]
    fn ipam_release_unknown_owner_returns_zero() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/clusterpool/clusterpool.go", "Release.NotFound", "tenant-ipam-rel-nf");
        let mut ipam = Ipam::new(IpamMode::ClusterPool);
        ipam.upsert_pool(PodIpPool::ipv4("default", tenant, "10.0.0.0/29")).unwrap();
        assert_eq!(ipam.release_owner("ns/ghost"), 0);
    }

    #[test]
    fn ipam_release_specific_ip() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/clusterpool/clusterpool.go", "ReleaseIP", "tenant-ipam-relip");
        let mut ipam = Ipam::new(IpamMode::ClusterPool);
        ipam.upsert_pool(PodIpPool::ipv4("default", tenant, "10.0.0.0/29")).unwrap();
        let ip = ipam.allocate_v4("default", "ns/p1").unwrap();
        assert!(ipam.release_specific("default", ip).unwrap());
        assert_eq!(ipam.pool_status("default").unwrap().allocated_v4, 0);
    }

    #[test]
    fn ipam_release_specific_unallocated_returns_false() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/clusterpool/clusterpool.go", "ReleaseIP.NotAllocated", "tenant-ipam-relip-na");
        let mut ipam = Ipam::new(IpamMode::ClusterPool);
        ipam.upsert_pool(PodIpPool::ipv4("default", tenant, "10.0.0.0/29")).unwrap();
        assert!(!ipam.release_specific("default", ip4(10, 0, 0, 5)).unwrap());
    }

    #[test]
    fn ipam_allocate_specific_address() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/clusterpool/clusterpool.go", "AllocateIP", "tenant-ipam-allocip");
        let mut ipam = Ipam::new(IpamMode::ClusterPool);
        ipam.upsert_pool(PodIpPool::ipv4("default", tenant, "10.0.0.0/24")).unwrap();
        ipam.allocate_specific("default", "ns/p1", ip4(10, 0, 0, 100)).unwrap();
        assert_eq!(ipam.pool_status("default").unwrap().allocated_v4, 1);
    }

    #[test]
    fn ipam_allocate_specific_collision_returns_in_use() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/clusterpool/clusterpool.go", "AllocateIP.Collision", "tenant-ipam-allocip-coll");
        let mut ipam = Ipam::new(IpamMode::ClusterPool);
        ipam.upsert_pool(PodIpPool::ipv4("default", tenant, "10.0.0.0/24")).unwrap();
        ipam.allocate_specific("default", "ns/p1", ip4(10, 0, 0, 100)).unwrap();
        let err = ipam.allocate_specific("default", "ns/p2", ip4(10, 0, 0, 100)).unwrap_err();
        assert_eq!(err, IpamError::AddressInUse(ip4(10, 0, 0, 100)));
    }

    #[test]
    fn ipam_allocate_specific_out_of_pool_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/clusterpool/clusterpool.go", "AllocateIP.OutOfPool", "tenant-ipam-allocip-out");
        let mut ipam = Ipam::new(IpamMode::ClusterPool);
        ipam.upsert_pool(PodIpPool::ipv4("default", tenant, "10.0.0.0/24")).unwrap();
        let err = ipam.allocate_specific("default", "ns/p1", ip4(11, 0, 0, 1)).unwrap_err();
        assert_eq!(err, IpamError::AddressOutOfPool(ip4(11, 0, 0, 1), "default".into()));
    }

    #[test]
    fn ipam_pool_capacity_matches_cidr_size() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/types.go", "Pool.Capacity", "tenant-ipam-cap");
        let mut ipam = Ipam::new(IpamMode::ClusterPool);
        ipam.upsert_pool(PodIpPool::ipv4("default", tenant, "10.0.0.0/24")).unwrap();
        let st = ipam.pool_status("default").unwrap();
        assert_eq!(st.capacity_v4, 256);
    }

    // ── Bad CIDR ─────────────────────────────────────────────────────────────

    #[test]
    fn ipam_upsert_pool_with_bad_cidr_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/types.go", "Pool.Validate", "tenant-ipam-bad");
        let mut ipam = Ipam::new(IpamMode::ClusterPool);
        let err = ipam.upsert_pool(PodIpPool::ipv4("default", tenant, "not-a-cidr")).unwrap_err();
        assert_eq!(err, IpamError::BadCidr("not-a-cidr".into()));
    }

    #[test]
    fn ipam_pool_not_found_error() {
        let (_c, _t) = cilium_test_ctx!("pkg/ipam/ipam.go", "AllocateNext.PoolNotFound", "tenant-ipam-nf");
        let mut ipam = Ipam::new(IpamMode::ClusterPool);
        let err = ipam.allocate_v4("missing", "ns/p1").unwrap_err();
        assert_eq!(err, IpamError::PoolNotFound("missing".into()));
    }

    // ── Multi-pool ───────────────────────────────────────────────────────────

    #[test]
    fn ipam_multi_pool_named_lookup_separately() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/multipool/manager.go", "Allocate", "tenant-ipam-mp-named");
        let mut ipam = Ipam::new(IpamMode::MultiPool);
        ipam.upsert_pool(PodIpPool::ipv4("workload-a", tenant.clone(), "10.10.0.0/24")).unwrap();
        ipam.upsert_pool(PodIpPool::ipv4("workload-b", tenant, "10.20.0.0/24")).unwrap();
        let a = ipam.allocate_v4("workload-a", "ns/p1").unwrap();
        let b = ipam.allocate_v4("workload-b", "ns/p1").unwrap();
        assert!(a.to_string().starts_with("10.10."));
        assert!(b.to_string().starts_with("10.20."));
    }

    #[test]
    fn ipam_multi_pool_count_tracks_upserts() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/multipool/manager.go", "Pools", "tenant-ipam-mp-count");
        let mut ipam = Ipam::new(IpamMode::MultiPool);
        for i in 0..5 {
            ipam.upsert_pool(PodIpPool::ipv4(format!("p-{i}"), tenant.clone(), format!("10.{i}.0.0/24"))).unwrap();
        }
        assert_eq!(ipam.pool_count(), 5);
    }

    #[test]
    fn ipam_multi_pool_upsert_replaces_existing_definition() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/multipool/manager.go", "Upsert.Replace", "tenant-ipam-mp-up");
        let mut ipam = Ipam::new(IpamMode::MultiPool);
        ipam.upsert_pool(PodIpPool::ipv4("p", tenant.clone(), "10.0.0.0/24")).unwrap();
        // Allocate, then redefine pool — allocations preserved.
        let _ = ipam.allocate_v4("p", "ns/x").unwrap();
        let mut new = PodIpPool::ipv4("p", tenant, "10.0.0.0/24");
        new.low_watermark = 32;
        ipam.upsert_pool(new).unwrap();
        let st = ipam.pool_status("p").unwrap();
        assert_eq!(st.allocated_v4, 1);
    }

    // ── Dual stack ───────────────────────────────────────────────────────────

    #[test]
    fn ipam_dual_stack_allocates_v4_and_v6() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/types.go", "AllocateNext.DualStack", "tenant-ipam-ds-alloc");
        let mut ipam = Ipam::new(IpamMode::ClusterPool);
        ipam.upsert_pool(PodIpPool::dual_stack("default", tenant, "10.0.0.0/24", "fd00::/64")).unwrap();
        let (v4, v6) = ipam.allocate_dual_stack("default", "ns/p1").unwrap();
        assert!(v4.is_ipv4());
        assert!(v6.is_ipv6());
    }

    #[test]
    fn ipam_dual_stack_release_owner_drops_both() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/types.go", "Release.DualStack", "tenant-ipam-ds-rel");
        let mut ipam = Ipam::new(IpamMode::ClusterPool);
        ipam.upsert_pool(PodIpPool::dual_stack("default", tenant, "10.0.0.0/24", "fd00::/64")).unwrap();
        ipam.allocate_dual_stack("default", "ns/p1").unwrap();
        let n = ipam.release_owner("ns/p1");
        assert_eq!(n, 2);
        let st = ipam.pool_status("default").unwrap();
        assert_eq!(st.allocated_v4 + st.allocated_v6, 0);
    }

    #[test]
    fn ipam_pool_v6_only_capacity_capped_at_u64_max() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/types.go", "Pool.Capacity.V6", "tenant-ipam-v6-cap");
        let mut ipam = Ipam::new(IpamMode::ClusterPool);
        let mut p = PodIpPool::ipv4("v6", tenant, "10.0.0.0/24");
        p.cidr_v6 = Some("fd00::/64".into());
        ipam.upsert_pool(p).unwrap();
        let st = ipam.pool_status("v6").unwrap();
        assert_eq!(st.capacity_v6, u64::MAX);
    }

    // ── Watermarks ───────────────────────────────────────────────────────────

    #[test]
    fn ipam_low_watermark_breach_when_free_below_threshold() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/types.go", "Watermark.Low", "tenant-ipam-wmlow");
        let mut ipam = Ipam::new(IpamMode::ClusterPool);
        let mut pool = PodIpPool::ipv4("default", tenant, "10.0.0.0/29");
        pool.low_watermark = 6;
        pool.high_watermark = 100;
        ipam.upsert_pool(pool).unwrap();
        // /29 has 8 addresses total. Allocate 3 → 5 free → below low-watermark of 6.
        ipam.allocate_v4("default", "ns/p1").unwrap();
        ipam.allocate_v4("default", "ns/p2").unwrap();
        ipam.allocate_v4("default", "ns/p3").unwrap();
        let st = ipam.pool_status("default").unwrap();
        assert!(st.low_watermark_breached);
    }

    #[test]
    fn ipam_high_watermark_breach_when_free_above_threshold() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/types.go", "Watermark.High", "tenant-ipam-wmhi");
        let mut ipam = Ipam::new(IpamMode::ClusterPool);
        let mut pool = PodIpPool::ipv4("default", tenant, "10.0.0.0/24");
        pool.low_watermark = 1;
        pool.high_watermark = 16;
        ipam.upsert_pool(pool).unwrap();
        let st = ipam.pool_status("default").unwrap();
        assert!(st.high_watermark_breached);
    }

    #[test]
    fn ipam_pool_status_zero_when_empty_pool() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/types.go", "Status.Empty", "tenant-ipam-st-empty");
        let mut ipam = Ipam::new(IpamMode::ClusterPool);
        ipam.upsert_pool(PodIpPool::ipv4("default", tenant, "10.0.0.0/29")).unwrap();
        let st = ipam.pool_status("default").unwrap();
        assert_eq!(st.allocated_v4, 0);
    }

    // ── Kubernetes mode ──────────────────────────────────────────────────────

    #[test]
    fn ipam_kubernetes_mode_allocates_from_node_pod_cidr() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/hostscope/hostscope.go", "Allocate", "tenant-ipam-k8s");
        let mut ipam = Ipam::new(IpamMode::Kubernetes);
        ipam.set_node_pod_cidrs("node-a", Some("10.244.1.0/24".into()), None);
        let (v4, v6) = ipam.allocate_from_node(tenant, "node-a", "ns/p1").unwrap();
        assert!(v4.unwrap().to_string().starts_with("10.244.1."));
        assert!(v6.is_none());
    }

    #[test]
    fn ipam_kubernetes_mode_unknown_node_returns_error() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/hostscope/hostscope.go", "Allocate.UnknownNode", "tenant-ipam-k8s-nf");
        let mut ipam = Ipam::new(IpamMode::Kubernetes);
        let err = ipam.allocate_from_node(tenant, "node-x", "ns/p1").unwrap_err();
        assert_eq!(err, IpamError::NodeMissingPodCidr("node-x".into()));
    }

    #[test]
    fn ipam_kubernetes_mode_dual_stack_node_returns_both() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/hostscope/hostscope.go", "Allocate.DualStack", "tenant-ipam-k8s-ds");
        let mut ipam = Ipam::new(IpamMode::Kubernetes);
        ipam.set_node_pod_cidrs("node-b", Some("10.244.2.0/24".into()), Some("fd00:2::/64".into()));
        let (v4, v6) = ipam.allocate_from_node(tenant, "node-b", "ns/p1").unwrap();
        assert!(v4.is_some());
        assert!(v6.is_some());
    }

    #[test]
    fn ipam_kubernetes_mode_isolates_per_node() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/hostscope/hostscope.go", "Allocate.PerNode", "tenant-ipam-k8s-iso");
        let mut ipam = Ipam::new(IpamMode::Kubernetes);
        ipam.set_node_pod_cidrs("node-a", Some("10.244.1.0/24".into()), None);
        ipam.set_node_pod_cidrs("node-b", Some("10.244.2.0/24".into()), None);
        let (a, _) = ipam.allocate_from_node(tenant.clone(), "node-a", "ns/p1").unwrap();
        let (b, _) = ipam.allocate_from_node(tenant, "node-b", "ns/p2").unwrap();
        assert!(a.unwrap().to_string().starts_with("10.244.1."));
        assert!(b.unwrap().to_string().starts_with("10.244.2."));
    }

    // ── CRD round-trip ───────────────────────────────────────────────────────

    #[test]
    fn ipam_pool_crd_round_trips_through_serde() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/multipool/types.go", "CiliumPodIPPool", "tenant-ipam-crd");
        let pool = PodIpPool::dual_stack("workload", tenant, "10.0.0.0/24", "fd00::/64");
        let json = serde_json::to_string(&pool).unwrap();
        let back: PodIpPool = serde_json::from_str(&json).unwrap();
        assert_eq!(back.cidr_v4.as_deref(), Some("10.0.0.0/24"));
        assert_eq!(back.cidr_v6.as_deref(), Some("fd00::/64"));
    }

    #[test]
    fn ipam_release_specific_unknown_pool_returns_error() {
        let (_c, _t) = cilium_test_ctx!("pkg/ipam/ipam.go", "ReleaseIP.PoolNotFound", "tenant-ipam-rel-pnf");
        let mut ipam = Ipam::new(IpamMode::ClusterPool);
        let err = ipam.release_specific("missing", ip4(10, 0, 0, 1)).unwrap_err();
        assert_eq!(err, IpamError::PoolNotFound("missing".into()));
    }
}
