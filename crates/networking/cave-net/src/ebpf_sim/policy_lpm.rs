// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Port-range-aware L4 policy LPM lookup.
//!
//! Cite: cilium/pkg/maps/policymap/policymap.go (the policy map is a
//! `BPF_MAP_TYPE_LPM_TRIE`) + cilium/bpf/lib/policy.h `__policy_get`
//! (pinned v1.19.3, Apache-2.0).
//!
//! A NetworkPolicy/CiliumNetworkPolicy L4 rule with an `EndPort` cannot
//! occupy a single trie key, so the agent decomposes the range with
//! [`port_range_to_masked_ports`] and inserts one entry per masked-port
//! prefix. The datapath resolves a concrete packet's `dport` via
//! longest-prefix-match: among matching prefixes the one with the most
//! specific port mask wins (an exact `/16` port beats a looser range
//! prefix).
//!
//! Precedence, mirroring `__policy_get`'s lookup order:
//!   1. Specific peer identity — longest port prefix wins.
//!   2. `ID_ALL` ("world") fallback — longest port prefix wins.
//!   3. Default — deny.
//!
//! `proto` and `direction` are exact-match dimensions (the trie keys
//! them as static prefix bits, never wildcarded by the port range).

use crate::ebpf_sim::bpf_host_sim::{Direction, HostVerdict, ID_ALL};
use crate::ebpf_sim::port_range::port_range_to_masked_ports;
use crate::ebpf_sim::program::L4Proto;
use std::collections::HashMap;

/// Trie key: a peer identity + protocol + direction + a masked-port
/// prefix. The `mask` encodes the prefix length over the 16-bit port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct RangeKey {
    peer_identity: u32,
    proto: u8,
    direction: Direction,
    port: u16,
    mask: u16,
}

/// L4 policy map that supports port ranges via masked-port prefixes,
/// resolving concrete ports with longest-prefix-match. Userspace stand
/// in for Cilium's `cilium_policy_v2` LPM trie.
#[derive(Debug, Default, Clone)]
pub struct RangePolicyMap {
    entries: HashMap<RangeKey, HostVerdict>,
}

impl RangePolicyMap {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Number of trie entries (one per masked-port prefix). Useful for
    /// asserting the decomposition fan-out.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Insert an L4 rule spanning `[start, end]` for `peer_identity`.
    /// The range is decomposed into masked-port prefixes; each becomes a
    /// trie entry. A later insert at the same prefix overwrites, matching
    /// upstream `bpf_map_update_elem` semantics.
    pub fn insert_range(
        &mut self,
        peer_identity: u32,
        start: u16,
        end: u16,
        proto: L4Proto,
        direction: Direction,
        verdict: HostVerdict,
    ) {
        for mp in port_range_to_masked_ports(start, end) {
            let key = RangeKey {
                peer_identity,
                proto: proto.proto_num(),
                direction,
                port: mp.port,
                mask: mp.mask,
            };
            self.entries.insert(key, verdict);
        }
    }

    /// Longest-prefix-match for `port` against the prefixes stored for
    /// `(peer_identity, proto, direction)`. Returns the verdict of the
    /// most specific (largest mask) match, or `None`.
    fn lookup_peer(
        &self,
        peer_identity: u32,
        port: u16,
        proto: L4Proto,
        direction: Direction,
    ) -> Option<HostVerdict> {
        let proto = proto.proto_num();
        // Walk prefix lengths from most specific (/16, mask 0xffff) to
        // least specific (/0, mask 0x0000). The first hit is the LPM.
        for wildcard_bits in 0..=16u32 {
            let mask: u16 = if wildcard_bits >= 16 {
                0
            } else {
                u16::MAX << wildcard_bits
            };
            let key = RangeKey {
                peer_identity,
                proto,
                direction,
                port: port & mask,
                mask,
            };
            if let Some(v) = self.entries.get(&key) {
                return Some(*v);
            }
        }
        None
    }

    /// Resolve the verdict for a concrete packet 5-tuple field set.
    /// Specific-peer rules take precedence over the `ID_ALL` world
    /// fallback; within a peer, longest port prefix wins. Default deny.
    pub fn lookup(
        &self,
        peer_identity: u32,
        port: u16,
        proto: L4Proto,
        direction: Direction,
    ) -> HostVerdict {
        if let Some(v) = self.lookup_peer(peer_identity, port, proto, direction) {
            return v;
        }
        if peer_identity != ID_ALL {
            if let Some(v) = self.lookup_peer(ID_ALL, port, proto, direction) {
                return v;
            }
        }
        HostVerdict::Deny
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_port_inserts_one_full_mask_entry() {
        let mut m = RangePolicyMap::new();
        m.insert_range(1, 443, 443, L4Proto::Tcp, Direction::Ingress, HostVerdict::Allow);
        assert_eq!(m.len(), 1);
        assert_eq!(m.lookup(1, 443, L4Proto::Tcp, Direction::Ingress), HostVerdict::Allow);
    }

    #[test]
    fn range_fan_out_matches_decomposition_size() {
        let mut m = RangePolicyMap::new();
        // 1-1023 decomposes into exactly 10 masked-port prefixes.
        m.insert_range(1, 1, 1023, L4Proto::Tcp, Direction::Ingress, HostVerdict::Allow);
        assert_eq!(m.len(), 10);
    }

    #[test]
    fn empty_map_denies() {
        let m = RangePolicyMap::new();
        assert!(m.is_empty());
        assert_eq!(m.lookup(1, 80, L4Proto::Tcp, Direction::Ingress), HostVerdict::Deny);
    }
}
