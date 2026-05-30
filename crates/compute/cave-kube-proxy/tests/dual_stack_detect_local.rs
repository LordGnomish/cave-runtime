// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Dual-stack DetectLocalByCIDR + per-family ClusterCIDR selection.
//
// Cite: kubernetes/kubernetes v1.36.0
//   - pkg/proxy/topology.go DetectLocalByCIDR (endpoint-local detection)
//   - pkg/proxy/util/utils.go GetClusterIPByFamily (per-family selection)
//   - pkg/proxy/apis/config/types.go:107 (ClusterCIDR — dual-stack)
//
// Closes the IPv6 ClusterCIDR plumbing partial: the v6 cluster CIDR was a
// dead `Option<String>` hook; it is now a parsed `IpCidr` consumed by the
// family-aware local-endpoint detector.

use cave_kube_proxy::ProxyConfig;
use std::net::IpAddr;

fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}

#[test]
fn with_cluster_cidrs_parses_dual_stack_pair() {
    let c = ProxyConfig::default()
        .with_cluster_cidrs("10.244.0.0/16,fd00:10:244::/56")
        .unwrap();
    assert!(c.cluster_cidr_for_family(false).is_some(), "v4 family present");
    assert!(c.cluster_cidr_for_family(true).is_some(), "v6 family present");
}

#[test]
fn with_cluster_cidrs_accepts_v6_only() {
    let c = ProxyConfig::default()
        .with_cluster_cidrs("fd00:10:244::/56")
        .unwrap();
    assert!(c.cluster_cidr_for_family(false).is_none());
    assert!(c.cluster_cidr_for_family(true).is_some());
}

#[test]
fn detect_local_matches_v6_endpoint_in_cluster_cidr() {
    let c = ProxyConfig::default()
        .with_cluster_cidrs("10.244.0.0/16,fd00:10:244::/56")
        .unwrap();
    assert!(c.detect_local_by_cidr(ip("fd00:10:244::5")));
    assert!(!c.detect_local_by_cidr(ip("fd00:10:99::5")));
}

#[test]
fn detect_local_matches_v4_endpoint_in_cluster_cidr() {
    let c = ProxyConfig::default()
        .with_cluster_cidrs("10.244.0.0/16,fd00:10:244::/56")
        .unwrap();
    assert!(c.detect_local_by_cidr(ip("10.244.7.3")));
    assert!(!c.detect_local_by_cidr(ip("10.99.0.1")));
}

#[test]
fn detect_local_is_false_when_no_cidr_for_family() {
    // v4-only config must not claim a v6 endpoint as local.
    let c = ProxyConfig::default().with_cluster_cidrs("10.244.0.0/16").unwrap();
    assert!(!c.detect_local_by_cidr(ip("fd00:10:244::5")));
}

#[test]
fn detect_local_is_false_when_unconfigured() {
    let c = ProxyConfig::default();
    assert!(!c.detect_local_by_cidr(ip("10.244.7.3")));
    assert!(!c.detect_local_by_cidr(ip("fd00:10:244::5")));
}

#[test]
fn with_cluster_cidrs_rejects_garbage() {
    assert!(ProxyConfig::default().with_cluster_cidrs("not-a-cidr").is_err());
}
