// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD port of Cilium's policy port-range → masked-port
//! decomposition.
//!
//! Upstream: cilium/pkg/policy/portrange.go `PortRangeToMaskedPorts`
//! and its exhaustive table test `pkg/policy/portrange_test.go::TestPortRange`
//! (pinned v1.19.3, Apache-2.0).
//!
//! The Cilium datapath indexes L4 policy in a longest-prefix-match
//! trie keyed by `(identity, traffic_dir, nexthdr, dport)`. A port
//! *range* in a NetworkPolicy/CiliumNetworkPolicy `EndPort` cannot be
//! a single trie key, so the agent decomposes `[start, end]` into the
//! minimal set of `(port, mask)` prefixes that exactly tile the range.
//! This is the classic "range → CIDR prefixes" algorithm over the
//! 16-bit port space.

use cave_net::ebpf_sim::port_range::{port_range_to_masked_ports, MaskedPort};

/// Sort masked ports by `port` for deterministic comparison, mirroring
/// the upstream test which sorts before `require.Equal`.
fn sorted(start: u16, end: u16) -> Vec<MaskedPort> {
    let mut v = port_range_to_masked_ports(start, end);
    v.sort_by_key(|m| m.port);
    v
}

fn mp(port: u16, mask: u16) -> MaskedPort {
    MaskedPort { port, mask }
}

/// Upstream `validateMaskedPorts`: the returned masked ports must form
/// a continuous, non-overlapping tiling whose union is exactly
/// `[start, end]`.
fn validate_continuous(masked: &[MaskedPort], start: u16, end: u16) {
    assert!(!masked.is_empty(), "expected non-empty tiling");
    let first = masked[0].port;
    // `^mask` is the count of addresses covered minus one.
    let mut last = first.wrapping_add(!masked[0].mask);
    for m in &masked[1..] {
        assert_eq!(m.port, last + 1, "tiling must be continuous");
        last = m.port.wrapping_add(!m.mask);
    }
    assert_eq!(first, start, "tiling must start at range start");
    assert_eq!(last, end, "tiling must end at range end");
}

#[test]
fn upstream_port_range_worst_case_1_to_65534() {
    let expected = vec![
        mp(0x1, 0xffff),
        mp(0x2, 0xfffe),
        mp(0x4, 0xfffc),
        mp(0x8, 0xfff8),
        mp(0x10, 0xfff0),
        mp(0x20, 0xffe0),
        mp(0x40, 0xffc0),
        mp(0x80, 0xff80),
        mp(0x100, 0xff00),
        mp(0x200, 0xfe00),
        mp(0x400, 0xfc00),
        mp(0x800, 0xf800),
        mp(0x1000, 0xf000),
        mp(0x2000, 0xe000),
        mp(0x4000, 0xc000),
        mp(0x8000, 0xc000),
        mp(0xc000, 0xe000),
        mp(0xe000, 0xf000),
        mp(0xf000, 0xf800),
        mp(0xf800, 0xfc00),
        mp(0xfc00, 0xfe00),
        mp(0xfe00, 0xff00),
        mp(0xff00, 0xff80),
        mp(0xff80, 0xffc0),
        mp(0xffc0, 0xffe0),
        mp(0xffe0, 0xfff0),
        mp(0xfff0, 0xfff8),
        mp(0xfff8, 0xfffc),
        mp(0xfffc, 0xfffe),
        mp(0xfffe, 0xffff),
    ];
    assert_eq!(sorted(1, 65534), expected);
    validate_continuous(&sorted(1, 65534), 1, 65534);
}

#[test]
fn upstream_port_range_1_to_1023() {
    let expected = vec![
        mp(0x1, 0xffff),
        mp(0x2, 0xfffe),
        mp(0x4, 0xfffc),
        mp(0x8, 0xfff8),
        mp(0x10, 0xfff0),
        mp(0x20, 0xffe0),
        mp(0x40, 0xffc0),
        mp(0x80, 0xff80),
        mp(0x100, 0xff00),
        mp(0x200, 0xfe00),
    ];
    assert_eq!(sorted(1, 1023), expected);
    validate_continuous(&sorted(1, 1023), 1, 1023);
}

#[test]
fn upstream_port_range_0_to_1023_single_prefix() {
    assert_eq!(sorted(0, 1023), vec![mp(0, 0xfc00)]);
}

#[test]
fn upstream_port_range_1024_to_65535() {
    let expected = vec![
        mp(0x400, 0xfc00),
        mp(0x800, 0xf800),
        mp(0x1000, 0xf000),
        mp(0x2000, 0xe000),
        mp(0x4000, 0xc000),
        mp(0x8000, 0x8000),
    ];
    assert_eq!(sorted(1024, 65535), expected);
    validate_continuous(&sorted(1024, 65535), 1024, 65535);
}

#[test]
fn upstream_port_range_10000_to_20000() {
    let expected = vec![
        mp(0x2710, 0xfff0),
        mp(0x2720, 0xffe0),
        mp(0x2740, 0xffc0),
        mp(0x2780, 0xff80),
        mp(0x2800, 0xf800),
        mp(0x3000, 0xf000),
        mp(0x4000, 0xf800),
        mp(0x4800, 0xfc00),
        mp(0x4c00, 0xfe00),
        mp(0x4e00, 0xffe0),
        mp(0x4e20, 0xffff),
    ];
    assert_eq!(sorted(10000, 20000), expected);
    validate_continuous(&sorted(10000, 20000), 10000, 20000);
}

