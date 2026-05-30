// SPDX-License-Identifier: AGPL-3.0-or-later
//! acl plugin tests (CoreDNS v1.14.3 plugin/acl).
use cave_dns::acl::{Acl, AclAction, AclRule, IpCidr};
use hickory_proto::rr::RecordType;
use std::net::IpAddr;

fn ip(s: &str) -> IpAddr { s.parse().unwrap() }

#[test]
fn cidr_contains_ipv4() {
    let net = IpCidr::parse("192.168.1.0/24").unwrap();
    assert!(net.contains(ip("192.168.1.5")));
    assert!(net.contains(ip("192.168.1.255")));
    assert!(!net.contains(ip("192.168.2.1")));
}
#[test]
fn cidr_contains_ipv6() {
    let net = IpCidr::parse("2001:db8::/32").unwrap();
    assert!(net.contains(ip("2001:db8::1")));
    assert!(!net.contains(ip("2001:dba::1")));
}
#[test]
fn cidr_host_route_slash_32() {
    let net = IpCidr::parse("10.0.0.7/32").unwrap();
    assert!(net.contains(ip("10.0.0.7")));
    assert!(!net.contains(ip("10.0.0.8")));
}
#[test]
fn cidr_rejects_malformed() {
    assert!(IpCidr::parse("not-a-cidr").is_err());
    assert!(IpCidr::parse("10.0.0.0/40").is_err());
}
#[test]
fn block_rule_matches_by_source() {
    let acl = Acl::new(vec![AclRule::new(AclAction::Block, vec![IpCidr::parse("10.0.0.0/8").unwrap()], vec![])]);
    assert_eq!(acl.evaluate(ip("10.1.2.3"), RecordType::A), AclAction::Block);
    assert_eq!(acl.evaluate(ip("192.0.2.1"), RecordType::A), AclAction::Allow);
}
#[test]
fn qtype_scoped_rule() {
    let acl = Acl::new(vec![AclRule::new(AclAction::Filter, vec![IpCidr::parse("0.0.0.0/0").unwrap()], vec![RecordType::AAAA])]);
    assert_eq!(acl.evaluate(ip("8.8.8.8"), RecordType::AAAA), AclAction::Filter);
    assert_eq!(acl.evaluate(ip("8.8.8.8"), RecordType::A), AclAction::Allow);
}
#[test]
fn first_matching_rule_wins() {
    let acl = Acl::new(vec![
        AclRule::new(AclAction::Allow, vec![IpCidr::parse("10.0.0.1/32").unwrap()], vec![]),
        AclRule::new(AclAction::Block, vec![IpCidr::parse("10.0.0.0/8").unwrap()], vec![]),
    ]);
    assert_eq!(acl.evaluate(ip("10.0.0.1"), RecordType::A), AclAction::Allow);
    assert_eq!(acl.evaluate(ip("10.0.0.2"), RecordType::A), AclAction::Block);
}
#[test]
fn default_is_allow() {
    assert_eq!(Acl::new(vec![]).evaluate(ip("1.2.3.4"), RecordType::MX), AclAction::Allow);
}
#[test]
fn action_response_code_matches_upstream() {
    assert_eq!(AclAction::Allow.response_code(), None);
    assert_eq!(AclAction::Block.response_code(), Some((5, false)));
    assert_eq!(AclAction::Filter.response_code(), Some((0, true)));
}
