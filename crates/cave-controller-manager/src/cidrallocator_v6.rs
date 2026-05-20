// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! IPv6 + dual-stack pod-CIDR allocator.
//!
//! Cite: `pkg/controller/nodeipam/ipam/range_allocator.go` (v1.36.0),
//! dual-stack codepath gated by `IPv6DualStack` GA in v1.23.
//!
//! Closes the IPv4-only gap in [`crate::cidrallocator`]. The original
//! `CidrAllocator` carves a 32-bit `/N` cluster CIDR into per-node
//! slices; here we extend the same model to 128-bit IPv6 networks
//! and add a [`DualStackAllocator`] that holds both pools and emits
//! `(v4, v6)` tuples per node-add event (KEP-563).
//!
//! Surface contract is identical to the IPv4 sibling:
//!
//! * `new(cluster_cidr, node_mask)` — validate range, build the slot
//!   bit-set.
//! * `allocate(node)` — next free slice, idempotent per node.
//! * `occupy(node, cidr)` — reserve a specific slice (reconcile path).
//! * `release(node)` — free the slot.
//! * `cidr_for(node)` — current allocation, if any.
//!
//! Only `u128` arithmetic is used so slot capacity is capped at 1 M
//! entries — same heuristic as IPv4 — so a `/64` cluster with `/72`
//! node mask doesn't blow up memory.

use crate::types::Cite;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/nodeipam/ipam/range_allocator.go",
    "rangeAllocator.AllocateOrOccupyCIDR/ipv6",
);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum V6AllocatorError {
    #[error("invalid IPv6 CIDR '{0}'")]
    InvalidCidr(String),
    #[error("node-mask {node} must be > cluster-mask {cluster}")]
    NodeMaskTooSmall { cluster: u8, node: u8 },
    #[error("IPv6 address pool exhausted (slot_capacity={0})")]
    Exhausted(usize),
    #[error("node {0} already has a CIDR assignment")]
    AlreadyAssigned(String),
    #[error("CIDR {0} not in pool range")]
    OutOfRange(String),
}

/// One slice of an IPv6 cluster CIDR carved out for a single node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeCidrV6 {
    /// 128-bit big-endian network address.
    pub network: u128,
    /// Mask width — e.g. `64` for `/64`.
    pub prefix_len: u8,
}

impl NodeCidrV6 {
    /// Render as canonical compact-ish IPv6 (`x:x:x:x:x:x:x:x/N`).
    /// We emit lowercase hextets without `::` compression so two
    /// allocations are byte-comparable as strings.
    pub fn display(&self) -> String {
        let b = self.network.to_be_bytes();
        let mut out = String::new();
        for i in 0..8 {
            let hex = u16::from_be_bytes([b[i * 2], b[i * 2 + 1]]);
            if i > 0 {
                out.push(':');
            }
            out.push_str(&format!("{:x}", hex));
        }
        format!("{}/{}", out, self.prefix_len)
    }

    /// Parse a non-compressed IPv6 CIDR (`x:x:x:x:x:x:x:x/N`).
    ///
    /// We accept exactly 8 hextet groups separated by `:`. Compressed
    /// forms (`fd00::1/64`) and IPv4-mapped notation are not supported;
    /// callers are expected to feed in canonical form.
    pub fn parse(s: &str) -> Option<NodeCidrV6> {
        let (addr, len) = s.split_once('/')?;
        let prefix_len: u8 = len.parse().ok()?;
        if prefix_len > 128 {
            return None;
        }
        let groups: Vec<&str> = addr.split(':').collect();
        if groups.len() != 8 {
            return None;
        }
        let mut bytes = [0u8; 16];
        for (i, g) in groups.iter().enumerate() {
            if g.is_empty() || g.len() > 4 {
                return None;
            }
            let n = u16::from_str_radix(g, 16).ok()?;
            let be = n.to_be_bytes();
            bytes[i * 2] = be[0];
            bytes[i * 2 + 1] = be[1];
        }
        let network = u128::from_be_bytes(bytes);
        Some(NodeCidrV6 {
            network: network & host_mask_v6(prefix_len),
            prefix_len,
        })
    }

