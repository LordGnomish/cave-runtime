// SPDX-License-Identifier: AGPL-3.0-or-later
//! iptables proxier — KUBE-SERVICES + KUBE-NODEPORTS chain emission.
//!
//! Upstream: `pkg/proxy/iptables/proxier.go` k8s v1.36.0.

use cave_kube_proxy::{
    EndpointInfo, EndpointSliceMap, IptablesProxier, KubeProxyError, Protocol, ServicePortInfo,
    ServicePortName,
};
use std::net::IpAddr;

const TENANT: &str = "tenant-acme-prod";

fn svc(name: &str, ip: &str, port: u16, node_port: Option<u16>) -> ServicePortInfo {
    let mut s = ServicePortInfo::cluster_ip_only(
        TENANT,
        ServicePortName::new("default", name, "http"),
        ip.parse::<IpAddr>().unwrap(),
        port,
        Protocol::Tcp,
    );
    s.node_port = node_port;
    s
}

/// Cite: `pkg/proxy/iptables/proxier.go:55` (kubeServicesChain) +
/// `:939–:989` (per-service ClusterIP DNAT entry). Each non-headless
/// Service contributes one `-A KUBE-SERVICES -d <vip> ... -j KUBE-SVC-XXXX` rule.
#[test]
fn kube_services_emits_one_rule_per_clusterip_service() {
    let p = IptablesProxier::new(TENANT);
    let services = vec![
        svc("api", "10.0.0.1", 80, None),
        svc("db",  "10.0.0.2", 5432, None),
    ];
    let rules = p.build_kube_services_rules(&services).unwrap();
    assert_eq!(rules.len(), 2);
    assert!(rules[0].contains("-A KUBE-SERVICES -d 10.0.0.1 -p tcp --dport 80"));
    assert!(rules[0].contains("-j KUBE-SVC-"));
    assert!(rules[1].contains("-d 10.0.0.2") && rules[1].contains("--dport 5432"));
}

/// Cite: `pkg/proxy/util/utils.go:55` (ShouldSkipService) — headless
/// services (no ClusterIP) MUST NOT contribute to KUBE-SERVICES.
#[test]
fn headless_services_skipped_from_kube_services() {
    let p = IptablesProxier::new(TENANT);
    let services = vec![
        svc("headless", "0.0.0.0", 80, None),  // skip sentinel
        svc("api",      "10.0.0.1", 80, None),
    ];
    let rules = p.build_kube_services_rules(&services).unwrap();
    assert_eq!(rules.len(), 1);
    assert!(rules[0].contains("-d 10.0.0.1"));
}

/// Cite: `pkg/proxy/iptables/proxier.go:61` (kubeNodePortsChain) +
/// `:1031–:1066`. Only Services with a NodePort contribute rules to
/// the KUBE-NODEPORTS chain.
#[test]
fn kube_nodeports_emits_only_for_nodeport_services() {
    let p = IptablesProxier::new(TENANT);
    let services = vec![
        svc("api", "10.0.0.1", 80, Some(31080)),
        svc("db",  "10.0.0.2", 5432, None),       // no NodePort
        svc("ui",  "10.0.0.3", 8080, Some(31443)),
    ];
    let rules = p.build_kube_nodeports_rules(&services).unwrap();
    assert_eq!(rules.len(), 2, "db has no NodePort → no KUBE-NODEPORTS rule");
    assert!(rules.iter().any(|r| r.contains("--dport 31080")));
    assert!(rules.iter().any(|r| r.contains("--dport 31443")));
}

/// Cite: `pkg/proxy/iptables/proxier.go:1301–:1326` — the trailing
/// jump from KUBE-SERVICES into KUBE-NODEPORTS for LOCAL traffic.
/// Must be emitted as the LAST rule so ClusterIP rules win.
#[test]
fn kube_services_to_nodeports_jump_uses_addrtype_local() {
    let p = IptablesProxier::new(TENANT);
    let line = p.build_kube_services_nodeports_terminator();
    assert!(line.contains("KUBE-SERVICES"));
    assert!(line.contains("KUBE-NODEPORTS"));
    assert!(line.contains("--dst-type LOCAL"),
        "addrtype LOCAL match required so only host-bound traffic hits NodePort");
}

/// Cite: `pkg/proxy/iptables/proxier.go` per-service SVC chain — the
/// random LB uses iptables `--mode random --probability` with a
/// 1/(n-i) decay so each ready endpoint gets uniform 1/n probability.
#[test]
fn svc_chain_emits_one_dnat_per_ready_endpoint_with_decaying_probability() {
    let p = IptablesProxier::new(TENANT);
    let s = svc("api", "10.0.0.1", 80, None);
    let mut eps = EndpointSliceMap::new(TENANT);
    eps.upsert_slice(s.name.clone(), "slice-1", vec![
        EndpointInfo::ready("10.1.0.1".parse().unwrap(), 8080),
        EndpointInfo::ready("10.1.0.2".parse().unwrap(), 8080),
        EndpointInfo::ready("10.1.0.3".parse().unwrap(), 8080),
    ]);

    let rules = p.build_svc_chain_rules(&s, &eps).unwrap();
    assert_eq!(rules.len(), 3, "one DNAT per endpoint");
    // 1/3, 1/2, 1.0 (last endpoint always taken if reached)
    assert!(rules[0].contains("--probability 0.3333"));
    assert!(rules[1].contains("--probability 0.5000"));
    assert!(rules[2].contains("--probability 1.0000"));
    assert!(rules.iter().all(|r| r.contains("DNAT")));

    // Cross-tenant safety
    let foreign_eps = EndpointSliceMap::new("tenant-other");
    let err = p.build_svc_chain_rules(&s, &foreign_eps).unwrap_err();
    assert!(matches!(err, KubeProxyError::CrossTenantDenied { .. }));
}
