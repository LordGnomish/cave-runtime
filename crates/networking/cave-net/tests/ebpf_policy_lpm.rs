// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD: port-range-aware L4 policy LPM lookup.
//!
//! Upstream: cilium/pkg/maps/policymap/policymap.go (the policy map is
//! a `BPF_MAP_TYPE_LPM_TRIE`) + cilium/bpf/lib/policy.h `__policy_get`
//! (v1.19.3, Apache-2.0). A NetworkPolicy port range is inserted as the
//! masked-port prefixes produced by `PortRangeToMaskedPorts`; datapath
//! lookup of a concrete port is a longest-prefix-match: the entry with
//! the most specific port mask wins.

use cave_net::ebpf_sim::bpf_host_sim::{Direction, HostVerdict};
use cave_net::ebpf_sim::policy_lpm::RangePolicyMap;
use cave_net::ebpf_sim::program::L4Proto;

#[test]
fn range_allows_ports_inside_and_denies_outside() {
    let mut m = RangePolicyMap::new();
    // Allow TCP 8080-8090 from identity 42 ingress.
    m.insert_range(42, 8080, 8090, L4Proto::Tcp, Direction::Ingress, HostVerdict::Allow);

    for p in 8080..=8090 {
        assert_eq!(
            m.lookup(42, p, L4Proto::Tcp, Direction::Ingress),
            HostVerdict::Allow,
            "port {p} should be allowed",
        );
    }
    // Just outside the range → default deny.
    assert_eq!(m.lookup(42, 8079, L4Proto::Tcp, Direction::Ingress), HostVerdict::Deny);
    assert_eq!(m.lookup(42, 8091, L4Proto::Tcp, Direction::Ingress), HostVerdict::Deny);
}

#[test]
fn more_specific_exact_port_overrides_range_longest_prefix_wins() {
    let mut m = RangePolicyMap::new();
    // Broad range allow.
    m.insert_range(42, 8000, 9000, L4Proto::Tcp, Direction::Ingress, HostVerdict::Allow);
    // A single port inside it explicitly denied — longest prefix (full
    // mask) must win over the range's looser prefix.
    m.insert_range(42, 8085, 8085, L4Proto::Tcp, Direction::Ingress, HostVerdict::Deny);

    assert_eq!(m.lookup(42, 8084, L4Proto::Tcp, Direction::Ingress), HostVerdict::Allow);
    assert_eq!(m.lookup(42, 8085, L4Proto::Tcp, Direction::Ingress), HostVerdict::Deny);
    assert_eq!(m.lookup(42, 8086, L4Proto::Tcp, Direction::Ingress), HostVerdict::Allow);
}

#[test]
fn full_range_acts_as_port_wildcard() {
    let mut m = RangePolicyMap::new();
    m.insert_range(7, 0, 65535, L4Proto::Tcp, Direction::Egress, HostVerdict::Allow);
    assert_eq!(m.lookup(7, 1, L4Proto::Tcp, Direction::Egress), HostVerdict::Allow);
    assert_eq!(m.lookup(7, 443, L4Proto::Tcp, Direction::Egress), HostVerdict::Allow);
    assert_eq!(m.lookup(7, 65535, L4Proto::Tcp, Direction::Egress), HostVerdict::Allow);
}

#[test]
fn proto_and_direction_are_distinct_dimensions() {
    let mut m = RangePolicyMap::new();
    m.insert_range(42, 53, 53, L4Proto::Udp, Direction::Egress, HostVerdict::Allow);
    // Right port+identity, wrong proto → deny.
    assert_eq!(m.lookup(42, 53, L4Proto::Tcp, Direction::Egress), HostVerdict::Deny);
    // Right port+proto, wrong direction → deny.
    assert_eq!(m.lookup(42, 53, L4Proto::Udp, Direction::Ingress), HostVerdict::Deny);
    // Exact match → allow.
    assert_eq!(m.lookup(42, 53, L4Proto::Udp, Direction::Egress), HostVerdict::Allow);
}

#[test]
fn identity_wildcard_world_fallback_when_no_specific_peer() {
    let mut m = RangePolicyMap::new();
    // World fallback (ID_ALL=0): allow 80 from anyone.
    m.insert_range(0, 80, 80, L4Proto::Tcp, Direction::Ingress, HostVerdict::Allow);
    // Unknown peer 99 still matches the world rule.
    assert_eq!(m.lookup(99, 80, L4Proto::Tcp, Direction::Ingress), HostVerdict::Allow);
    // But a specific-peer rule takes precedence over world.
    m.insert_range(99, 80, 80, L4Proto::Tcp, Direction::Ingress, HostVerdict::Deny);
    assert_eq!(m.lookup(99, 80, L4Proto::Tcp, Direction::Ingress), HostVerdict::Deny);
    // Other peers still get the world allow.
    assert_eq!(m.lookup(123, 80, L4Proto::Tcp, Direction::Ingress), HostVerdict::Allow);
}

#[test]
fn overlapping_ranges_same_specificity_latest_insert_wins_within_prefix() {
    // Two ranges covering the same port at the SAME mask specificity:
    // upstream map semantics overwrite the key, so the later insert wins.
    let mut m = RangePolicyMap::new();
    m.insert_range(5, 100, 100, L4Proto::Tcp, Direction::Ingress, HostVerdict::Allow);
    m.insert_range(5, 100, 100, L4Proto::Tcp, Direction::Ingress, HostVerdict::Deny);
    assert_eq!(m.lookup(5, 100, L4Proto::Tcp, Direction::Ingress), HostVerdict::Deny);
}