#[test]
fn upstream_port_range_1000_to_1999() {
    let expected = vec![
        mp(0x3e8, 0xfff8),
        mp(0x3f0, 0xfff0),
        mp(0x400, 0xfe00),
        mp(0x600, 0xff00),
        mp(0x700, 0xff80),
        mp(0x780, 0xffc0),
        mp(0x7c0, 0xfff0),
    ];
    assert_eq!(sorted(1000, 1999), expected);
    validate_continuous(&sorted(1000, 1999), 1000, 1999);
}

#[test]
fn upstream_port_range_0_to_1() {
    assert_eq!(sorted(0, 1), vec![mp(0, 0xfffe)]);
}

#[test]
fn upstream_port_range_16_to_31_single_prefix() {
    assert_eq!(sorted(16, 31), vec![mp(0x10, 0xfff0)]);
}

#[test]
fn upstream_port_range_high_65280_to_65535() {
    assert_eq!(sorted(0xff00, 0xffff), vec![mp(0xff00, 0xff00)]);
}

#[test]
fn upstream_port_range_full_0_to_65535_is_wildcard() {
    assert_eq!(sorted(0, 0xffff), vec![mp(0x0, 0x0000)]);
}

#[test]
fn upstream_port_range_1_to_7() {
    let expected = vec![mp(0x1, 0xffff), mp(0x2, 0xfffe), mp(0x4, 0xfffc)];
    assert_eq!(sorted(1, 7), expected);
    validate_continuous(&sorted(1, 7), 1, 7);
}

#[test]
fn upstream_port_range_0_to_7_single_prefix() {
    assert_eq!(sorted(0, 7), vec![mp(0x0, 0xfff8)]);
}

#[test]
fn upstream_port_range_5_to_10() {
    let expected = vec![
        mp(0b0000000000000101, 0b1111111111111111),
        mp(0b0000000000000110, 0b1111111111111110),
        mp(0b0000000000001000, 0b1111111111111110),
        mp(0b0000000000001010, 0b1111111111111111),
    ];
    assert_eq!(sorted(5, 10), expected);
    validate_continuous(&sorted(5, 10), 5, 10);
}

#[test]
fn upstream_port_range_0_to_16() {
    let expected = vec![
        mp(0b0000000000000000, 0b1111111111110000),
        mp(0b0000000000010000, 0b1111111111111111),
    ];
    assert_eq!(sorted(0, 16), expected);
    validate_continuous(&sorted(0, 16), 0, 16);
}

#[test]
fn upstream_port_range_16_to_391() {
    let expected = vec![
        mp(0b0000000000010000, 0b1111111111110000),
        mp(0b0000000000100000, 0b1111111111100000),
        mp(0b0000000001000000, 0b1111111111000000),
        mp(0b0000000010000000, 0b1111111110000000),
        mp(0b0000000100000000, 0b1111111110000000),
        mp(0b0000000110000000, 0b1111111111111000),
    ];
    assert_eq!(sorted(16, 391), expected);
    validate_continuous(&sorted(16, 391), 16, 391);
}

#[test]
fn upstream_port_range_22_to_23() {
    assert_eq!(sorted(22, 23), vec![mp(0b0000000000010110, 0b1111111111111110)]);
}

#[test]
fn upstream_port_range_23_to_24() {
    let expected = vec![
        mp(0b0000000000010111, 0b1111111111111111),
        mp(0b0000000000011000, 0b1111111111111111),
    ];
    assert_eq!(sorted(23, 24), expected);
    validate_continuous(&sorted(23, 24), 23, 24);
}

#[test]
fn upstream_port_range_0_to_0x7fff() {
    assert_eq!(sorted(0, 0x7fff), vec![mp(0x0, 0x8000)]);
}

#[test]
fn upstream_port_range_single_port_256() {
    assert_eq!(sorted(256, 256), vec![mp(0x100, 0xffff)]);
}

#[test]
fn upstream_port_range_single_port_65535() {
    assert_eq!(sorted(65535, 65535), vec![mp(65535, 0xffff)]);
}

#[test]
fn upstream_port_range_32767_to_32768_straddles_high_bit() {
    let expected = vec![mp(0x7fff, 0xffff), mp(0x8000, 0xffff)];
    assert_eq!(sorted(32767, 32768), expected);
    validate_continuous(&sorted(32767, 32768), 32767, 32768);
}

#[test]
fn upstream_port_range_alternating_bits_0x5555_to_0x55d5() {
    let expected = vec![
        mp(0b0101010101010101, 0b1111111111111111),
        mp(0b0101010101010110, 0b1111111111111110),
        mp(0b0101010101011000, 0b1111111111111000),
        mp(0b0101010101100000, 0b1111111111100000),
        mp(0b0101010110000000, 0b1111111111000000),
        mp(0b0101010111000000, 0b1111111111110000),
        mp(0b0101010111010000, 0b1111111111111100),
        mp(0b0101010111010100, 0b1111111111111110),
    ];
    assert_eq!(sorted(0x5555, 0x55d5), expected);
    validate_continuous(&sorted(0x5555, 0x55d5), 0x5555, 0x55d5);
}

#[test]
fn upstream_port_range_0_to_0_is_wildcard() {
    assert_eq!(sorted(0, 0), vec![mp(0, 0)]);
}

#[test]
fn upstream_port_range_65535_to_0_is_single_start_port() {
    // "0" defines no range, so the start port is returned fully masked.
    assert_eq!(sorted(65535, 0), vec![mp(0xffff, 0xffff)]);
}

#[test]
fn upstream_port_range_invalid_start_gt_end_returns_start() {
    // start >= end with non-zero end: ambiguous, upstream returns the
    // start port fully masked.
    assert_eq!(sorted(65535, 1), vec![mp(0xffff, 0xffff)]);
    assert_eq!(sorted(65530, 5), vec![mp(0xfffa, 0xffff)]);
    assert_eq!(sorted(10, 5), vec![mp(0xa, 0xffff)]);
}
