// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Userspace simulation of Cilium's EDT bandwidth scheduler.
//!
//! Cite: cilium/bpf/lib/edt.h `edt_sched_departure` + cilium/pkg/maps/bwmap
//!       (the `cilium_throttle` `BPF_MAP_TYPE_HASH`) + cilium/pkg/bandwidth
//!       (pinned v1.19.3, Apache-2.0).
//!
//! Cilium's bandwidth manager throttles egress per aggregate (an
//! endpoint group) using the kernel's Earliest-Departure-Time model:
//! instead of dropping over-rate packets it stamps each packet with a
//! future `tstamp` so the FQ qdisc paces them out at the configured
//! rate. The BPF half is `edt_sched_departure`, run on egress:
//!
//!   1. Resolve the packet's aggregate id; `0` ("no aggregate") → pass.
//!   2. Look up the aggregate in `cilium_throttle`; miss → pass.
//!   3. `bps == 0` (no rate) → pass.
//!   4. `t = max(packet.tstamp, now)` — never schedule into the past.
//!   5. `delay = len * NSEC_PER_SEC / bps` — transmission time at the
//!      configured byte-rate.
//!   6. `t_next = t_last + delay` — earliest this packet may depart.
//!      * `t_next <= t` → the aggregate is already drained past this
//!        packet: fast-admit, advance `t_last = t`, leave tstamp alone.
//!      * `t_next - now >= t_horizon_drop` → the FQ drop horizon would
//!        reject it anyway; drop now so `t_last` is not corrupted.
//!      * otherwise admit: `t_last = t_next`, stamp `tstamp = t_next`.
//!
//! `bps` is stored in **bytes per second** (the manager divides the
//! `kubernetes.io/egress-bandwidth` bit-rate annotation by 8 before
//! writing the map), so `delay` comes out in nanoseconds directly.
//!
//! Out of scope (kernel BPF harness owns it): the `validate_ethertype`
//! / IPv4-or-IPv6 gate, reading the aggregate id out of `skb->cb`, and
//! the actual FQ qdisc that consumes the stamped `tstamp`. Those are
//! pure wire/qdisc mechanics with no control-plane state. This sim
//! reproduces the **departure-time arithmetic and map state machine**.

use std::collections::BTreeMap;

/// Nanoseconds per second. Mirrors upstream `NSEC_PER_SEC`.
pub const NSEC_PER_SEC: u64 = 1_000_000_000;

/// Default FQ drop horizon: a packet scheduled more than this far past
/// `now` is dropped rather than stamped. The bandwidth manager defaults
/// `t_horizon_drop` to `2 * NSEC_PER_SEC` (`DEFAULT_DROP_HORIZON`).
pub const DEFAULT_DROP_HORIZON_NS: u64 = 2 * NSEC_PER_SEC;

/// One `cilium_throttle` map value (`struct edt_info`). `bps` is the
/// rate in **bytes/sec**; `t_last` is the running earliest-departure
/// watermark the datapath advances per packet; `t_horizon_drop` is the
/// per-aggregate FQ drop horizon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdtInfo {
    pub bps: u64,
    pub t_last: u64,
    pub t_horizon_drop: u64,
}

impl EdtInfo {
    /// A fresh aggregate with `t_last == 0` (never transmitted yet).
    pub fn new(bps: u64, t_horizon_drop: u64) -> Self {
        Self {
            bps,
            t_last: 0,
            t_horizon_drop,
        }
    }

    /// An aggregate whose watermark is pre-seeded — models a flow already
    /// mid-stream when the packet arrives.
    pub fn with_t_last(bps: u64, t_horizon_drop: u64, t_last: u64) -> Self {
        Self {
            bps,
            t_last,
            t_horizon_drop,
        }
    }
}

/// The datapath verdict — `CTX_ACT_OK` (`Pass`) or `CTX_ACT_DROP`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdtVerdict {
    Pass,
    Drop,
}

