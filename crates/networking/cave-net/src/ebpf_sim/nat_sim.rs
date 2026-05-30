// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Userspace simulation of Cilium's IPv4 SNAT / masquerade datapath.
//!
//! Cite: cilium/bpf/lib/nat.h (v1.19.3) — `snat_v4_new_mapping`,
//!       `__snat_try_keep_port`, `__snat_clamp_port_range`,
//!       `set_v4_rtuple`, `snat_v4_nat` (forward track path).
//!
//! This is a **userspace datapath approximation**, not a stub: it
//! reproduces the observable state machine of masquerading —
//!
//!   * **Port allocation.** `__snat_try_keep_port` keeps the original
//!     source port if it falls in the target range, else clamps a
//!     pseudo-random value with the upstream biased-multiply formula.
//!     On a reverse-tuple collision the loop retries up to
//!     `SNAT_COLLISION_RETRIES` (32), using `prandom` for the first
//!     retry then a linear `port + 1` scan — byte-for-byte the
//!     upstream `for` loop in `snat_v4_new_mapping`.
//!   * **Two map entries per flow.** A *forward* SNAT entry keyed by
//!     the original tuple (rewrites source → `target.addr:port`) and a
//!     *reverse* RevSNAT entry keyed by the swapped reply tuple
//!     (restores the original source on the way back). This mirrors
//!     `__snat_create(map, otuple, ...)` + `__snat_create(map, &rtuple, ...)`.
//!
//! What is intentionally **out of scope** (covered by upstream's
//! kernel BPF test harness, not reproducible in a deterministic
//! `cargo test`): packet-buffer rewriting, L3/L4 checksum fixups,
//! and the netlink/conntrack-GC side channels. Those are pure
//! wire mechanics with no control-plane state.

use crate::ebpf_sim::helpers::Helpers;
use crate::ebpf_sim::map::{Map, UpdateFlag};
use serde::{Deserialize, Serialize};

/// `SNAT_COLLISION_RETRIES` — `bpf/node_config.h`.
pub const SNAT_COLLISION_RETRIES: u32 = 32;

/// Tuple direction. Upstream encodes this in `ipv4_ct_tuple.flags` as
/// `TUPLE_F_OUT` (forward / original) and `TUPLE_F_IN` (reply).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NatDir {
    /// `TUPLE_F_OUT` — the original (to-be-SNATed) direction.
    Out,
    /// `TUPLE_F_IN` — reply traffic; matches the reverse mapping.
    In,
}

/// `struct ipv4_ct_tuple` reduced to the fields a userspace NAT
/// lookup keys on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NatTuple {
    pub saddr: u32,
    pub daddr: u32,
    pub sport: u16,
    pub dport: u16,
    pub nexthdr: u8,
    pub dir: NatDir,
}

/// `struct ipv4_nat_target` — where to masquerade to + port range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NatTarget {
    /// `target->addr` — the masquerade source address.
    pub addr: u32,
    /// `target->min_port` / `max_port` — host-order ephemeral range.
    pub min_port: u16,
    pub max_port: u16,
}

/// `struct ipv4_nat_entry` reduced to the rewrite it encodes.
///
/// For a **forward** entry `to_addr/to_port` is the masquerade
/// `target.addr` + allocated source port. For a **reverse** entry it
/// is the *original* source addr + port to restore on the reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NatEntry {
    pub created_at_ns: u64,
    pub to_addr: u32,
    pub to_port: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NatError {
    /// `DROP_NAT_NO_MAPPING` — no free port after all retries.
    NoMapping,
}

pub type NatMap = Map<NatTuple, NatEntry>;

/// The SNAT map is an LRU hash in the kernel (`cilium_snat_v4_external`).
pub fn new_snat_map(capacity: u32) -> NatMap {
    Map::new_lru_hash(capacity)
}

/// `__snat_clamp_port_range(start, end, val)`.
///
/// `n = (end - start) + 1; m = val * n; return start + (m >> 16)`.
/// Biased-multiply bounded-rand (see the upstream comment citing
/// <https://www.pcg-random.org/posts/bounded-rands.html>).
pub fn clamp_port_range(start: u16, end: u16, val: u16) -> u16 {
    let n = (end - start) as u32 + 1;
    let m = val as u32 * n;
    start + (m >> 16) as u16
}

/// `__snat_try_keep_port(start, end, val)` with the prandom value
/// supplied by the caller (pure — lets a test pin the fallback).
pub fn try_keep_port(start: u16, end: u16, val: u16, prandom: u16) -> u16 {
    if val >= start && val <= end {
        val
    } else {
        clamp_port_range(start, end, prandom)
    }
}

