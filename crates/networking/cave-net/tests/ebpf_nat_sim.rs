// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Userspace SNAT/masquerade datapath parity tests.
//!
//! Cite: cilium/bpf/lib/nat.h (v1.19.3) — `snat_v4_new_mapping`,
//! `__snat_try_keep_port`, `__snat_clamp_port_range`, `set_v4_rtuple`.
//!
//! These exercise the observable port-allocation + forward/reverse
//! mapping state machine. Packet-buffer rewriting and L4 checksum
//! fixups are out of scope (upstream covers those under the kernel
//! BPF test harness); this is the userspace datapath approximation.

use cave_net::ebpf_sim::helpers::Helpers;
use cave_net::ebpf_sim::nat_sim::{
    clamp_port_range, new_snat_map, snat_v4_new_mapping, snat_v4_rev_lookup, snat_v4_track,
    try_keep_port, NatDir, NatError, NatTarget, NatTuple, SNAT_COLLISION_RETRIES,
};
use cave_net::ebpf_sim::program::L4Proto;

fn otuple(sport: u16) -> NatTuple {
    NatTuple {
        // pod 10.0.0.5 -> remote 1.1.1.1:443
        saddr: u32::from_be_bytes([10, 0, 0, 5]),
        daddr: u32::from_be_bytes([1, 1, 1, 1]),
        sport,
        dport: 443,
        nexthdr: L4Proto::Tcp.proto_num(),
        dir: NatDir::Out,
    }
}

fn target(min: u16, max: u16) -> NatTarget {
    NatTarget {
        // masquerade to node IP 192.168.1.10
        addr: u32::from_be_bytes([192, 168, 1, 10]),
        min_port: min,
        max_port: max,
    }
}

/// `__snat_clamp_port_range`: n=(end-start)+1, m=val*n, start+(m>>16).
#[test]
fn clamp_port_range_matches_upstream_biased_multiply() {
    // Range of 1 port always returns start regardless of val.
    assert_eq!(clamp_port_range(100, 100, 0), 100);
    assert_eq!(clamp_port_range(100, 100, 65535), 100);
    // Range [100,101] (n=2): val>=32768 -> 101, else 100.
    assert_eq!(clamp_port_range(100, 101, 0), 100);
    assert_eq!(clamp_port_range(100, 101, 32767), 100);
    assert_eq!(clamp_port_range(100, 101, 32768), 101);
    assert_eq!(clamp_port_range(100, 101, 65535), 101);
    // Result always within [start,end].
    for v in [0u16, 1, 1000, 30000, 60000, 65535] {
        let p = clamp_port_range(1024, 65535, v);
        assert!((1024..=65535).contains(&p), "port {p} out of range for val {v}");
    }
}

/// `__snat_try_keep_port`: keep val if in [start,end], else clamp prandom.
#[test]
fn try_keep_port_retains_in_range_value() {
    // In range -> kept verbatim (source-port preservation).
    assert_eq!(try_keep_port(1024, 65535, 40000, 12345), 40000);
    // Out of range -> derived from the prandom argument, not the val.
    let p = try_keep_port(1024, 65535, 80, 0xFFFF);
    assert_ne!(p, 80);
    assert!((1024..=65535).contains(&p));
}

/// First masquerade of a flow: kept source port, forward + reverse
/// entries created, forward entry rewrites source to target.addr.
#[test]
fn new_mapping_allocates_and_creates_both_entries() {
    let helpers = Helpers::new();
    let mut m = new_snat_map(1024);
    let o = otuple(40000); // in default ephemeral range
    let t = target(1024, 65535);
    let entry = snat_v4_new_mapping(&mut m, &o, &t, &helpers).expect("mapping");
    // Source rewritten to the masquerade address.
    assert_eq!(entry.to_addr, t.addr);
    // Source port preserved because 40000 is in range.
    assert_eq!(entry.to_port, 40000);
    // Two entries: the forward SNAT entry + the reverse RevSNAT entry.
    assert_eq!(m.len(), 2);
}