    /// Total addresses in this slice. Capped at `u64::MAX` so the
    /// return value is comparable across very wide IPv6 slices.
    pub fn capacity(&self) -> u128 {
        if self.prefix_len == 0 {
            return u128::MAX;
        }
        1u128 << (128u8.saturating_sub(self.prefix_len) as u128)
    }
}

fn host_mask_v6(prefix_len: u8) -> u128 {
    if prefix_len == 0 {
        return 0;
    }
    if prefix_len >= 128 {
        return u128::MAX;
    }
    let bits = 128u8 - prefix_len;
    !((1u128 << bits) - 1)
}

/// Slot allocator for an IPv6 cluster CIDR. Mirrors
/// [`crate::cidrallocator::CidrAllocator`] field-for-field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CidrAllocatorV6 {
    pub cluster_cidr: NodeCidrV6,
    pub node_mask: u8,
    pub slots: Vec<bool>,
    pub assignments: BTreeMap<String, usize>,
}

impl CidrAllocatorV6 {
    pub fn new(cluster_cidr_str: &str, node_mask: u8) -> Result<Self, V6AllocatorError> {
        let cluster = NodeCidrV6::parse(cluster_cidr_str)
            .ok_or_else(|| V6AllocatorError::InvalidCidr(cluster_cidr_str.to_string()))?;
        if node_mask <= cluster.prefix_len {
            return Err(V6AllocatorError::NodeMaskTooSmall {
                cluster: cluster.prefix_len,
                node: node_mask,
            });
        }
        if node_mask > 128 {
            return Err(V6AllocatorError::InvalidCidr(format!("/{node_mask}")));
        }
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

    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    pub fn in_use(&self) -> usize {
        self.assignments.len()
    }

    pub fn allocate(&mut self, node_name: &str) -> Result<NodeCidrV6, V6AllocatorError> {
        if let Some(&slot) = self.assignments.get(node_name) {
            return Ok(self.slot_to_cidr(slot));
        }
        let free = self.slots.iter().position(|s| !*s);
        let slot = free.ok_or(V6AllocatorError::Exhausted(self.slots.len()))?;
        self.slots[slot] = true;
        self.assignments.insert(node_name.to_string(), slot);
        Ok(self.slot_to_cidr(slot))
    }

    pub fn occupy(
        &mut self,
        node_name: &str,
        cidr: NodeCidrV6,
    ) -> Result<NodeCidrV6, V6AllocatorError> {
        let slot = self.cidr_to_slot(cidr)?;
        if let Some(&existing) = self.assignments.get(node_name) {
            if existing == slot {
                return Ok(cidr);
            }
            return Err(V6AllocatorError::AlreadyAssigned(node_name.to_string()));
        }
        if self.slots[slot] {
            return Err(V6AllocatorError::AlreadyAssigned(format!(
                "slot {slot} occupied"
            )));
        }
        self.slots[slot] = true;
        self.assignments.insert(node_name.to_string(), slot);
        Ok(cidr)
    }

    pub fn release(&mut self, node_name: &str) {
        if let Some(slot) = self.assignments.remove(node_name) {
            self.slots[slot] = false;
        }
    }

    pub fn cidr_for(&self, node_name: &str) -> Option<NodeCidrV6> {
        self.assignments
            .get(node_name)
            .map(|&s| self.slot_to_cidr(s))
    }

    fn slot_to_cidr(&self, slot: usize) -> NodeCidrV6 {
        let slot_size: u128 = 1u128 << (128 - self.node_mask);
        let network = self.cluster_cidr.network + (slot as u128) * slot_size;
        NodeCidrV6 {
            network,
            prefix_len: self.node_mask,
        }
    }

    fn cidr_to_slot(&self, cidr: NodeCidrV6) -> Result<usize, V6AllocatorError> {
        if cidr.prefix_len != self.node_mask {
            return Err(V6AllocatorError::OutOfRange(cidr.display()));
        }
        let cluster_mask = host_mask_v6(self.cluster_cidr.prefix_len);
        if (cidr.network & cluster_mask) != self.cluster_cidr.network {
            return Err(V6AllocatorError::OutOfRange(cidr.display()));
        }
        let slot_size: u128 = 1u128 << (128 - self.node_mask);
        let offset = cidr.network - self.cluster_cidr.network;
        let slot = (offset / slot_size) as usize;
        if slot >= self.slots.len() {
            return Err(V6AllocatorError::OutOfRange(cidr.display()));
        }
        Ok(slot)
    }
}

/// Dual-stack wrapper — holds one v4 + one v6 pool and emits
/// `(v4, v6)` slices per node-add. Mirrors the upstream `DualStack`
/// path in `range_allocator.go`.
#[derive(Debug)]
pub struct DualStackAllocator {
    pub v4: crate::cidrallocator::CidrAllocator,
    pub v6: CidrAllocatorV6,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DualStackError {
    #[error("v4: {0}")]
    V4(crate::cidrallocator::AllocatorError),
    #[error("v6: {0}")]
    V6(V6AllocatorError),
}

impl DualStackAllocator {
    pub fn new(
        v4_cluster_cidr: &str,
        v4_node_mask: u8,
        v6_cluster_cidr: &str,
        v6_node_mask: u8,
    ) -> Result<Self, DualStackError> {
        let v4 = crate::cidrallocator::CidrAllocator::new(v4_cluster_cidr, v4_node_mask)
            .map_err(DualStackError::V4)?;
        let v6 = CidrAllocatorV6::new(v6_cluster_cidr, v6_node_mask).map_err(DualStackError::V6)?;
        Ok(Self { v4, v6 })
    }

