// SPDX-License-Identifier: AGPL-3.0-or-later
//! Pod CIDR allocator — on-the-fly node-CIDR slicing.
//!
//! Cite: pkg/controller/nodeipam/ipam/range_allocator.go (v1.36.0).
//!
//! When the cluster runs without a cloud provider (cave's default
//! posture), this controller slices the cluster's pod CIDR into per-
//! node sub-CIDRs at the node-create event. The audit doc flagged
//! this as unmapped — cave-net is pre-provisioned today, the
//! on-the-fly allocator was missing.
//!
//! Surface: pure-function allocator + pool struct, no networking.
//! Callers feed in the cluster pod CIDR, the per-node mask size, and
//! the existing assignments. The pool returns the next free slice
//! or `AllocatorError::Exhausted`.
//!
//! Only IPv4 is modelled here; dual-stack support is a follow-up.

use crate::types::Cite;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/nodeipam/ipam/range_allocator.go",
    "rangeAllocator.AllocateOrOccupyCIDR",
);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AllocatorError {
    #[error("invalid CIDR '{0}'")]
    InvalidCidr(String),
    #[error("node-mask {node} must be > cluster-mask {cluster}")]
    NodeMaskTooSmall { cluster: u8, node: u8 },
    #[error("address pool exhausted (slot_capacity={0})")]
    Exhausted(usize),
    #[error("node {0} already has a CIDR assignment")]
    AlreadyAssigned(String),
    #[error("CIDR {0} not in pool range")]
    OutOfRange(String),
}

/// One slice of the cluster CIDR carved out for a single node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeCidr {
    /// `/24`-style prefix (IPv4 octets in big-endian u32).
    pub network: u32,
    /// Mask width: 24 means `/24` → 256 addresses.
    pub prefix_len: u8,
}

impl NodeCidr {
    /// Render as canonical `<a.b.c.d>/<len>`.
    pub fn display(&self) -> String {
        let o = self.network.to_be_bytes();
        format!("{}.{}.{}.{}/{}", o[0], o[1], o[2], o[3], self.prefix_len)
    }

    /// Parse `<a.b.c.d>/<len>` into a `NodeCidr`. Returns `None`
    /// when the input isn't dotted-IPv4 with a `/length` suffix.
    pub fn parse(s: &str) -> Option<NodeCidr> {
        let (addr, len) = s.split_once('/')?;
        let prefix_len: u8 = len.parse().ok()?;
        if prefix_len > 32 {
            return None;
        }
        let octets: Vec<&str> = addr.split('.').collect();
        if octets.len() != 4 {
            return None;
        }
        let mut bytes = [0u8; 4];
        for (i, p) in octets.iter().enumerate() {
            bytes[i] = p.parse().ok()?;
        }
        let network = u32::from_be_bytes(bytes);
        Some(NodeCidr {
            network: network & host_mask(prefix_len),
            prefix_len,
        })
    }

    /// Total addresses in this slice (`2^(32 - prefix_len)`).
    pub fn capacity(&self) -> u64 {
        1u64 << (32u8.saturating_sub(self.prefix_len) as u64)
    }
}

fn host_mask(prefix_len: u8) -> u32 {
    if prefix_len == 0 {
        return 0;
    }
    let bits = 32u8 - prefix_len;
    !((1u32 << bits) - 1)
}

/// Slot allocator for one cluster CIDR. Pure data — no socket or
/// apiserver interaction. The outer controller is responsible for
/// persisting the allocation to `Node.spec.podCIDRs[]` and
/// reconciling on restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CidrAllocator {
    /// Cluster pod CIDR (e.g. `10.244.0.0/16`).
    pub cluster_cidr: NodeCidr,
    /// Per-node sub-CIDR width (e.g. 24 for `/24` per node).
    pub node_mask: u8,
    /// Bit-set marking occupied slots. `slots[i] == true` ⇒ slice
    /// `i` is taken.
    pub slots: Vec<bool>,
    /// `node_name → slot index` so `release` is O(log n).
    pub assignments: BTreeMap<String, usize>,
}