/// Reverse lookup restores the original source addr+port for reply
/// traffic. This is the round-trip that makes masquerade transparent.
#[test]
fn reverse_lookup_restores_original_source() {
    let helpers = Helpers::new();
    let mut m = new_snat_map(1024);
    let o = otuple(40000);
    let t = target(1024, 65535);
    let fwd = snat_v4_new_mapping(&mut m, &o, &t, &helpers).expect("mapping");

    // Reply packet 5-tuple: remote(1.1.1.1):443 -> node(192.168.1.10):to_port
    let rtuple = NatTuple {
        saddr: o.daddr,       // remote
        daddr: t.addr,        // masquerade addr
        sport: o.dport,       // 443
        dport: fwd.to_port,   // allocated source port
        nexthdr: o.nexthdr,
        dir: NatDir::In,
    };
    let rev = snat_v4_rev_lookup(&mut m, &rtuple).expect("reverse entry");
    // Reverse entry restores the original pod IP + source port.
    assert_eq!(rev.to_addr, o.saddr);
    assert_eq!(rev.to_port, o.sport);
}

/// `snat_v4_track` is idempotent: a second packet of the same flow
/// returns the already-allocated mapping, no new entries.
#[test]
fn track_is_idempotent_for_same_flow() {
    let helpers = Helpers::new();
    let mut m = new_snat_map(1024);
    let o = otuple(40000);
    let t = target(1024, 65535);
    let first = snat_v4_track(&mut m, &o, &t, &helpers).expect("first");
    let len_after_first = m.len();
    let second = snat_v4_track(&mut m, &o, &t, &helpers).expect("second");
    assert_eq!(first.to_port, second.to_port);
    assert_eq!(first.to_addr, second.to_addr);
    assert_eq!(m.len(), len_after_first, "no new entries on repeat");
}

/// Port exhaustion: a single-port range whose reverse slot is already
/// taken drains all SNAT_COLLISION_RETRIES and returns NoMapping.
#[test]
fn exhaustion_returns_no_mapping() {
    let helpers = Helpers::new();
    let mut m = new_snat_map(1024);
    let t = target(100, 100); // exactly one port

    // First flow grabs port 100.
    let o1 = otuple(100);
    let e1 = snat_v4_new_mapping(&mut m, &o1, &t, &helpers).expect("first grabs 100");
    assert_eq!(e1.to_port, 100);

    // Second, distinct flow wants the same masquerade addr; only port
    // 100 exists and its reverse slot is taken -> all retries collide.
    let o2 = NatTuple { sport: 100, saddr: u32::from_be_bytes([10, 0, 0, 6]), ..otuple(100) };
    let err = snat_v4_new_mapping(&mut m, &o2, &t, &helpers).unwrap_err();
    assert_eq!(err, NatError::NoMapping);
}

/// Collision then linear-scan success: port 100 taken, range [100,101],
/// prandom seeded to land on 101 for the first retry.
#[test]
fn collision_retries_to_next_free_port() {
    let helpers = Helpers::new();
    helpers.push_prandom(0xFFFF); // -> clamp(100,101,65535) = 101
    let mut m = new_snat_map(1024);
    let t = target(100, 101);

    // Occupy port 100 with a first flow (keeps its in-range sport 100).
    let o1 = otuple(100);
    let e1 = snat_v4_new_mapping(&mut m, &o1, &t, &helpers).expect("first");
    assert_eq!(e1.to_port, 100);

    // Second flow also prefers 100 (collision) -> retry picks 101.
    let o2 = NatTuple { sport: 100, saddr: u32::from_be_bytes([10, 0, 0, 7]), ..otuple(100) };
    let e2 = snat_v4_new_mapping(&mut m, &o2, &t, &helpers).expect("second");
    assert_eq!(e2.to_port, 101);
}

#[test]
fn collision_retries_constant_matches_upstream() {
    assert_eq!(SNAT_COLLISION_RETRIES, 32);
}