/// `set_v4_rtuple` — build the reply (reverse) tuple for a forward
/// tuple + the chosen masquerade address/port.
fn reverse_tuple(otuple: &NatTuple, to_addr: u32, to_port: u16) -> NatTuple {
    NatTuple {
        dir: NatDir::In,
        nexthdr: otuple.nexthdr,
        saddr: otuple.daddr,
        daddr: to_addr,
        sport: otuple.dport,
        dport: to_port,
    }
}

/// `snat_v4_new_mapping` — allocate a source port and create the
/// forward + reverse entries. Returns the forward `NatEntry`.
pub fn snat_v4_new_mapping(
    map: &mut NatMap,
    otuple: &NatTuple,
    target: &NatTarget,
    helpers: &Helpers,
) -> Result<NatEntry, NatError> {
    let now = helpers.ktime_get_ns();

    // Reverse-mapping payload: restores the original source.
    let rstate = NatEntry {
        created_at_ns: now,
        to_addr: otuple.saddr,
        to_port: otuple.sport,
    };

    // `__snat_try_keep_port`, with the prandom fallback consumed
    // lazily — exactly the upstream ternary (only the false branch
    // calls `get_prandom_u32`).
    let mut port = if otuple.sport >= target.min_port && otuple.sport <= target.max_port {
        otuple.sport
    } else {
        clamp_port_range(target.min_port, target.max_port, helpers.get_prandom_u32() as u16)
    };

    let mut retries = 0u32;
    while retries < SNAT_COLLISION_RETRIES {
        let rtuple = reverse_tuple(otuple, target.addr, port);
        // Try to create the RevSNAT entry (BPF_NOEXIST).
        if map.update(rtuple, rstate, UpdateFlag::NoExist).is_ok() {
            // create_nat_entry: forward entry rewrites src -> target.
            let ostate = NatEntry {
                created_at_ns: now,
                to_addr: target.addr,
                to_port: port,
            };
            if map.update(*otuple, ostate, UpdateFlag::NoExist).is_err() {
                // Rollback the reverse entry we just made.
                let _ = map.delete(&rtuple);
                return Err(NatError::NoMapping);
            }
            return Ok(ostate);
        }
        // Collision: pick the next candidate. First retry uses
        // prandom, subsequent retries scan linearly (`port + 1`).
        let seed = if retries > 0 {
            port.wrapping_add(1)
        } else {
            helpers.get_prandom_u32() as u16
        };
        port = clamp_port_range(target.min_port, target.max_port, seed);
        retries += 1;
    }
    Err(NatError::NoMapping)
}

/// `snat_v4_nat` forward path: return the existing mapping for this
/// flow if one exists, else allocate a new one. Idempotent per-flow.
pub fn snat_v4_track(
    map: &mut NatMap,
    otuple: &NatTuple,
    target: &NatTarget,
    helpers: &Helpers,
) -> Result<NatEntry, NatError> {
    if let Some(entry) = map.lookup(otuple) {
        return Ok(entry);
    }
    snat_v4_new_mapping(map, otuple, target, helpers)
}

/// Reverse lookup for reply traffic. `rtuple` is the reply packet's
/// 5-tuple (with `NatDir::In`); the returned entry's `to_addr/to_port`
/// restore the original source.
pub fn snat_v4_rev_lookup(map: &mut NatMap, rtuple: &NatTuple) -> Option<NatEntry> {
    map.lookup(rtuple)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ebpf_sim::program::L4Proto;

    fn otuple(sport: u16) -> NatTuple {
        NatTuple {
            saddr: u32::from_be_bytes([10, 0, 0, 5]),
            daddr: u32::from_be_bytes([1, 1, 1, 1]),
            sport,
            dport: 443,
            nexthdr: L4Proto::Tcp.proto_num(),
            dir: NatDir::Out,
        }
    }

    /// Upstream `__snat_clamp_port_range` biased multiply.
    #[test]
    fn clamp_single_port_range_is_constant() {
        assert_eq!(clamp_port_range(100, 100, 0), 100);
        assert_eq!(clamp_port_range(100, 100, 65535), 100);
    }

    #[test]
    fn try_keep_port_keeps_in_range() {
        assert_eq!(try_keep_port(1024, 65535, 40000, 7), 40000);
        assert_ne!(try_keep_port(1024, 65535, 80, 0xFFFF), 80);
    }

    /// Forward + reverse entries created; src rewritten to target.
    #[test]
    fn new_mapping_creates_forward_and_reverse() {
        let h = Helpers::new();
        let mut m = new_snat_map(1024);
        let t = NatTarget { addr: u32::from_be_bytes([192, 168, 1, 10]), min_port: 1024, max_port: 65535 };
        let e = snat_v4_new_mapping(&mut m, &otuple(40000), &t, &h).unwrap();
        assert_eq!(e.to_addr, t.addr);
        assert_eq!(e.to_port, 40000);
        assert_eq!(m.len(), 2);
    }
}
