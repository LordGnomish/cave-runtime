// SPDX-License-Identifier: AGPL-3.0-or-later
//! EndpointSliceMap — parity tests against k8s v1.36.0.
//!
//! Upstream: `pkg/proxy/endpointslicecache.go` (cache + change tracking)
//! and `pkg/proxy/topology.go` (locality categorisation).

use cave_kube_proxy::{EndpointInfo, EndpointSliceMap, KubeProxyError, ServicePortName};
use std::net::IpAddr;

const TENANT: &str = "tenant-acme-prod";

fn ep(addr: &str, port: u16, ready: bool, node: Option<&str>) -> EndpointInfo {
    EndpointInfo {
        addresses: vec![addr.parse::<IpAddr>().unwrap()],
        port,
        ready,
        serving: ready,
        terminating: false,
        node_name: node.map(String::from),
        zone: None,
    }
}

fn svc(name: &str) -> ServicePortName {
    ServicePortName::new("default", name, "http")
}

/// Cite: `pkg/proxy/endpointslicecache.go:95` (updatePending) — an
/// upsert overwrites the named slice in place; subsequent
/// `endpoints_for` reflects only the latest endpoints.
#[test]
fn upsert_slice_overwrites_in_place() {
    let mut m = EndpointSliceMap::new(TENANT);
    let s = svc("api");
    m.upsert_slice(s.clone(), "slice-1", vec![ep("10.1.0.1", 8080, true, Some("nodeA"))]);
    assert_eq!(m.endpoints_for(&s).len(), 1);

    m.upsert_slice(s.clone(), "slice-1", vec![
        ep("10.1.0.1", 8080, true, Some("nodeA")),
        ep("10.1.0.2", 8080, true, Some("nodeB")),
    ]);
    assert_eq!(m.endpoints_for(&s).len(), 2, "old slice contents are replaced, not merged");
}

/// Cite: `pkg/proxy/endpointslicecache.go:162` (getEndpointsMap) —
/// multiple slices on the same Service are flattened into a single
/// endpoint list when the proxier asks for it.
#[test]
fn endpoints_for_flattens_multiple_slices() {
    let mut m = EndpointSliceMap::new(TENANT);
    let s = svc("api");
    m.upsert_slice(s.clone(), "slice-1", vec![ep("10.1.0.1", 80, true, Some("nodeA"))]);
    m.upsert_slice(s.clone(), "slice-2", vec![
        ep("10.1.0.2", 80, true, Some("nodeB")),
        ep("10.1.0.3", 80, false, Some("nodeC")),
    ]);
    assert_eq!(m.endpoints_for(&s).len(), 3, "flatten across slices");
    assert_eq!(m.ready_endpoints_for(&s).len(), 2, "filter unready");
}

/// Cite: `pkg/proxy/endpointslicecache.go:95` (updatePending,
/// `remove == true`) — deleting one slice leaves its sibling slices
/// untouched and intact.
#[test]
fn delete_slice_keeps_sibling_slices_intact() {
    let mut m = EndpointSliceMap::new(TENANT);
    let s = svc("api");
    m.upsert_slice(s.clone(), "slice-1", vec![ep("10.1.0.1", 80, true, Some("nodeA"))]);
    m.upsert_slice(s.clone(), "slice-2", vec![ep("10.1.0.2", 80, true, Some("nodeB"))]);

    assert!(m.delete_slice(&s, "slice-1"));
    assert_eq!(m.endpoints_for(&s).len(), 1);
    assert_eq!(m.endpoints_for(&s)[0].addresses[0], "10.1.0.2".parse::<IpAddr>().unwrap());
    assert!(!m.delete_slice(&s, "slice-1"), "second delete is a no-op");
}

/// Cite: `pkg/proxy/topology.go:48` (CategorizeEndpoints) — local
/// endpoints support externalTrafficPolicy=Local. cave's
/// `local_ready_endpoints` filters by both `ready` and `node_name`.
#[test]
fn local_ready_endpoints_filter_by_node_and_ready() {
    let mut m = EndpointSliceMap::new(TENANT);
    let s = svc("api");
    m.upsert_slice(s.clone(), "slice-1", vec![
        ep("10.1.0.1", 80, true,  Some("nodeA")),
        ep("10.1.0.2", 80, false, Some("nodeA")),  // not ready
        ep("10.1.0.3", 80, true,  Some("nodeB")),  // wrong node
    ]);
    let local = m.local_ready_endpoints(&s, "nodeA");
    assert_eq!(local.len(), 1);
    assert_eq!(local[0].addresses[0], "10.1.0.1".parse::<IpAddr>().unwrap());

    // Cross-tenant guard — kept here so the tenant invariant is exercised
    // alongside the topology test.
    let err = m.check_tenant("tenant-other").unwrap_err();
    assert!(matches!(err, KubeProxyError::CrossTenantDenied { .. }));
}
