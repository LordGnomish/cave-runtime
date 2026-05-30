// SPDX-License-Identifier: AGPL-3.0-or-later
//! dns64 plugin tests (CoreDNS v1.14.3 plugin/dns64; RFC 6052 §2.4 vectors).
use cave_dns::dns64::Dns64;
use hickory_proto::rr::rdata::{A, AAAA, TXT};
use hickory_proto::rr::{Name, RData, Record};
use std::net::{Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

fn v4(s: &str) -> Ipv4Addr { s.parse().unwrap() }
fn v6(s: &str) -> Ipv6Addr { s.parse().unwrap() }
fn name(s: &str) -> Name { Name::from_str(s).unwrap() }

#[test]
fn rfc6052_section_2_4_vectors() {
    let ip = v4("192.0.2.33");
    let cases = [
        ("2001:db8::", 32u8, "2001:db8:c000:221::"),
        ("2001:db8:100::", 40, "2001:db8:1c0:2:21::"),
        ("2001:db8:122::", 48, "2001:db8:122:c000:2:2100::"),
        ("2001:db8:122:300::", 56, "2001:db8:122:3c0:0:221::"),
        ("2001:db8:122:344::", 64, "2001:db8:122:344:c0:2:2100::"),
        ("2001:db8:122:344::", 96, "2001:db8:122:344::c000:221"),
    ];
    for (prefix, len, expect) in cases {
        let d = Dns64::new(v6(prefix), len).unwrap();
        assert_eq!(d.synthesize(ip), v6(expect), "prefix {prefix}/{len}");
    }
}
#[test]
fn well_known_prefix_is_64_ff9b_96() {
    let d = Dns64::well_known();
    assert_eq!(d.synthesize(v4("192.0.2.1")), v6("64:ff9b::c000:201"));
    assert_eq!(d.synthesize(v4("8.8.8.8")), v6("64:ff9b::808:808"));
}
#[test]
fn rejects_invalid_prefix_length() {
    assert!(Dns64::new(v6("2001:db8::"), 50).is_err());
    assert!(Dns64::new(v6("2001:db8::"), 0).is_err());
    assert!(Dns64::new(v6("2001:db8::"), 128).is_err());
}
#[test]
fn synthesize_records_maps_a_to_aaaa() {
    let d = Dns64::well_known();
    let a = Record::from_rdata(name("example.org."), 300, RData::A(A(v4("192.0.2.1"))));
    let out = d.synthesize_records(std::slice::from_ref(&a));
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].ttl(), 300);
    assert_eq!(out[0].name(), &name("example.org."));
    match out[0].data() {
        Some(RData::AAAA(aaaa)) => assert_eq!(aaaa.0, v6("64:ff9b::c000:201")),
        other => panic!("expected AAAA, got {other:?}"),
    }
}
#[test]
fn synthesize_records_skips_non_a() {
    let d = Dns64::well_known();
    let txt = Record::from_rdata(name("x."), 60, RData::TXT(TXT::new(vec!["hello".to_string()])));
    assert!(d.synthesize_records(std::slice::from_ref(&txt)).is_empty());
}
#[test]
fn should_synthesize_only_when_no_aaaa() {
    let d = Dns64::well_known();
    assert!(d.should_synthesize(false));
    assert!(!d.should_synthesize(true));
}
