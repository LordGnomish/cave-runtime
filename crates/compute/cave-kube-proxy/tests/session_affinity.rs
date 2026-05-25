// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Session affinity (ClientIP timeout) — parity tests against k8s v1.36.0.
//!
//! Upstream: `pkg/proxy/serviceport.go:43` (SessionAffinityType),
//! `:115` (StickyMaxAgeSeconds), `:186-:188` (newBaseServiceInfo —
//! reads `SessionAffinityConfig.ClientIP.TimeoutSeconds`).

use cave_kube_proxy::{
    EndpointInfo, EndpointSliceMap, IptablesProxier, NftablesProxier, Protocol, ServicePortInfo,
    ServicePortName, SessionAffinity,
};
use std::net::IpAddr;

const TENANT: &str = "tenant-acme-prod";

fn svc_with_affinity(ttl: Option<u32>) -> ServicePortInfo {
    let mut s = ServicePortInfo::cluster_ip_only(
        TENANT,
        ServicePortName::new("default", "api", "http"),
        "10.0.0.1".parse::<IpAddr>().unwrap(),
        80,
        Protocol::Tcp,
    );
    s.session_affinity = SessionAffinity::ClientIp;
    s.sticky_max_age_seconds = ttl;
    s
}

fn endpoints_for(name: &ServicePortName) -> EndpointSliceMap {
    let mut m = EndpointSliceMap::new(TENANT);
    m.upsert_slice(
        name.clone(),
        "slice-1",
        vec![EndpointInfo::ready("10.1.0.1".parse().unwrap(), 8080)],
    );
    m
}

/// Cite: `pkg/proxy/serviceport.go:43` (SessionAffinityType) — when
/// `session_affinity == None`, no affinity-pinning rule is emitted by
/// the iptables proxier.
#[test]
fn no_affinity_emits_no_recent_rule() {
    let p = IptablesProxier::new(TENANT);
    let s = ServicePortInfo::cluster_ip_only(
        TENANT,
        ServicePortName::new("default", "api", "http"),
        "10.0.0.1".parse::<IpAddr>().unwrap(),
        80,
        Protocol::Tcp,
    );
    let eps = endpoints_for(&s.name);
    let rules = p.build_svc_chain_rules(&s, &eps).unwrap();
    assert!(
        rules.iter().all(|r| !r.contains("--rcheck")),
        "no affinity → no -m recent rule"
    );
}

/// Cite: `pkg/proxy/serviceport.go:115` (StickyMaxAgeSeconds) +
/// `:188` — when affinity=ClientIP and TTL is supplied, the iptables
/// proxier emits a `recent --rcheck --seconds <ttl>` guard before the
/// random LB rules.
#[test]
fn iptables_emits_recent_rcheck_for_clientip_affinity() {
    let p = IptablesProxier::new(TENANT);
    let s = svc_with_affinity(Some(7200));
    let eps = endpoints_for(&s.name);
    let rules = p.build_svc_chain_rules(&s, &eps).unwrap();
    let affinity = rules
        .iter()
        .find(|r| r.contains("--rcheck"))
        .expect("affinity rule emitted");
    assert!(affinity.contains("--seconds 7200"));
    assert!(affinity.contains("-AFFINITY"));
}

/// Cite: `pkg/proxy/serviceport.go:181-:188` — when the apiserver
/// guarantees `SessionAffinityConfig.ClientIP.TimeoutSeconds`, the
/// kube-proxy default fallback is the upstream constant 10800s (3h).
/// Cave honours the same default when `sticky_max_age_seconds = None`.
#[test]
fn iptables_falls_back_to_10800s_default_when_ttl_unset() {
    let p = IptablesProxier::new(TENANT);
    let s = svc_with_affinity(None);
    let eps = endpoints_for(&s.name);
    let rules = p.build_svc_chain_rules(&s, &eps).unwrap();
    let affinity = rules.iter().find(|r| r.contains("--rcheck")).unwrap();
    assert!(
        affinity.contains("--seconds 10800"),
        "default sticky TTL = 3 hours"
    );
}

/// Cite: `pkg/proxy/nftables/proxier.go` per-service affinity — the
/// nftables proxier expresses affinity via `ip saddr @<chain>-affinity
/// timeout <ttl>s`, mirroring the iptables `-m recent` semantics.
#[test]
fn nftables_emits_saddr_set_with_timeout_for_clientip_affinity() {
    let p = NftablesProxier::new(TENANT);
    let s = svc_with_affinity(Some(900));
    let eps = endpoints_for(&s.name);
    let rules = p.build_svc_chain_rules(&s, &eps).unwrap();
    let line = rules
        .iter()
        .find(|r| r.contains("ip saddr @"))
        .expect("nftables affinity rule emitted");
    assert!(line.contains("-affinity"));
    assert!(line.contains("timeout 900s"));
}
