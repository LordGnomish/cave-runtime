// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! LoadBalancer source ranges — parity tests against k8s v1.36.0.
//!
//! Upstream: `pkg/proxy/serviceport.go:53` (LoadBalancerSourceRanges),
//! `pkg/proxy/iptables/proxier.go:1066` (KUBE-FW chain — drop traffic
//! that does not match any allowed source CIDR).

use cave_kube_proxy::{
    Cidr, IptablesProxier, KubeProxyError, Protocol, ServicePortInfo, ServicePortName,
};
use std::net::{IpAddr, Ipv4Addr};

const TENANT: &str = "tenant-acme-prod";

fn svc_with_ranges(ranges: &[&str]) -> ServicePortInfo {
    let mut s = ServicePortInfo::cluster_ip_only(
        TENANT,
        ServicePortName::new("default", "lb", "https"),
        "10.0.0.1".parse::<IpAddr>().unwrap(),
        443,
        Protocol::Tcp,
    );
    s.load_balancer_source_ranges = ranges.iter().map(|r| Cidr::parse(r).unwrap()).collect();
    s.load_balancer_vips = vec!["1.2.3.4".parse().unwrap()];
    s
}

/// Cite: `pkg/proxy/serviceport.go:124` (LoadBalancerSourceRanges) —
/// when the list is empty, every source is allowed (open LB).
#[test]
fn empty_source_ranges_means_allow_all() {
    let s = svc_with_ranges(&[]);
    assert!(s.allowed_by_source_ranges("8.8.8.8".parse().unwrap()));
    assert!(s.allowed_by_source_ranges("203.0.113.1".parse().unwrap()));
}

/// Cite: cave Cidr parser + the upstream `LoadBalancerSourceRanges`
/// shape (slice of `*net.IPNet`). `/24`, `/16`, `/0`, and the literal
/// `0.0.0.0/0` (allow-all) must all be accepted.
#[test]
fn cidr_parser_accepts_canonical_forms_rejects_garbage() {
    assert!(Cidr::parse("10.0.0.0/24").is_ok());
    assert!(Cidr::parse("10.0.0.0/16").is_ok());
    assert!(Cidr::parse("0.0.0.0/0").is_ok());
    assert!(Cidr::parse("10.0.0.0/32").is_ok());

    assert!(matches!(
        Cidr::parse("10.0.0.0/33"),
        Err(KubeProxyError::InvalidCidr(_, _))
    ));
    assert!(matches!(
        Cidr::parse("10.0.0.0"),  // missing /
        Err(KubeProxyError::InvalidCidr(_, _))
    ));
    assert!(matches!(
        Cidr::parse("not.an.ip/24"),
        Err(KubeProxyError::InvalidCidr(_, _))
    ));
}

/// Cite: `pkg/proxy/serviceport.go:124` (LoadBalancerSourceRanges) —
/// `Cidr::contains` mirrors `(*net.IPNet).Contains`. The /0 CIDR
/// matches every IPv4 (open Internet); narrower CIDRs reject outside.
#[test]
fn cidr_contains_matches_only_in_range_ips() {
    let c = Cidr::parse("10.0.0.0/24").unwrap();
    assert!(c.contains("10.0.0.1".parse().unwrap()));
    assert!(c.contains("10.0.0.255".parse().unwrap()));
    assert!(!c.contains("10.0.1.1".parse().unwrap()));
    assert!(!c.contains("11.0.0.1".parse().unwrap()));

    let zero = Cidr::parse("0.0.0.0/0").unwrap();
    assert!(zero.contains(Ipv4Addr::new(8, 8, 8, 8)));
    assert!(zero.contains(Ipv4Addr::new(127, 0, 0, 1)));
}

/// Cite: `pkg/proxy/iptables/proxier.go:1066` (KUBE-FW emission) —
/// the firewall chain emits one allow rule per source range followed
/// by a trailing `KUBE-MARK-DROP` to reject unmatched traffic.
#[test]
fn kube_fw_chain_emits_per_range_accepts_then_terminating_drop() {
    let p = IptablesProxier::new(TENANT);
    let s = svc_with_ranges(&["10.0.0.0/8", "192.168.1.0/24"]);

    let rules = p.build_loadbalancer_firewall_rules(&s).unwrap();
    assert_eq!(rules.len(), 3, "2 allows + 1 trailing drop");
    assert!(rules[0].contains("-s 10.0.0.0/8"));
    assert!(rules[0].contains("-j KUBE-SVC-"));
    assert!(rules[1].contains("-s 192.168.1.0/24"));
    assert!(rules[2].contains("-j KUBE-MARK-DROP"),
        "trailing drop is mandatory — otherwise traffic falls through to ACCEPT");

    // No source ranges → no firewall chain
    let open = svc_with_ranges(&[]);
    assert!(p.build_loadbalancer_firewall_rules(&open).unwrap().is_empty());
}