/// Schedule a packet's departure against an aggregate's `EdtInfo`,
/// mutating `t_last` in place exactly as the kernel `WRITE_ONCE`s the
/// looked-up map pointer. Returns `(verdict, tstamp)` where `tstamp` is
/// the (possibly forward-stamped) departure timestamp the FQ qdisc would
/// see. On `Pass`-fast and `Drop` the timestamp is returned unchanged.
///
/// Verbatim port of `edt_sched_departure`'s arithmetic core (steps 3-6
/// above); the ethertype gate and aggregate-id extraction are handled by
/// [`EdtThrottleMap::schedule`].
pub fn edt_sched_departure(
    info: &mut EdtInfo,
    packet_len: u64,
    now_ns: u64,
    tstamp_ns: u64,
) -> (EdtVerdict, u64) {
    // No rate configured → CTX_ACT_OK.
    if info.bps == 0 {
        return (EdtVerdict::Pass, tstamp_ns);
    }

    // t = max(tstamp, now): never schedule a departure into the past.
    let t = if tstamp_ns < now_ns { now_ns } else { tstamp_ns };

    let delay = packet_len * NSEC_PER_SEC / info.bps;
    let t_next = info.t_last + delay;

    // Aggregate already drained past this packet → fast admit. Upstream
    // advances t_last to t but does NOT touch the packet tstamp.
    if t_next <= t {
        info.t_last = t;
        return (EdtVerdict::Pass, tstamp_ns);
    }

    // FQ would reject anything past the horizon; drop now so t_last is
    // not advanced into a corrupt future. Boundary is inclusive (`>=`).
    if t_next - now_ns >= info.t_horizon_drop {
        return (EdtVerdict::Drop, tstamp_ns);
    }

    // Admit, pacing the packet: advance the watermark and stamp the
    // packet's departure forward.
    info.t_last = t_next;
    (EdtVerdict::Pass, t_next)
}

/// The `cilium_throttle` `BPF_MAP_TYPE_HASH`, keyed by aggregate id
/// (`struct edt_id { __u64 id; }`). Wraps a `BTreeMap` so `schedule`
/// can mutate the looked-up value in place — modelling the kernel's
/// `map_lookup_elem` returning a writable pointer.
#[derive(Debug, Default)]
pub struct EdtThrottleMap {
    entries: BTreeMap<u64, EdtInfo>,
}

impl EdtThrottleMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Install / overwrite an aggregate's throttle info.
    pub fn insert(&mut self, aggregate_id: u64, info: EdtInfo) {
        self.entries.insert(aggregate_id, info);
    }

    /// Read-only peek (test/inspection helper).
    pub fn get(&self, aggregate_id: u64) -> Option<EdtInfo> {
        self.entries.get(&aggregate_id).copied()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Full `edt_sched_departure` flow including the aggregate-id /
    /// map-miss gates. `aggregate_id == 0` ("no aggregate") or an absent
    /// entry → `CTX_ACT_OK` with the tstamp untouched.
    pub fn schedule(
        &mut self,
        aggregate_id: u64,
        packet_len: u64,
        now_ns: u64,
        tstamp_ns: u64,
    ) -> (EdtVerdict, u64) {
        if aggregate_id == 0 {
            return (EdtVerdict::Pass, tstamp_ns);
        }
        match self.entries.get_mut(&aggregate_id) {
            Some(info) => edt_sched_departure(info, packet_len, now_ns, tstamp_ns),
            None => (EdtVerdict::Pass, tstamp_ns),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_packet_fresh_aggregate_admits_and_seeds_watermark() {
        // t_last starts at 0, now in the future → t_next (delay) << now →
        // fast admit, t_last clamped up to now.
        let mut info = EdtInfo::new(125_000, DEFAULT_DROP_HORIZON_NS);
        let now = 5 * NSEC_PER_SEC;
        let (v, tstamp) = edt_sched_departure(&mut info, 1500, now, now);
        assert_eq!(v, EdtVerdict::Pass);
        assert_eq!(tstamp, now);
        assert_eq!(info.t_last, now);
    }

    #[test]
    fn back_to_back_packets_accumulate_delay() {
        // Two packets at the same instant should stack their delays:
        // the second departs one `delay` after the first.
        let now = 5 * NSEC_PER_SEC;
        let mut info = EdtInfo::with_t_last(125_000, DEFAULT_DROP_HORIZON_NS, now);
        let delay = 1500u64 * NSEC_PER_SEC / 125_000;
        let (_, t1) = edt_sched_departure(&mut info, 1500, now, now);
        assert_eq!(t1, now + delay);
        let (_, t2) = edt_sched_departure(&mut info, 1500, now, now);
        assert_eq!(t2, now + 2 * delay);
    }
}
