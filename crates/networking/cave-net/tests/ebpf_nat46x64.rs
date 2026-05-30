// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Behavioral-parity tests for Cilium's stateless NAT46/64 address
//! embedding — `bpf/lib/nat_46x64.h` `build_v4_in_v6` /
//! `build_v4_in_v6_rfc6052` / `get_v4_from_v6` / `is_v4_in_v6*`
//! (pinned cilium/cilium v1.19.3, Apache-2.0).
//!
//! Cilium's NAT46x64 gateway maps an IPv4 address into an IPv6 address
//! two ways, both with the IPv4 in the low 32 bits:
//!   * the IPv4-mapped form `::ffff:a.b.c.d` (RFC 4291 §2.5.5.2), used
//!     as the internal datapath sentinel; and
//!   * the RFC 6052 §2.1 well-known prefix `64:ff9b::/96`.
//! `get_v4_from_v6` validates one of those encodings and pulls the
//! low 32 bits back out; an address in neither form is `DROP_INVALID`.

use cave_net::ebpf_sim::program::Ipv4;
use cave_net::ebpf_sim::{
    build_v4_in_v6, build_v4_in_v6_rfc6052, get_v4_from_v6, is_v4_in_v6, is_v4_in_v6_rfc6052,
    V6Addr, RFC6052_WELL_KNOWN_PREFIX,
};

fn v4(a: u8, b: u8, c: u8, d: u8) -> Ipv4 {
    Ipv4::from_octets(a, b, c, d)
}

#[test]
fn rfc6052_well_known_prefix_bytes() {
    // 64:ff9b::/96 — RFC 6052 §2.1.
    assert_eq!(RFC6052_WELL_KNOWN_PREFIX, [0x00, 0x64, 0xff, 0x9b]);
}

#[test]
fn build_v4_mapped_lays_out_ffff_sentinel() {
    // ::ffff:192.0.2.33 → bytes 0..10 = 0, [10]=[11]=0xff, [12..16]=v4.
    let a = build_v4_in_v6(v4(192, 0, 2, 33));
    let mut want = [0u8; 16];
    want[10] = 0xff;
    want[11] = 0xff;
    want[12..16].copy_from_slice(&[192, 0, 2, 33]);
    assert_eq!(a.0, want);
}

#[test]
fn build_rfc6052_lays_out_well_known_prefix() {
    // 64:ff9b::203.0.113.5 → [0..4]=64:ff9b, [4..12]=0, [12..16]=v4.
    let a = build_v4_in_v6_rfc6052(v4(203, 0, 113, 5));
    let mut want = [0u8; 16];
    want[0..4].copy_from_slice(&[0x00, 0x64, 0xff, 0x9b]);
    want[12..16].copy_from_slice(&[203, 0, 113, 5]);
    assert_eq!(a.0, want);
}

#[test]
fn is_v4_in_v6_recognizes_only_the_ffff_sentinel() {
    assert!(is_v4_in_v6(&build_v4_in_v6(v4(10, 0, 0, 1))));
    // The rfc6052 form is NOT the ffff sentinel.
    assert!(!is_v4_in_v6(&build_v4_in_v6_rfc6052(v4(10, 0, 0, 1))));
    // A plain global IPv6 (2001:db8::1) is neither.
    let mut a = [0u8; 16];
    a[0] = 0x20;
    a[1] = 0x01;
    a[2] = 0x0d;
    a[3] = 0xb8;
    a[15] = 1;
    assert!(!is_v4_in_v6(&V6Addr(a)));
}

#[test]
fn is_v4_in_v6_rfc6052_recognizes_only_the_well_known_prefix() {
    assert!(is_v4_in_v6_rfc6052(&build_v4_in_v6_rfc6052(v4(8, 8, 8, 8))));
    assert!(!is_v4_in_v6_rfc6052(&build_v4_in_v6(v4(8, 8, 8, 8))));
}

#[test]
fn get_v4_round_trips_both_encodings() {
    for ip in [v4(192, 168, 1, 1), v4(0, 0, 0, 0), v4(255, 255, 255, 255)] {
        assert_eq!(get_v4_from_v6(&build_v4_in_v6(ip)), Some(ip));
        assert_eq!(get_v4_from_v6(&build_v4_in_v6_rfc6052(ip)), Some(ip));
    }
}

#[test]
fn get_v4_rejects_non_embedded_address() {
    // Pure IPv6 with a non-matching prefix → DROP_INVALID (None).
    let mut a = [0u8; 16];
    a[0] = 0xfe;
    a[1] = 0x80; // link-local
    a[15] = 0x99;
    assert_eq!(get_v4_from_v6(&V6Addr(a)), None);
}

#[test]
fn get_v4_low_32_bits_match_octets_exactly() {
    // The recovered v4 must be the exact low-32-bit octets, not byte-swapped.
    let a = build_v4_in_v6_rfc6052(v4(1, 2, 3, 4));
    assert_eq!(get_v4_from_v6(&a).unwrap().octets(), [1, 2, 3, 4]);
}