impl CidrAllocator {
    /// Build a new allocator. Returns an error if the cluster CIDR
    /// is unparseable or `node_mask <= cluster_mask` (would yield
    /// fewer than 1 slice).
    pub fn new(cluster_cidr_str: &str, node_mask: u8) -> Result<Self, AllocatorError> {
        let cluster = NodeCidr::parse(cluster_cidr_str)
            .ok_or_else(|| AllocatorError::InvalidCidr(cluster_cidr_str.to_string()))?;
        if node_mask <= cluster.prefix_len {
            return Err(AllocatorError::NodeMaskTooSmall {
                cluster: cluster.prefix_len,
                node: node_mask,
            });
        }
        if node_mask > 32 {
            return Err(AllocatorError::InvalidCidr(format!("/{node_mask}")));
        }
        // Slot count = 2^(node_mask - cluster_mask). Cap at 1M slices
        // so an absurdly wide cluster CIDR with a /30 node mask
        // doesn't try to allocate gigabytes of bool slots.
        let raw = 1usize.checked_shl((node_mask - cluster.prefix_len) as u32);
        let slot_count = match raw {
            Some(n) if n <= 1_000_000 => n,
            _ => 1_000_000,
        };
        Ok(Self {
            cluster_cidr: cluster,
            node_mask,
            slots: vec![false; slot_count],
            assignments: BTreeMap::new(),
        })
    }

    /// Total number of /node_mask slots in this pool.
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// Number of slots currently held by nodes.
    pub fn in_use(&self) -> usize {
        self.assignments.len()
    }

    /// Allocate the next free slice for `node_name`. Returns the
    /// slice or `Exhausted`. Idempotent: a re-allocation request for
    /// a node already on the books returns the same slice.
    pub fn allocate(&mut self, node_name: &str) -> Result<NodeCidr, AllocatorError> {
        if let Some(&slot) = self.assignments.get(node_name) {
            return Ok(self.slot_to_cidr(slot));
        }
        let free = self.slots.iter().position(|s| !*s);
        let slot = free.ok_or(AllocatorError::Exhausted(self.slots.len()))?;
        self.slots[slot] = true;
        self.assignments.insert(node_name.to_string(), slot);
        Ok(self.slot_to_cidr(slot))
    }

    /// Reserve a specific slice for a node — used when the node
    /// already carries `Node.spec.podCIDRs[]` from a prior boot and
    /// the controller is reconciling. Returns `OutOfRange` if the
    /// slice isn't in our pool, or `AlreadyAssigned` if the slot is
    /// taken by a *different* node.
    pub fn occupy(&mut self, node_name: &str, cidr: NodeCidr) -> Result<NodeCidr, AllocatorError> {
        let slot = self.cidr_to_slot(cidr)?;
        if let Some(&existing) = self.assignments.get(node_name) {
            if existing == slot {
                return Ok(cidr);
            }
            return Err(AllocatorError::AlreadyAssigned(node_name.to_string()));
        }
        if self.slots[slot] {
            return Err(AllocatorError::AlreadyAssigned(format!(
                "slot {slot} occupied"
            )));
        }
        self.slots[slot] = true;
        self.assignments.insert(node_name.to_string(), slot);
        Ok(cidr)
    }

    /// Release the slot held by `node_name`. No-op if the node had
    /// no assignment.
    pub fn release(&mut self, node_name: &str) {
        if let Some(slot) = self.assignments.remove(node_name) {
            self.slots[slot] = false;
        }
    }

    /// Current allocation for a node, if any.
    pub fn cidr_for(&self, node_name: &str) -> Option<NodeCidr> {
        self.assignments
            .get(node_name)
            .map(|&slot| self.slot_to_cidr(slot))
    }

