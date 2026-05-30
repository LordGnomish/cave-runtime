// SPDX-License-Identifier: AGPL-3.0-or-later
//! IXFR tests (RFC 1995 §4; closes the transfer partial).
use cave_dns::ixfr::{soa_serial, IxfrDelta};
use hickory_proto::rr::rdata::{A, SOA};
use hickory_proto::rr::{Name, RData, Record};
use std::net::Ipv4Addr;
use std::str::FromStr;

fn name(s: &str) -> Name { Name::from_str(s).unwrap() }
fn a_rec(n: &str, ip: &str) -> Record {
    Record::from_rdata(name(n), 3600, RData::A(A(ip.parse::<Ipv4Addr>().unwrap())))
}
fn soa(serial: u32) -> Record {
    let rdata = SOA::new(name("ns.example.org."), name("admin.example.org."), serial, 7200, 3600, 1_209_600, 3600);
    Record::from_rdata(name("example.org."), 3600, RData::SOA(rdata))
}

#[test]
fn parses_soa_serial() {
    assert_eq!(soa_serial(&soa(2021010101)), Some(2021010101));
    assert_eq!(soa_serial(&a_rec("x.", "1.2.3.4")), None);
}
#[test]
fn computes_additions_and_deletions() {
    let old = vec![soa(1), a_rec("a.example.org.", "10.0.0.1"), a_rec("b.example.org.", "10.0.0.2")];
    let new = vec![soa(2), a_rec("a.example.org.", "10.0.0.1"), a_rec("c.example.org.", "10.0.0.3")];
    let delta = IxfrDelta::compute(&old, &new).unwrap();
    assert_eq!(soa_serial(&delta.old_soa), Some(1));
    assert_eq!(soa_serial(&delta.new_soa), Some(2));
    assert_eq!(delta.deletions, vec![a_rec("b.example.org.", "10.0.0.2")]);
    assert_eq!(delta.additions, vec![a_rec("c.example.org.", "10.0.0.3")]);
}
#[test]
fn to_wire_follows_rfc1995_sequence() {
    let old = vec![soa(1), a_rec("b.example.org.", "10.0.0.2")];
    let new = vec![soa(2), a_rec("c.example.org.", "10.0.0.3")];
    let wire = IxfrDelta::compute(&old, &new).unwrap().to_wire();
    assert_eq!(wire.len(), 6);
    assert_eq!(soa_serial(&wire[0]), Some(2));
    assert_eq!(soa_serial(&wire[1]), Some(1));
    assert_eq!(wire[2], a_rec("b.example.org.", "10.0.0.2"));
    assert_eq!(soa_serial(&wire[3]), Some(2));
    assert_eq!(wire[4], a_rec("c.example.org.", "10.0.0.3"));
    assert_eq!(soa_serial(&wire[5]), Some(2));
}
#[test]
fn no_change_yields_single_soa() {
    let old = vec![soa(5), a_rec("a.example.org.", "10.0.0.1")];
    let new = vec![soa(5), a_rec("a.example.org.", "10.0.0.1")];
    let wire = IxfrDelta::compute(&old, &new).unwrap().to_wire();
    assert_eq!(wire.len(), 1);
    assert_eq!(soa_serial(&wire[0]), Some(5));
}
#[test]
fn missing_soa_is_an_error() {
    assert!(IxfrDelta::compute(&[a_rec("a.example.org.", "10.0.0.1")], &[soa(2)]).is_err());
}
#[test]
fn axfr_fallback_when_old_serial_newer() {
    let delta = IxfrDelta::compute(&[soa(9)], &[soa(2)]).unwrap();
    assert!(delta.needs_axfr_fallback());
}
