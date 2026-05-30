// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Dual-stack CIDR parsing + family-aware containment.
//
// Cite: kubernetes/kubernetes v1.36.0
//   - k8s.io/utils/net ParseCIDRs (comma-separated dual-stack ClusterCIDR)
//   - pkg/proxy/util/utils.go GetClusterIPByFamily (per-family selection)
//   - pkg/proxy/topology.go / DetectLocalByCIDR (cluster-CIDR local detect)
//
// Upstream kube-proxy is dual-stack: ClusterCIDR is a comma-separated
// "v4cidr,v6cidr" string and a proxier is instantiated per IP family.
// `cave`'s pre-existing `Cidr` type was IPv4-only; `IpCidr` is the
// family-aware variant that closes the IPv6 ClusterCIDR partial.

use cave_kube_proxy::IpCidr;
use std::net::IpAddr;

fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}

#[test]
fn parses_ipv4_cidr() {
    let c = IpCidr::parse("10.0.0.0/24").unwrap();
    assert!(!c.is_ipv6());
    assert_eq!(c.prefix(), 24);
}

#[test]
fn parses_ipv6_cidr() {
    let c = IpCidr::parse("fd00:10:96::/112").unwrap();
    assert!(c.is_ipv6());
    assert_eq!(c.prefix(), 112);
}

#[test]
fn rejects_ipv4_prefix_over_32() {
    assert!(IpCidr::parse("10.0.0.0/33").is_err());
}

#[test]
fn rejects_ipv6_prefix_over_128() {
    assert!(IpCidr::parse("fd00::/129").is_err());
}

#[test]
fn rejects_missing_slash() {
    assert!(IpCidr::parse("10.0.0.0").is_err());
}

#[test]
fn ipv6_contains_matches_inside_prefix() {
    let c = IpCidr::parse("fd00:10:96::/112").unwrap();
    assert!(c.contains(ip("fd00:10:96::abcd")));
    assert!(!c.contains(ip("fd00:10:97::1")));
}

#[test]
fn ipv4_contains_matches_inside_prefix() {
    let c = IpCidr::parse("10.244.0.0/16").unwrap();
    assert!(c.contains(ip("10.244.5.9")));
    assert!(!c.contains(ip("10.245.0.1")));
}

#[test]
fn cross_family_never_contains() {
    // An IPv4 CIDR must never claim an IPv6 address and vice-versa.
    let v4 = IpCidr::parse("0.0.0.0/0").unwrap();
    let v6 = IpCidr::parse("::/0").unwrap();
    assert!(!v4.contains(ip("fd00::1")));
    assert!(!v6.contains(ip("10.0.0.1")));
}

#[test]
fn zero_prefix_matches_whole_family() {
    let v6 = IpCidr::parse("::/0").unwrap();
    assert!(v6.contains(ip("2001:db8::1")));
    assert!(v6.contains(ip("fd00::ffff")));
}

#[test]
fn parse_list_splits_dual_stack_pair() {
    // Upstream ClusterCIDR is comma-separated for dual-stack clusters.
    let cidrs = IpCidr::parse_list("10.244.0.0/16,fd00:10:244::/56").unwrap();
    assert_eq!(cidrs.len(), 2);
    assert!(!cidrs[0].is_ipv6());
    assert!(cidrs[1].is_ipv6());
}

#[test]
fn parse_list_tolerates_whitespace_and_empty_entries() {
    let cidrs = IpCidr::parse_list(" 10.0.0.0/8 , ").unwrap();
    assert_eq!(cidrs.len(), 1);
}

#[test]
fn canonical_roundtrips() {
    let c = IpCidr::parse("fd00:10:96::/112").unwrap();
    assert_eq!(IpCidr::parse(&c.to_string_canonical()).unwrap(), c);
}