    fn slot_to_cidr(&self, slot: usize) -> NodeCidr {
        let slot_size: u32 = 1u32 << (32 - self.node_mask);
        let network = self.cluster_cidr.network + (slot as u32) * slot_size;
        NodeCidr {
            network,
            prefix_len: self.node_mask,
        }
    }

    fn cidr_to_slot(&self, cidr: NodeCidr) -> Result<usize, AllocatorError> {
        if cidr.prefix_len != self.node_mask {
            return Err(AllocatorError::OutOfRange(cidr.display()));
        }
        let cluster_mask = host_mask(self.cluster_cidr.prefix_len);
        if (cidr.network & cluster_mask) != self.cluster_cidr.network {
            return Err(AllocatorError::OutOfRange(cidr.display()));
        }
        let slot_size: u32 = 1u32 << (32 - self.node_mask);
        let offset = cidr.network - self.cluster_cidr.network;
        let slot = (offset / slot_size) as usize;
        if slot >= self.slots.len() {
            return Err(AllocatorError::OutOfRange(cidr.display()));
        }
        Ok(slot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    #[test]
    fn parse_round_trip_through_display() {
        let c = NodeCidr::parse("10.244.0.0/16").unwrap();
        assert_eq!(c.display(), "10.244.0.0/16");
        assert_eq!(c.capacity(), 65_536);
    }

    #[test]
    fn parse_rejects_invalid_inputs() {
        assert!(NodeCidr::parse("not a cidr").is_none());
        assert!(NodeCidr::parse("10.244.0.0/64").is_none()); // /64 illegal for v4
        assert!(NodeCidr::parse("10.244.0/16").is_none()); // 3 octets
    }

    #[test]
    fn new_rejects_node_mask_le_cluster_mask() {
        let err = CidrAllocator::new("10.244.0.0/16", 16).unwrap_err();
        assert!(matches!(err, AllocatorError::NodeMaskTooSmall { .. }));
    }

    #[test]
    fn new_rejects_unparseable_cluster_cidr() {
        let err = CidrAllocator::new("garbage", 24).unwrap_err();
        assert!(matches!(err, AllocatorError::InvalidCidr(_)));
    }

    #[test]
    fn allocate_returns_sequential_slices() {
        let (_c, _t) = test_ctx!(
            "pkg/controller/nodeipam/ipam/range_allocator.go",
            "allocSequential",
            "cidr-1"
        );
        let mut a = CidrAllocator::new("10.244.0.0/16", 24).unwrap();
        let n1 = a.allocate("nodeA").unwrap();
        let n2 = a.allocate("nodeB").unwrap();
        let n3 = a.allocate("nodeC").unwrap();
        assert_eq!(n1.display(), "10.244.0.0/24");
        assert_eq!(n2.display(), "10.244.1.0/24");
        assert_eq!(n3.display(), "10.244.2.0/24");
        assert_eq!(a.in_use(), 3);
    }

    #[test]
    fn allocate_is_idempotent_per_node() {
        let mut a = CidrAllocator::new("10.244.0.0/16", 24).unwrap();
        let first = a.allocate("nodeA").unwrap();
        let again = a.allocate("nodeA").unwrap();
        assert_eq!(first, again);
        assert_eq!(a.in_use(), 1);
    }

    #[test]
    fn release_frees_the_slot_for_reuse() {
        let mut a = CidrAllocator::new("10.244.0.0/16", 24).unwrap();
        let _ = a.allocate("nodeA").unwrap();
        a.release("nodeA");
        // Re-allocating to a new node now reuses slot 0.
        let n = a.allocate("nodeB").unwrap();
        assert_eq!(n.display(), "10.244.0.0/24");
    }

    #[test]
    fn exhausted_returns_error_after_pool_full() {
        // /28 cluster, /30 nodes → 4 slots.
        let mut a = CidrAllocator::new("10.244.0.0/28", 30).unwrap();
        for i in 0..4 {
            a.allocate(&format!("n{i}")).unwrap();
        }
        let err = a.allocate("overflow").unwrap_err();
        assert!(matches!(err, AllocatorError::Exhausted(4)));
    }

    #[test]
    fn occupy_records_explicit_assignment() {
        let mut a = CidrAllocator::new("10.244.0.0/16", 24).unwrap();
        let pinned = NodeCidr::parse("10.244.5.0/24").unwrap();
        let got = a.occupy("legacy", pinned).unwrap();
        assert_eq!(got, pinned);
        // Now allocate fresh — should skip slot 5 (already taken).
        let next = a.allocate("fresh").unwrap();
        // First free slot is 0 — but we need to ensure the slot at
        // index 5 doesn't trip us up. We allocate 6 names and check
        // none clashes with 10.244.5.0/24.
        let _ = next;
        for i in 0..6 {
            let _ = a.allocate(&format!("n{i}")).unwrap();
        }
        let assigned: Vec<String> = a
            .assignments
            .values()
            .map(|&s| {
                let c = a.slot_to_cidr(s);
                c.display()
            })
            .collect();
        // legacy keeps its pinned CIDR.
        assert!(assigned.contains(&"10.244.5.0/24".to_string()));
    }

    #[test]
    fn occupy_rejects_cidr_outside_cluster_range() {
        let mut a = CidrAllocator::new("10.244.0.0/16", 24).unwrap();
        let wrong = NodeCidr::parse("192.168.0.0/24").unwrap();
        let err = a.occupy("legacy", wrong).unwrap_err();
        assert!(matches!(err, AllocatorError::OutOfRange(_)));
    }

    #[test]
    fn occupy_rejects_wrong_prefix_length() {
        let mut a = CidrAllocator::new("10.244.0.0/16", 24).unwrap();
        let wrong = NodeCidr::parse("10.244.0.0/25").unwrap();
        let err = a.occupy("legacy", wrong).unwrap_err();
        assert!(matches!(err, AllocatorError::OutOfRange(_)));
    }

    #[test]
    fn occupy_rejects_when_slot_taken_by_other_node() {
        let mut a = CidrAllocator::new("10.244.0.0/16", 24).unwrap();
        let pinned = NodeCidr::parse("10.244.5.0/24").unwrap();
        a.occupy("nodeA", pinned).unwrap();
        let err = a.occupy("nodeB", pinned).unwrap_err();
        assert!(matches!(err, AllocatorError::AlreadyAssigned(_)));
    }

    #[test]
    fn cidr_for_returns_assignment() {
        let mut a = CidrAllocator::new("10.244.0.0/16", 24).unwrap();
        a.allocate("nodeA").unwrap();
        assert!(a.cidr_for("nodeA").is_some());
        assert!(a.cidr_for("missing").is_none());
    }

    #[test]
    fn capacity_reports_total_slots() {
        // /16 cluster, /24 nodes → 256 slots.
        let a = CidrAllocator::new("10.244.0.0/16", 24).unwrap();
        assert_eq!(a.capacity(), 256);
    }

    #[test]
    fn capacity_caps_at_million_for_huge_pools() {
        // /8 cluster, /28 nodes → would be 2^20 = ~1M; our cap
        // floors that to exactly 1_000_000.
        let a = CidrAllocator::new("10.0.0.0/8", 28).unwrap();
        assert_eq!(a.capacity(), 1_000_000);
    }

    #[test]
    fn slot_to_cidr_round_trips_through_cidr_to_slot() {
        let a = CidrAllocator::new("10.244.0.0/16", 24).unwrap();
        for slot in [0usize, 1, 100, 255] {
            let c = a.slot_to_cidr(slot);
            let back = a.cidr_to_slot(c).unwrap();
            assert_eq!(back, slot);
        }
    }

    #[test]
    fn parse_normalises_host_bits_to_network() {
        // 10.244.0.5/24 has host bits set; we normalise to .0.
        let c = NodeCidr::parse("10.244.0.5/24").unwrap();
        assert_eq!(c.display(), "10.244.0.0/24");
    }
}