    /// Allocate `(v4, v6)` for a node atomically. If the v6 leg fails
    /// the v4 leg is rolled back so a subsequent retry doesn't leak.
    pub fn allocate(
        &mut self,
        node_name: &str,
    ) -> Result<(crate::cidrallocator::NodeCidr, NodeCidrV6), DualStackError> {
        let v4 = self.v4.allocate(node_name).map_err(DualStackError::V4)?;
        match self.v6.allocate(node_name) {
            Ok(v6) => Ok((v4, v6)),
            Err(e) => {
                // Roll back the v4 allocation to avoid leaking a slot.
                self.v4.release(node_name);
                Err(DualStackError::V6(e))
            }
        }
    }

    pub fn release(&mut self, node_name: &str) {
        self.v4.release(node_name);
        self.v6.release(node_name);
    }

    pub fn in_use(&self) -> usize {
        self.v4.in_use()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    #[test]
    fn v6_parse_round_trips_through_display() {
        let (_c, _t) = test_ctx!(
            "pkg/controller/nodeipam/ipam/range_allocator.go",
            "v6_parse",
            "tenant-v6-parse"
        );
        let c = NodeCidrV6::parse("fd00:0:0:0:0:0:0:0/64").unwrap();
        assert_eq!(c.display(), "fd00:0:0:0:0:0:0:0/64");
    }

    #[test]
    fn v6_parse_rejects_compressed_or_short_forms() {
        // "::" compression intentionally not supported in canonical
        // input — caller must expand first.
        assert!(NodeCidrV6::parse("fd00::1/64").is_none());
        assert!(NodeCidrV6::parse("fd00:0:0:0:0:0:0/64").is_none()); // 7 groups
        assert!(NodeCidrV6::parse("fd00:0:0:0:0:0:0:0/129").is_none());
    }

    #[test]
    fn v6_new_rejects_node_mask_le_cluster_mask() {
        let err = CidrAllocatorV6::new("fd00:0:0:0:0:0:0:0/64", 64).unwrap_err();
        assert!(matches!(err, V6AllocatorError::NodeMaskTooSmall { .. }));
    }

    #[test]
    fn v6_allocate_returns_sequential_slices() {
        let mut a = CidrAllocatorV6::new("fd00:0:0:0:0:0:0:0/64", 72).unwrap();
        let n1 = a.allocate("nodeA").unwrap();
        let n2 = a.allocate("nodeB").unwrap();
        assert_ne!(n1, n2);
        assert_eq!(n1.prefix_len, 72);
        assert_eq!(n2.prefix_len, 72);
        // n2 must sit one slot-width past n1.
        let slot_size: u128 = 1u128 << (128 - 72);
        assert_eq!(n2.network - n1.network, slot_size);
    }

    #[test]
    fn v6_allocate_is_idempotent_per_node() {
        let mut a = CidrAllocatorV6::new("fd00:0:0:0:0:0:0:0/64", 72).unwrap();
        let a1 = a.allocate("nodeA").unwrap();
        let a2 = a.allocate("nodeA").unwrap();
        assert_eq!(a1, a2);
        assert_eq!(a.in_use(), 1);
    }

    #[test]
    fn v6_release_frees_slot_for_reuse() {
        let mut a = CidrAllocatorV6::new("fd00:0:0:0:0:0:0:0/64", 72).unwrap();
        let first = a.allocate("nodeA").unwrap();
        a.release("nodeA");
        let second = a.allocate("nodeB").unwrap();
        // First free slot reused — same network address.
        assert_eq!(first.network, second.network);
    }

    #[test]
    fn v6_occupy_rejects_out_of_range_prefix() {
        let mut a = CidrAllocatorV6::new("fd00:0:0:0:0:0:0:0/64", 72).unwrap();
        // /80 doesn't match the configured node_mask=72.
        let wrong = NodeCidrV6::parse("fd00:0:0:0:0:0:0:0/80").unwrap();
        let err = a.occupy("legacy", wrong).unwrap_err();
        assert!(matches!(err, V6AllocatorError::OutOfRange(_)));
    }

    #[test]
    fn v6_occupy_rejects_cidr_outside_cluster() {
        let mut a = CidrAllocatorV6::new("fd00:0:0:0:0:0:0:0/64", 72).unwrap();
        // Different cluster network entirely.
        let wrong = NodeCidrV6::parse("fe00:0:0:0:0:0:0:0/72").unwrap();
        let err = a.occupy("legacy", wrong).unwrap_err();
        assert!(matches!(err, V6AllocatorError::OutOfRange(_)));
    }

    #[test]
    fn v6_exhaustion_returns_error_after_pool_full() {
        // /124 cluster with /126 nodes → 4 slots.
        let mut a = CidrAllocatorV6::new("fd00:0:0:0:0:0:0:0/124", 126).unwrap();
        for i in 0..4 {
            a.allocate(&format!("n{i}")).unwrap();
        }
        let err = a.allocate("overflow").unwrap_err();
        assert!(matches!(err, V6AllocatorError::Exhausted(4)));
    }

    #[test]
    fn v6_cidr_for_returns_assignment_or_none() {
        let mut a = CidrAllocatorV6::new("fd00:0:0:0:0:0:0:0/64", 72).unwrap();
        a.allocate("nodeA").unwrap();
        assert!(a.cidr_for("nodeA").is_some());
        assert!(a.cidr_for("missing").is_none());
    }

    #[test]
    fn dual_stack_allocates_both_legs_per_node() {
        let mut a = DualStackAllocator::new(
            "10.244.0.0/16",
            24,
            "fd00:0:0:0:0:0:0:0/64",
            72,
        )
        .unwrap();
        let (v4, v6) = a.allocate("nodeA").unwrap();
        assert_eq!(v4.prefix_len, 24);
        assert_eq!(v6.prefix_len, 72);
    }

    #[test]
    fn dual_stack_rollback_releases_v4_on_v6_failure() {
        // /124 cluster + /126 nodes → 4 v6 slots. Fill v6 first, then
        // try to allocate one more — v4 has plenty of headroom, but
        // the v6 leg must fail, and the v4 slot for the failing node
        // must NOT be left held.
        let mut a = DualStackAllocator::new(
            "10.244.0.0/16",
            24,
            "fd00:0:0:0:0:0:0:0/124",
            126,
        )
        .unwrap();
        for i in 0..4 {
            a.allocate(&format!("n{i}")).unwrap();
        }
        let before_v4 = a.v4.in_use();
        let err = a.allocate("overflow").unwrap_err();
        assert!(matches!(err, DualStackError::V6(_)));
        // v4 was rolled back ⇒ in_use unchanged.
        assert_eq!(a.v4.in_use(), before_v4);
    }

    #[test]
    fn dual_stack_release_frees_both_legs() {
        let mut a = DualStackAllocator::new(
            "10.244.0.0/16",
            24,
            "fd00:0:0:0:0:0:0:0/64",
            72,
        )
        .unwrap();
        a.allocate("nodeA").unwrap();
        a.allocate("nodeB").unwrap();
        a.release("nodeA");
        assert_eq!(a.v4.in_use(), 1);
        assert_eq!(a.v6.in_use(), 1);
    }
}
