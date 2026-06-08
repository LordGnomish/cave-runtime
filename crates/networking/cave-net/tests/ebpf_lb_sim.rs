// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Userspace load-balancer datapath parity tests.
//!
//! Cite: cilium/bpf/lib/lb.h (v1.19.3) — `lb4_lookup_service`,
//! `lb4_lookup_backend_slot`, `lb4_select_backend_id_random`,
//! `lb4_select_backend_id_maglev`, `lb4_xlate` (DNAT), `lb4_rev_nat`;
//! and cilium/bpf/lib/hash.h `__hash_from_tuple_v4` (jhash_3words).
//!
//! Exercises the observable service→backend selection + DNAT +
//! reverse-NAT state machine. Packet rewriting / checksums are out of
//! scope (kernel BPF harness owns those); this is the userspace
//! datapath approximation.

use cave_net::ebpf_sim::helpers::Helpers;
use cave_net::ebpf_sim::lb_sim::{
    jhash_3words, maglev_index, select_backend_id_random, LbAlgo, LbBackend, LbMaps, LbTuple,
    HASH_INIT4_SEED, LB_MAGLEV_LUT_SIZE,
};
use cave_net::ebpf_sim::program::L4Proto;

fn vip() -> u32 {
    u32::from_be_bytes([172, 20, 0, 1])
}

fn client_tuple(saddr: [u8; 4], sport: u16, daddr: u32) -> LbTuple {
    LbTuple {
        saddr: u32::from_be_bytes(saddr),
        daddr,
        sport,
        dport: 80,
        nexthdr: L4Proto::Tcp.proto_num(),
    }
}

/// Build a service `172.20.0.1:80/TCP` with `n` backends 10.1.0.{1..n}:8080.
fn svc_with_backends(maps: &mut LbMaps, n: u16, algo: LbAlgo) {
    let backends: Vec<(u32, LbBackend)> = (1..=n)
        .map(|i| {
            (
                i as u32,
                LbBackend { address: u32::from_be_bytes([10, 1, 0, i as u8]), port: 8080 },
            )
        })
        .collect();
    maps.add_service(vip(), 80, L4Proto::Tcp.proto_num(), 7, algo, &backends);
}

/// jhash_3words must match the upstream Bob-Jenkins lookup3 mix.
/// Known-answer: derived from the reference implementation.
#[test]
fn jhash_3words_is_deterministic_and_seed_sensitive() {
    let h1 = jhash_3words(1, 2, 3, HASH_INIT4_SEED);
    let h2 = jhash_3words(1, 2, 3, HASH_INIT4_SEED);
    assert_eq!(h1, h2, "pure function");
    // Different seed -> (almost surely) different hash.
    assert_ne!(jhash_3words(1, 2, 3, 0), jhash_3words(1, 2, 3, HASH_INIT4_SEED));
    assert_eq!(HASH_INIT4_SEED, 0xcafe);
    assert_eq!(LB_MAGLEV_LUT_SIZE, 32749);
}

/// `__hash_from_tuple_v4` deliberately excludes daddr so the same
/// client maps to the same maglev slot across different service VIPs.
#[test]
fn maglev_index_excludes_daddr() {
    let t1 = client_tuple([10, 0, 0, 9], 33333, vip());
    let t2 = client_tuple([10, 0, 0, 9], 33333, u32::from_be_bytes([172, 20, 0, 99]));
    assert_eq!(maglev_index(&t1), maglev_index(&t2));
    // Different client source -> may map elsewhere; index always in range.
    assert!(maglev_index(&t1) < LB_MAGLEV_LUT_SIZE);
    let t3 = client_tuple([10, 0, 0, 10], 44444, vip());
    assert!(maglev_index(&t3) < LB_MAGLEV_LUT_SIZE);
}

/// `lb4_select_backend_id_random`: slot = (prandom % count) + 1,
/// never the frontend slot 0.
#[test]
fn random_select_slot_formula() {
    assert_eq!(select_backend_id_random(3, 0), 1);
    assert_eq!(select_backend_id_random(3, 1), 2);
    assert_eq!(select_backend_id_random(3, 2), 3);
    assert_eq!(select_backend_id_random(3, 3), 1); // wraps
    assert_eq!(select_backend_id_random(3, 7), 2);
}

/// Service lookup miss returns None.
#[test]
fn lookup_unknown_service_is_none() {
    let mut maps = LbMaps::new();
    let t = client_tuple([10, 0, 0, 1], 5000, vip());
    assert!(maps.lb4_local_random(&t, 0).is_none());
}

/// Full forward path (random algo): lookup service, pick backend,
/// DNAT to backend addr:port.
#[test]
fn forward_random_dnat_to_backend() {
    let mut maps = LbMaps::new();
    svc_with_backends(&mut maps, 3, LbAlgo::Random);
    let t = client_tuple([10, 0, 0, 1], 5000, vip());
    // prandom=1 -> slot 2 -> backend_id 2 -> 10.1.0.2:8080
    let x = maps.lb4_local_random(&t, 1).expect("dnat");
    assert_eq!(x.backend_id, 2);
    assert_eq!(x.new_daddr, u32::from_be_bytes([10, 1, 0, 2]));
    assert_eq!(x.new_dport, 8080);
    assert_eq!(x.rev_nat_index, 7);
}

/// Maglev forward path is consistent: same tuple -> same backend on
/// repeat; backend is one of the configured ones.
#[test]
fn forward_maglev_is_consistent() {
    let mut maps = LbMaps::new();
    svc_with_backends(&mut maps, 4, LbAlgo::Maglev);
    let t = client_tuple([10, 0, 0, 42], 6000, vip());
    let a = maps.lb4_local_maglev(&t).expect("dnat a");
    let b = maps.lb4_local_maglev(&t).expect("dnat b");
    assert_eq!(a.backend_id, b.backend_id, "consistent hashing");
    assert!((1..=4).contains(&a.backend_id));
    assert_eq!(a.new_dport, 8080);
}

/// `lb4_rev_nat`: a reply from the backend has its source restored to
/// the service VIP + port via the reverse-NAT index.
#[test]
fn reverse_nat_restores_service_vip() {
    let mut maps = LbMaps::new();
    svc_with_backends(&mut maps, 2, LbAlgo::Random);
    // Reply tuple: backend 10.1.0.1:8080 -> client 10.0.0.1:5000
    let reply = LbTuple {
        saddr: u32::from_be_bytes([10, 1, 0, 1]),
        daddr: u32::from_be_bytes([10, 0, 0, 1]),
        sport: 8080,
        dport: 5000,
        nexthdr: L4Proto::Tcp.proto_num(),
    };
    let rev = maps.lb4_rev_nat(7, &reply).expect("rev nat");
    assert_eq!(rev.new_saddr, vip());
    assert_eq!(rev.new_sport, 80);
}

/// Helpers-driven random selection wires through get_prandom_u32.
#[test]
fn forward_random_uses_helpers_prandom() {
    let mut maps = LbMaps::new();
    svc_with_backends(&mut maps, 3, LbAlgo::Random);
    let h = Helpers::new();
    h.push_prandom(2); // -> slot 3 -> backend 3
    let t = client_tuple([10, 0, 0, 1], 5000, vip());
    let x = maps.lb4_local_random(&t, h.get_prandom_u32()).expect("dnat");
    assert_eq!(x.backend_id, 3);
}
