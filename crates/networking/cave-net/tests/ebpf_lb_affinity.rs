// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Userspace load-balancer **session-affinity** datapath parity tests.
//!
//! Cite: cilium/bpf/lib/lb.h (v1.19.3) — `__lb4_affinity_backend_id`,
//! `__lb4_update_affinity`, `lb4_affinity_backend_id_by_addr`,
//! `lb_affinity_key` / `lb_affinity_val` / `lb_affinity_match`.
//!
//! Cilium pins a client to one backend for `affinity_timeout` seconds:
//! the first packet of a new connection selects a backend (random) and
//! records `(client_ip, rev_nat_id) -> (backend_id, last_used)` in
//! `cilium_lb4_affinity`. Subsequent packets within the window reuse
//! that backend; the entry's `last_used` is refreshed on every hit, the
//! entry is dropped once it ages past the timeout, and dropped too if
//! the backend left the service (`cilium_lb_affinity_match` miss).
//!
//! This exercises the observable state machine. The monotonic clock is
//! pinned per call (`now_ns`) the way `bpf_mono_now()` reads it.

use cave_net::ebpf_sim::lb_sim::{LbAlgo, LbBackend, LbMaps, LbTuple};
use cave_net::ebpf_sim::program::L4Proto;

const SEC: u64 = 1_000_000_000;

fn vip() -> u32 {
    u32::from_be_bytes([172, 20, 0, 1])
}

fn tuple(saddr: [u8; 4], sport: u16) -> LbTuple {
    LbTuple {
        saddr: u32::from_be_bytes(saddr),
        daddr: vip(),
        sport,
        dport: 80,
        nexthdr: L4Proto::Tcp.proto_num(),
    }
}

fn three_backend_maps(affinity_timeout: u32) -> LbMaps {
    let mut maps = LbMaps::new();
    maps.add_service_with_affinity(
        vip(),
        80,
        L4Proto::Tcp.proto_num(),
        9,
        LbAlgo::Random,
        &[
            (1, LbBackend { address: u32::from_be_bytes([10, 1, 0, 1]), port: 8080 }),
            (2, LbBackend { address: u32::from_be_bytes([10, 1, 0, 2]), port: 8080 }),
            (3, LbBackend { address: u32::from_be_bytes([10, 1, 0, 3]), port: 8080 }),
        ],
        affinity_timeout,
    );
    maps
}

#[test]
fn first_packet_records_affinity_and_is_sticky() {
    let mut maps = three_backend_maps(100);
    let t = tuple([10, 0, 0, 5], 5000);

    // prandom=0 -> slot 1 -> backend_id 1 on the CT_NEW path.
    let first = maps.lb4_local_affinity(&t, 0, 1_000).unwrap();
    assert_eq!(first.backend_id, 1);

    // A wildly different prandom would pick a different slot, but the
    // sticky entry must override it: same client -> same backend.
    let second = maps.lb4_local_affinity(&t, 999, 2_000).unwrap();
    assert_eq!(second.backend_id, 1, "session affinity must override random selection");
    let third = maps.lb4_local_affinity(&t, 7, 3_000).unwrap();
    assert_eq!(third.backend_id, 1);
}

#[test]
fn affinity_disabled_when_timeout_zero() {
    let mut maps = three_backend_maps(0);
    let t = tuple([10, 0, 0, 6], 5001);
    // timeout 0 => no stickiness; selection follows prandom each time.
    let a = maps.lb4_local_affinity(&t, 0, 1_000).unwrap(); // slot 1
    let b = maps.lb4_local_affinity(&t, 1, 2_000).unwrap(); // slot 2
    assert_eq!(a.backend_id, 1);
    assert_eq!(b.backend_id, 2);
}

#[test]
fn affinity_expires_after_timeout_window() {
    let mut maps = three_backend_maps(100); // 100s window
    let t = tuple([10, 0, 0, 7], 5002);
    let first = maps.lb4_local_affinity(&t, 0, 0).unwrap(); // backend 1 @ t=0
    assert_eq!(first.backend_id, 1);

    // last_used(0) + 100s <= now  => expired exactly at the boundary.
    // After expiry a new selection is made (prandom=1 -> slot 2).
    let after = maps.lb4_local_affinity(&t, 1, 100 * SEC).unwrap();
    assert_eq!(after.backend_id, 2, "expired affinity must re-select");

    // Just inside the window the entry is still honored.
    let mut maps2 = three_backend_maps(100);
    maps2.lb4_local_affinity(&t, 0, 0).unwrap(); // backend 1
    let inside = maps2.lb4_local_affinity(&t, 1, 100 * SEC - 1).unwrap();
    assert_eq!(inside.backend_id, 1, "within window stays sticky");
}

#[test]
fn last_used_is_refreshed_on_each_hit() {
    let mut maps = three_backend_maps(100);
    let t = tuple([10, 0, 0, 8], 5003);
    maps.lb4_local_affinity(&t, 0, 0).unwrap(); // last_used = 0
    // Hit at t=90s refreshes last_used to 90s...
    maps.lb4_local_affinity(&t, 9, 90 * SEC).unwrap();
    // ...so at t=180s (90s after the refresh, < 100s window) it sticks.
    let still = maps.lb4_local_affinity(&t, 9, 180 * SEC).unwrap();
    assert_eq!(still.backend_id, 1, "refresh must slide the window forward");
}

#[test]
fn stale_backend_drops_affinity_entry() {
    let mut maps = three_backend_maps(100);
    let t = tuple([10, 0, 0, 9], 5004);
    let first = maps.lb4_local_affinity(&t, 0, 1_000).unwrap();
    assert_eq!(first.backend_id, 1);

    // Backend 1 leaves the service (affinity-match removed). The next
    // lookup detects the stale match, drops the entry, and re-selects.
    maps.remove_affinity_match(9, 1);
    let next = maps.lb4_local_affinity(&t, 1, 2_000).unwrap();
    assert_eq!(next.backend_id, 2, "stale backend must force re-selection");
}

#[test]
fn low_level_affinity_backend_id_miss_returns_zero() {
    let mut maps = three_backend_maps(100);
    // No entry recorded yet for this client => 0 (CT_NEW must select).
    let bid = maps.lb_affinity_backend_id(9, 100, u32::from_be_bytes([10, 0, 0, 42]), 1_000);
    assert_eq!(bid, 0);
}

#[test]
fn distinct_clients_get_independent_affinity() {
    let mut maps = three_backend_maps(100);
    let a = tuple([10, 0, 0, 20], 6000);
    let b = tuple([10, 0, 0, 21], 6001);
    let xa = maps.lb4_local_affinity(&a, 0, 1_000).unwrap(); // backend 1
    let xb = maps.lb4_local_affinity(&b, 1, 1_000).unwrap(); // backend 2
    assert_eq!(xa.backend_id, 1);
    assert_eq!(xb.backend_id, 2);
    // Each client stays on its own backend regardless of prandom.
    assert_eq!(maps.lb4_local_affinity(&a, 2, 2_000).unwrap().backend_id, 1);
    assert_eq!(maps.lb4_local_affinity(&b, 0, 2_000).unwrap().backend_id, 2);
}
