// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cluster-pool IPAM — a port of cilium's `pkg/ipam` cluster-pool mode.
//!
//! Two layers, matching cilium's split between the operator and the agent:
//!   * [`PodCidrAllocator`] is the operator-side allocator
//!     (`pkg/ipam/allocator/clusterpool`): it carves per-node PodCIDRs out
//!     of the cluster CIDR(s) at a fixed node mask, lowest-free first, and
//!     reclaims them on node deletion.
//!   * [`NodePool`] is the agent-side allocator (`pkg/ipam` host scope): it
//!     hands out individual pod IPs from the node's CIDR, tracking the
//!     owning endpoint so a GC sweep can release everything an endpoint held.
//!
//! [`ClusterPoolState`] ties them together for the HTTP surface.

use std::collections::{BTreeSet, HashMap};
use std::net::Ipv4Addr;

use ipnet::Ipv4Net;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum IpamError {
    #[error("cluster CIDR pool exhausted — no free node CIDR")]
    ClusterExhausted,
    #[error("node CIDR exhausted — no free pod IP")]
    NodeExhausted,
    #[error("no IPAM pool for node {0}")]
    UnknownNode(String),
    #[error("IPAM is not configured with any cluster CIDR")]
    NotConfigured,
}

/// One cluster CIDR plus the prefix length carved per node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterCidr {
    pub net: Ipv4Net,
    pub node_mask: u8,
}

impl ClusterCidr {
    pub fn new(net: Ipv4Net, node_mask: u8) -> Self {
        ClusterCidr { net, node_mask }
    }
}

/// Operator-side allocator: carves node PodCIDRs from cluster CIDRs.
#[derive(Debug, Default)]
pub struct PodCidrAllocator {
    cidrs: Vec<ClusterCidr>,
    allocated: BTreeSet<Ipv4Net>,
}

impl PodCidrAllocator {
    pub fn new(cidrs: Vec<ClusterCidr>) -> Self {
        PodCidrAllocator {
            cidrs,
            allocated: BTreeSet::new(),
        }
    }

    /// Carve the lowest free node CIDR across all configured cluster CIDRs.
    pub fn allocate_node_cidr(&mut self) -> Result<Ipv4Net, IpamError> {
        if self.cidrs.is_empty() {
            return Err(IpamError::NotConfigured);
        }
        for c in &self.cidrs {
            // `subnets` yields every node-mask subnet of the cluster CIDR.
            if let Ok(subnets) = c.net.subnets(c.node_mask) {
                for sub in subnets {
                    if !self.allocated.contains(&sub) {
                        self.allocated.insert(sub);
                        return Ok(sub);
                    }
                }
            }
        }
        Err(IpamError::ClusterExhausted)
    }

    pub fn release_node_cidr(&mut self, net: Ipv4Net) -> bool {
        self.allocated.remove(&net)
    }

    pub fn is_allocated(&self, net: &Ipv4Net) -> bool {
        self.allocated.contains(net)
    }
}

/// Agent-side allocator: hands out pod IPs from one node CIDR.
#[derive(Debug)]
pub struct NodePool {
    cidr: Ipv4Net,
    allocated: BTreeSet<Ipv4Addr>,
    owners: HashMap<Ipv4Addr, String>,
}

impl NodePool {
    pub fn new(cidr: Ipv4Net) -> Self {
        NodePool {
            cidr,
            allocated: BTreeSet::new(),
            owners: HashMap::new(),
        }
    }

    pub fn cidr(&self) -> Ipv4Net {
        self.cidr
    }

    /// Allocate the lowest free usable host address to `owner`.
    pub fn allocate(&mut self, owner: &str) -> Result<Ipv4Addr, IpamError> {
        // `hosts()` excludes the network and broadcast addresses for
        // prefixes shorter than /31, matching cilium's usable-host set.
        for host in self.cidr.hosts() {
            if !self.allocated.contains(&host) {
                self.allocated.insert(host);
                self.owners.insert(host, owner.to_string());
                return Ok(host);
            }
        }
        Err(IpamError::NodeExhausted)
    }

    /// Release a single IP. Returns false if it was not allocated.
    pub fn release(&mut self, ip: Ipv4Addr) -> bool {
        self.owners.remove(&ip);
        self.allocated.remove(&ip)
    }

    /// GC sweep: release every IP held by `owner`, returning them sorted.
    pub fn release_owner(&mut self, owner: &str) -> Vec<Ipv4Addr> {
        let mut to_free: Vec<Ipv4Addr> = self
            .owners
            .iter()
            .filter(|(_, o)| o.as_str() == owner)
            .map(|(ip, _)| *ip)
            .collect();
        for ip in &to_free {
            self.allocated.remove(ip);
            self.owners.remove(ip);
        }
        to_free.sort();
        to_free
    }

    pub fn allocated_count(&self) -> usize {
        self.allocated.len()
    }
}

/// Combined cluster-pool IPAM state for the HTTP surface.
#[derive(Debug, Default)]
pub struct ClusterPoolState {
    allocator: Option<PodCidrAllocator>,
    nodes: HashMap<String, NodePool>,
}

impl ClusterPoolState {
    /// Configure (or reconfigure) the cluster CIDRs.
    pub fn configure(&mut self, cidrs: Vec<ClusterCidr>) {
        self.allocator = Some(PodCidrAllocator::new(cidrs));
    }

