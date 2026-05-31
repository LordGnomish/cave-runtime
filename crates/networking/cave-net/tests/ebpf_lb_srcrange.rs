// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Userspace LB **source-range** (LoadBalancerSourceRanges) ACL tests.
//!
//! Cite: cilium/bpf/lib/lb.h (v1.19.3) — `lb4_src_range_ok`,
//! `struct lb4_src_range_key`, `cilium_lb4_source_range` (LPM trie),
//! `SVC_FLAG_SOURCE_RANGE_DENY`.
//!
//! A service may restrict which client CIDRs reach it. The datapath
//! does an LPM lookup of the source IP in `cilium_lb4_source_range`
//! (keyed by `rev_nat_id`): a hit means the client is within a
//! configured range. With no ranges the service is open to all. The
//! `SVC_FLAG_SOURCE_RANGE_DENY` flag inverts the verdict (the ranges
//! become a block-list instead of an allow-list).

use cave_net::ebpf_sim::lb_sim::LbMaps;

fn ip(a: u8, b: u8, c: u8, d: u8) -> u32 {
    u32::from_be_bytes([a, b, c, d])
}

#[test]
fn no_ranges_means_open_to_all() {
    let maps = LbMaps::new();
    // Service 7 has no source ranges configured => everyone passes.
    assert!(maps.lb4_src_range_ok(7, ip(8, 8, 8, 8)));
    assert!(maps.lb4_src_range_ok(7, ip(10, 0, 0, 1)));
}

#[test]
fn allowlist_admits_only_matching_cidrs() {
    let mut maps = LbMaps::new();
    maps.add_source_range(7, ip(10, 0, 0, 0), 8); // 10.0.0.0/8
    maps.add_source_range(7, ip(192, 168, 1, 0), 24); // 192.168.1.0/24

    assert!(maps.lb4_src_range_ok(7, ip(10, 5, 6, 7)), "inside 10/8");
    assert!(maps.lb4_src_range_ok(7, ip(192, 168, 1, 50)), "inside 192.168.1/24");
    assert!(!maps.lb4_src_range_ok(7, ip(192, 168, 2, 1)), "outside both");
    assert!(!maps.lb4_src_range_ok(7, ip(8, 8, 8, 8)), "outside both");
}

#[test]
fn ranges_are_scoped_per_service() {
    let mut maps = LbMaps::new();
    maps.add_source_range(7, ip(10, 0, 0, 0), 8);
    // Service 9 has no ranges of its own => still open, even though 7
    // restricts the same address.
    assert!(!maps.lb4_src_range_ok(7, ip(8, 8, 8, 8)));
    assert!(maps.lb4_src_range_ok(9, ip(8, 8, 8, 8)));
}

#[test]
fn exact_host_route_slash32() {
    let mut maps = LbMaps::new();
    maps.add_source_range(7, ip(203, 0, 113, 5), 32);
    assert!(maps.lb4_src_range_ok(7, ip(203, 0, 113, 5)));
    assert!(!maps.lb4_src_range_ok(7, ip(203, 0, 113, 6)));
}

#[test]
fn slash_zero_matches_everything() {
    let mut maps = LbMaps::new();
    maps.add_source_range(7, 0, 0); // 0.0.0.0/0
    assert!(maps.lb4_src_range_ok(7, ip(1, 2, 3, 4)));
    assert!(maps.lb4_src_range_ok(7, ip(255, 255, 255, 255)));
}

#[test]
fn deny_flag_inverts_verdict() {
    let mut maps = LbMaps::new();
    maps.add_source_range(7, ip(10, 0, 0, 0), 8);
    maps.set_source_range_deny(7, true);
    // Now 10/8 is a block-list: members are denied, others pass.
    assert!(!maps.lb4_src_range_ok(7, ip(10, 1, 2, 3)), "in deny range => blocked");
    assert!(maps.lb4_src_range_ok(7, ip(8, 8, 8, 8)), "outside deny range => allowed");
}

#[test]
fn deny_flag_alone_without_ranges_is_open() {
    let mut maps = LbMaps::new();
    // Deny set but no ranges => has_src_range_check() is false => open.
    maps.set_source_range_deny(7, true);
    assert!(maps.lb4_src_range_ok(7, ip(10, 1, 2, 3)));
}
