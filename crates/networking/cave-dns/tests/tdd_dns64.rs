// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD: CoreDNS v1.14.3 `plugin/dns64/dns64.go` synthesis (RFC 6147).

use cave_dns::plugins::dns64::Dns64Plugin;
use hickory_proto::rr::RecordType;
use std::net::{Ipv4Addr, Ipv6Addr};

#[test]
fn to6_well_known_prefix() {
    // RFC 6052: 64:ff9b::/96 + 192.0.2.1 => 64:ff9b::c000:201
    let p = Dns64Plugin::default();
    let v6 = p.to6(Ipv4Addr::new(192, 0, 2, 1));
    assert_eq!(v6, "64:ff9b::c000:201".parse::<Ipv6Addr>().unwrap());
}

#[test]
fn to6_maps_full_octets() {
    let p = Dns64Plugin::default();
    let v6 = p.to6(Ipv4Addr::new(10, 0, 0, 1));
    assert_eq!(v6, "64:ff9b::a00:1".parse::<Ipv6Addr>().unwrap());
}

#[test]
fn intercepts_only_aaaa() {
    // requestShouldIntercept: AAAA over IPv6 (allow_ipv4=false) only.
    assert!(Dns64Plugin::request_should_intercept(RecordType::AAAA, false, false));
    assert!(!Dns64Plugin::request_should_intercept(RecordType::A, false, false));
    // IPv4 client without allow_ipv4 is not intercepted.
    assert!(!Dns64Plugin::request_should_intercept(RecordType::AAAA, true, false));
    // ...unless allow_ipv4 is set.
    assert!(Dns64Plugin::request_should_intercept(RecordType::AAAA, true, true));
}

#[test]
fn response_needs_dns64_only_without_aaaa_and_not_nxdomain() {
    // responseShouldDNS64: no AAAA + not NameError => translate.
    assert!(Dns64Plugin::response_should_dns64(false, false));
    // NameError (NXDOMAIN) => pass through.
    assert!(!Dns64Plugin::response_should_dns64(true, false));
    // Already has AAAA => pass through.
    assert!(!Dns64Plugin::response_should_dns64(false, true));
}

#[test]
fn synthesize_maps_all_a_answers() {
    let p = Dns64Plugin::default();
    let out = p.synthesize_aaaa(&[Ipv4Addr::new(192, 0, 2, 1), Ipv4Addr::new(192, 0, 2, 2)]);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0], "64:ff9b::c000:201".parse::<Ipv6Addr>().unwrap());
    assert_eq!(out[1], "64:ff9b::c000:202".parse::<Ipv6Addr>().unwrap());
}