    /// Ensure `node` has a carved PodCIDR; idempotent.
    pub fn ensure_node(&mut self, node: &str) -> Result<Ipv4Net, IpamError> {
        if let Some(pool) = self.nodes.get(node) {
            return Ok(pool.cidr());
        }
        let alloc = self.allocator.as_mut().ok_or(IpamError::NotConfigured)?;
        let cidr = alloc.allocate_node_cidr()?;
        self.nodes.insert(node.to_string(), NodePool::new(cidr));
        Ok(cidr)
    }

    pub fn allocate_ip(&mut self, node: &str, owner: &str) -> Result<Ipv4Addr, IpamError> {
        self.nodes
            .get_mut(node)
            .ok_or_else(|| IpamError::UnknownNode(node.to_string()))?
            .allocate(owner)
    }

    pub fn release_ip(&mut self, node: &str, ip: Ipv4Addr) -> bool {
        self.nodes
            .get_mut(node)
            .map(|p| p.release(ip))
            .unwrap_or(false)
    }

    pub fn node_cidr(&self, node: &str) -> Option<Ipv4Net> {
        self.nodes.get(node).map(|p| p.cidr())
    }

    pub fn nodes(&self) -> Vec<String> {
        let mut v: Vec<String> = self.nodes.keys().cloned().collect();
        v.sort();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn cidr(s: &str) -> ipnet::Ipv4Net {
        s.parse().unwrap()
    }

    #[test]
    fn carves_node_cidrs_lowest_first() {
        let mut a = PodCidrAllocator::new(vec![ClusterCidr::new(cidr("10.0.0.0/16"), 24)]);
        assert_eq!(a.allocate_node_cidr().unwrap(), cidr("10.0.0.0/24"));
        assert_eq!(a.allocate_node_cidr().unwrap(), cidr("10.0.1.0/24"));
        assert_eq!(a.allocate_node_cidr().unwrap(), cidr("10.0.2.0/24"));
    }

    #[test]
    fn released_node_cidr_is_reused() {
        let mut a = PodCidrAllocator::new(vec![ClusterCidr::new(cidr("10.0.0.0/16"), 24)]);
        let first = a.allocate_node_cidr().unwrap();
        let _second = a.allocate_node_cidr().unwrap();
        a.release_node_cidr(first);
        // Lowest free is the released one again.
        assert_eq!(a.allocate_node_cidr().unwrap(), cidr("10.0.0.0/24"));
    }

    #[test]
    fn cluster_cidr_exhaustion_errors() {
        // /30 split into /31 → exactly 2 node CIDRs.
        let mut a = PodCidrAllocator::new(vec![ClusterCidr::new(cidr("10.0.0.0/30"), 31)]);
        assert!(a.allocate_node_cidr().is_ok());
        assert!(a.allocate_node_cidr().is_ok());
        assert!(matches!(
            a.allocate_node_cidr(),
            Err(IpamError::ClusterExhausted)
        ));
    }

    #[test]
    fn node_pool_allocates_usable_hosts() {
        let mut p = NodePool::new(cidr("10.0.0.0/24"));
        // First usable host is .1 (network .0 reserved).
        assert_eq!(p.allocate("pod-a").unwrap(), Ipv4Addr::new(10, 0, 0, 1));
        assert_eq!(p.allocate("pod-b").unwrap(), Ipv4Addr::new(10, 0, 0, 2));
        assert_eq!(p.allocated_count(), 2);
    }

    #[test]
    fn release_frees_lowest_for_reuse() {
        let mut p = NodePool::new(cidr("10.0.0.0/24"));
        let a = p.allocate("pod-a").unwrap();
        let _b = p.allocate("pod-b").unwrap();
        assert!(p.release(a));
        // freed .1 is handed back out first
        assert_eq!(p.allocate("pod-c").unwrap(), Ipv4Addr::new(10, 0, 0, 1));
        // releasing an unallocated address is a no-op
        assert!(!p.release(Ipv4Addr::new(10, 0, 0, 200)));
    }

    #[test]
    fn gc_releases_all_ips_for_owner() {
        let mut p = NodePool::new(cidr("10.0.0.0/24"));
        p.allocate("pod-a").unwrap();
        p.allocate("pod-a").unwrap();
        p.allocate("pod-b").unwrap();
        let freed = p.release_owner("pod-a");
        assert_eq!(freed.len(), 2);
        assert_eq!(p.allocated_count(), 1);
    }

    #[test]
    fn node_pool_exhaustion_errors() {
        // /30 usable hosts = .1, .2 (network .0, broadcast .3 reserved).
        let mut p = NodePool::new(cidr("10.0.0.0/30"));
        assert!(p.allocate("a").is_ok());
        assert!(p.allocate("b").is_ok());
        assert!(matches!(p.allocate("c"), Err(IpamError::NodeExhausted)));
    }

    #[test]
    fn cluster_pool_state_end_to_end() {
        let mut st = ClusterPoolState::default();
        st.configure(vec![ClusterCidr::new(cidr("10.244.0.0/16"), 24)]);
        // ensure_node carves a /24 for the node and records it.
        let n1 = st.ensure_node("node-1").unwrap();
        assert_eq!(n1, cidr("10.244.0.0/24"));
        let n2 = st.ensure_node("node-2").unwrap();
        assert_eq!(n2, cidr("10.244.1.0/24"));
        // idempotent: ensuring again returns the same CIDR.
        assert_eq!(st.ensure_node("node-1").unwrap(), n1);

        let ip = st.allocate_ip("node-1", "pod-x").unwrap();
        assert!(cidr("10.244.0.0/24").contains(&ip));
        assert!(st.release_ip("node-1", ip));

        // allocating on an unknown node errors.
        assert!(st.allocate_ip("ghost", "p").is_err());
    }
}
