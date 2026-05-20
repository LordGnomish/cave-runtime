// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Edge coverage for cave-net — NetState data plane, models, Cilium types.

use cave_net::cilium::types::{Cite, TenantId, UPSTREAM_REPO, UPSTREAM_VERSION};
use cave_net::dataplane::NetState;
use cave_net::models::{
    EgressRule, Endpoint, FlowDirection, FlowRecord, FlowVerdict, IngressRule, NetworkPolicy,
    PeerSelector, PolicyPort, PolicyType, Protocol, ServiceEntry, ServicePort, ServiceType,
};
use chrono::Utc;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use uuid::Uuid;

fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(a, b, c, d))
}

fn labels(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

// ---------------------------------------------------------------------------
// NetState — allocation
// ---------------------------------------------------------------------------

#[test]
fn netstate_default_cidrs_match_k8s_defaults() {
    let s = NetState::new();
    assert_eq!(s.pod_cidr, "10.0.0.0/16");
    assert_eq!(s.service_cidr, "10.96.0.0/12");
}

#[test]
fn netstate_default_equiv_new() {
    let a = NetState::default();
    let b = NetState::new();
    assert_eq!(a.pod_cidr, b.pod_cidr);
    assert_eq!(a.service_cidr, b.service_cidr);
}

#[test]
fn netstate_allocate_pod_ip_assigns_distinct() {
    let s = NetState::new();
    let p1 = s.allocate_pod_ip("a", "ns", "node", HashMap::new());
    let p2 = s.allocate_pod_ip("b", "ns", "node", HashMap::new());
    assert_ne!(p1.pod_ip, p2.pod_ip);
    assert_eq!(s.pods.len(), 2);
}

#[test]
fn netstate_release_pod_ip_decreases_count() {
    let s = NetState::new();
    s.allocate_pod_ip("a", "ns", "node", HashMap::new());
    assert_eq!(s.pods.len(), 1);
    s.release_pod_ip("a", "ns");
    assert_eq!(s.pods.len(), 0);
}

#[test]
fn netstate_release_unknown_pod_is_noop() {
    let s = NetState::new();
    s.release_pod_ip("ghost", "ns"); // should not panic
    assert_eq!(s.pods.len(), 0);
}

#[test]
fn netstate_allocate_carries_labels_and_node() {
    let s = NetState::new();
    let l = labels(&[("app", "nginx"), ("tier", "web")]);
    let p = s.allocate_pod_ip("nginx", "ns", "worker-1", l.clone());
    assert_eq!(p.node_name, "worker-1");
    assert_eq!(p.labels, l);
    assert_eq!(p.namespace, "ns");
}

// ---------------------------------------------------------------------------
// NetState — services + endpoints
// ---------------------------------------------------------------------------

fn svc(name: &str, ns: &str) -> ServiceEntry {
    ServiceEntry {
        name: name.into(),
        namespace: ns.into(),
        cluster_ip: ip(10, 96, 0, 1),
        service_type: ServiceType::ClusterIP,
        ports: vec![ServicePort {
            name: Some("http".into()),
            port: 80,
            target_port: 8080,
            protocol: Protocol::TCP,
            node_port: None,
        }],
        selector: HashMap::new(),
        endpoints: vec![],
        created_at: Utc::now(),
    }
}

#[test]
fn netstate_register_remove_service() {
    let s = NetState::new();
    s.register_service(svc("api", "default"));
    assert_eq!(s.services.len(), 1);
    s.remove_service("api", "default");
    assert_eq!(s.services.len(), 0);
}

#[test]
fn netstate_update_endpoints_replaces_list() {
    let s = NetState::new();
    s.register_service(svc("api", "default"));
    let eps = vec![
        Endpoint { ip: ip(10, 0, 0, 1), port: 8080, pod_name: "p1".into(), ready: true },
        Endpoint { ip: ip(10, 0, 0, 2), port: 8080, pod_name: "p2".into(), ready: false },
    ];
    s.update_endpoints("api", "default", eps);
    let key = "default/api";
    let stored = s.services.get(key).unwrap();
    assert_eq!(stored.endpoints.len(), 2);
    assert_eq!(stored.endpoints[1].pod_name, "p2");
    assert!(!stored.endpoints[1].ready);
}

#[test]
fn netstate_update_endpoints_unknown_service_is_noop() {
    let s = NetState::new();
    s.update_endpoints("missing", "ns", vec![]);
    assert_eq!(s.services.len(), 0);
}

// ---------------------------------------------------------------------------
// NetState — policy enforcement
// ---------------------------------------------------------------------------

fn policy(name: &str, ns: &str, ingress: Vec<IngressRule>) -> NetworkPolicy {
    NetworkPolicy {
        name: name.into(),
        namespace: ns.into(),
        pod_selector: HashMap::new(),
        policy_types: vec![PolicyType::Ingress],
        ingress_rules: ingress,
        egress_rules: vec![],
        created_at: Utc::now(),
    }
}

#[test]
fn check_policy_allows_when_no_policies_in_namespace() {
    let s = NetState::new();
    assert_eq!(s.check_policy("a", "ns1", "b", "ns2", 80), FlowVerdict::Allowed);
}

#[test]
fn check_policy_denies_with_empty_ingress() {
    let s = NetState::new();
    s.apply_policy(policy("deny", "prod", vec![]));
    assert_eq!(s.check_policy("any", "other", "dst", "prod", 80), FlowVerdict::Denied);
}

#[test]
fn check_policy_allows_with_empty_from_rule() {
    let s = NetState::new();
    let allow_all = vec![IngressRule { from: vec![], ports: vec![] }];
    s.apply_policy(policy("allow", "prod", allow_all));
    assert_eq!(s.check_policy("any", "ns", "dst", "prod", 80), FlowVerdict::Allowed);
}

#[test]
fn remove_policy_reverts_to_default_allow() {
    let s = NetState::new();
    s.apply_policy(policy("deny", "prod", vec![]));
    s.remove_policy("deny", "prod");
    assert_eq!(s.check_policy("a", "ns", "dst", "prod", 80), FlowVerdict::Allowed);
}

// ---------------------------------------------------------------------------
// NetState — flows
// ---------------------------------------------------------------------------

fn flow(direction: FlowDirection, verdict: FlowVerdict) -> FlowRecord {
    FlowRecord {
        id: Uuid::new_v4(),
        timestamp: Utc::now(),
        source_ip: ip(10, 0, 0, 1),
        source_pod: None,
        destination_ip: ip(10, 0, 0, 2),
        destination_pod: None,
        destination_port: 80,
        protocol: Protocol::TCP,
        verdict,
        bytes: 1500,
        direction,
    }
}

#[test]
fn record_flow_stores_record() {
    let s = NetState::new();
    s.record_flow(flow(FlowDirection::Ingress, FlowVerdict::Allowed));
    s.record_flow(flow(FlowDirection::Egress, FlowVerdict::Denied));
    assert_eq!(s.flows.len(), 2);
}

// ---------------------------------------------------------------------------
// Models: enums + serde
// ---------------------------------------------------------------------------

#[test]
fn protocol_serializes_as_string_variants() {
    assert_eq!(serde_json::to_string(&Protocol::TCP).unwrap(), "\"TCP\"");
    assert_eq!(serde_json::to_string(&Protocol::UDP).unwrap(), "\"UDP\"");
}

#[test]
fn service_type_variants_round_trip_serde() {
    for t in [
        ServiceType::ClusterIP,
        ServiceType::NodePort,
        ServiceType::LoadBalancer,
        ServiceType::ExternalName,
    ] {
        let j = serde_json::to_string(&t).unwrap();
        let back: ServiceType = serde_json::from_str(&j).unwrap();
        assert_eq!(t, back);
    }
}

#[test]
fn flow_verdict_distinct_variants() {
    assert_ne!(FlowVerdict::Allowed, FlowVerdict::Denied);
    assert_ne!(FlowVerdict::Denied, FlowVerdict::Dropped);
}

#[test]
fn policy_type_serde_round_trip() {
    let j = serde_json::to_string(&PolicyType::Ingress).unwrap();
    let back: PolicyType = serde_json::from_str(&j).unwrap();
    assert_eq!(PolicyType::Ingress, back);
}

#[test]
fn ingress_egress_rules_default_construct() {
    let i = IngressRule { from: vec![], ports: vec![PolicyPort { port: 80, protocol: Protocol::TCP }] };
    assert_eq!(i.ports.len(), 1);
    let e = EgressRule { to: vec![PeerSelector { pod_selector: None, namespace_selector: None, ip_block: None }], ports: vec![] };
    assert_eq!(e.to.len(), 1);
}

// ---------------------------------------------------------------------------
// Cilium Cite types
// ---------------------------------------------------------------------------

#[test]
fn cilium_upstream_version_pinned() {
    assert!(UPSTREAM_VERSION.starts_with('v'));
    assert_eq!(UPSTREAM_REPO, "cilium/cilium");
}

#[test]
fn cilium_cite_url_points_to_pinned_version() {
    let c = Cite::cilium("pkg/x.go", "Sym");
    let url = c.url();
    assert!(url.contains("cilium/cilium"));
    assert!(url.contains(UPSTREAM_VERSION));
    assert!(url.contains("pkg/x.go"));
}

#[test]
fn cilium_cite_display_includes_repo_and_version() {
    let c = Cite::cilium("a/b.go", "S");
    let s = format!("{}", c);
    assert!(s.contains("cilium/cilium"));
    assert!(s.contains(UPSTREAM_VERSION));
}

#[test]
fn tenant_id_rejects_empty() {
    assert!(TenantId::new("").is_err());
    assert!(TenantId::new("ok").is_ok());
}
