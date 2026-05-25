// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! nftables proxier — preferred datapath on Linux ≥ 7.1.
//!
//! Upstream: `pkg/proxy/nftables/proxier.go` k8s v1.36.0.
//! cave drops the userspace mode entirely (greenfield, no upgrade path).

use cave_kube_proxy::{
    EndpointInfo, EndpointSliceMap, KubeProxyError, NftablesProxier, Protocol, ServicePortInfo,
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

/// Cite: `pkg/proxy/nftables/proxier.go:71` (servicesChain) +
/// `:72` (serviceIPsMap) + `:73` (serviceNodePortsMap) +
/// `:410` (setupNFTables). The scaffold table must declare the chains
/// and both maps with the canonical key types.
#[test]
fn table_scaffold_declares_chains_and_maps() {
    let p = NftablesProxier::new(TENANT);
    let lines = p.build_table_scaffold();
    let blob = lines.join("\n");
    assert!(blob.contains("table inet kube-proxy"));
    assert!(blob.contains("chain services"));
    assert!(blob.contains("chain nodeports"));
    assert!(blob.contains("map service-ips"));
    assert!(blob.contains("map service-nodeports"));
    assert!(blob.contains("ipv4_addr . inet_proto . inet_service : verdict"));
}

/// Cite: `pkg/proxy/nftables/proxier.go:637` (services chain VMAP
/// dispatch) — every Service contributes one element to the
/// `service-ips` map: `(clusterIP, proto, port) → goto svc-XXXX`.
#[test]
fn service_ips_map_one_entry_per_clusterip_service() {
    let p = NftablesProxier::new(TENANT);
    let services = vec![
        svc("api", "10.0.0.1", 80, None),
        svc("db", "10.0.0.2", 5432, None),
        svc("headless", "0.0.0.0", 80, None), // skipped
    ];
    let entries = p.build_service_ips_map_entries(&services).unwrap();
    assert_eq!(entries.len(), 2, "headless service skipped");
    assert!(
        entries
            .iter()
            .any(|e| e.contains("10.0.0.1 . tcp . 80 : goto svc-"))
    );
    assert!(
        entries
            .iter()
            .any(|e| e.contains("10.0.0.2 . tcp . 5432 : goto svc-"))
    );
}

/// Cite: `pkg/proxy/nftables/proxier.go` `service-nodeports` map
/// rendering — only Services with a NodePort contribute an entry, and
/// the key is the (proto, port) tuple matching upstream knftables Map
/// definitions.
#[test]
fn service_nodeports_map_only_for_nodeport_services() {
    let p = NftablesProxier::new(TENANT);
    let services = vec![
        svc("api", "10.0.0.1", 80, Some(31080)),
        svc("db", "10.0.0.2", 5432, None),
        svc("ui", "10.0.0.3", 8080, Some(31443)),
    ];
    let entries = p.build_service_nodeports_map_entries(&services).unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().any(|e| e.starts_with("tcp . 31080 :")));
    assert!(entries.iter().any(|e| e.starts_with("tcp . 31443 :")));
}

/// Cite: `pkg/proxy/nftables/proxier.go:381–:382` — jumps installed
/// from `nat prerouting` and `nat output` into the `services` chain
/// so traffic from both ingress directions hits the dispatcher.
#[test]
fn jump_rules_cover_prerouting_and_output() {
    let p = NftablesProxier::new(TENANT);
    let rules = p.build_jump_rules();
    assert_eq!(rules.len(), 2);
    assert!(rules.iter().any(|r| r.contains("prerouting jump services")));
    assert!(rules.iter().any(|r| r.contains("output     jump services")));
}

/// Cite: `pkg/proxy/nftables/proxier.go` per-service chain — the
/// random LB uses nftables `numgen random mod N == i` for the first
/// N-1 endpoints and an unconditional DNAT for the last (mirrors the
/// iptables 1/(n-i) decay). When N=0 the chain rejects.
#[test]
fn svc_chain_emits_numgen_random_lb_and_rejects_when_no_endpoints() {
    let p = NftablesProxier::new(TENANT);
    let s = svc("api", "10.0.0.1", 80, None);

    // Empty endpoints → reject
    let empty = EndpointSliceMap::new(TENANT);
    let rules = p.build_svc_chain_rules(&s, &empty).unwrap();
    assert!(rules.iter().any(|r| r.contains("reject with icmp")));

    // 3 endpoints → 2 numgen rules + 1 trailing DNAT
    let mut eps = EndpointSliceMap::new(TENANT);
    eps.upsert_slice(
        s.name.clone(),
        "slice-1",
        vec![
            EndpointInfo::ready("10.1.0.1".parse().unwrap(), 8080),
            EndpointInfo::ready("10.1.0.2".parse().unwrap(), 8080),
            EndpointInfo::ready("10.1.0.3".parse().unwrap(), 8080),
        ],
    );
    let rules = p.build_svc_chain_rules(&s, &eps).unwrap();
    let body: Vec<&String> = rules
        .iter()
        .filter(|r| r.contains("dnat to") || r.contains("numgen"))
        .collect();
    assert_eq!(body.len(), 3);
    assert!(body[0].contains("numgen random mod 3 == 0"));
    assert!(body[1].contains("numgen random mod 3 == 1"));
    assert!(
        !body[2].contains("numgen"),
        "last endpoint is unconditional"
    );

    // Cross-tenant safety
    let foreign_eps = EndpointSliceMap::new("tenant-other");
    let err = p.build_svc_chain_rules(&s, &foreign_eps).unwrap_err();
    assert!(matches!(err, KubeProxyError::CrossTenantDenied { .. }));
}
