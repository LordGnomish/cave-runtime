// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! REST API routes for networking.

use crate::dataplane::NetState;
use crate::ebpf_sim::bpf_host_sim::{Direction, HostVerdict};
use crate::ebpf_sim::policy_lpm::RangePolicyMap;
use crate::ebpf_sim::port_range::port_range_to_masked_ports;
use crate::ebpf_sim::program::L4Proto;
use crate::models::*;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

pub fn create_router(state: Arc<NetState>) -> Router {
    Router::new()
        .route("/api/net/health", get(health))
        .route("/api/net/pods", get(list_pod_ips).post(allocate_pod_ip))
        .route("/api/net/pods/{ns}/{name}", delete(release_pod_ip))
        .route(
            "/api/net/services",
            get(list_services).post(register_service),
        )
        .route("/api/net/services/{ns}/{name}", delete(remove_service))
        .route("/api/net/policies", get(list_policies).post(apply_policy))
        .route("/api/net/policies/{ns}/{name}", delete(remove_policy))
        .route("/api/net/flows", get(list_flows))
        .route("/api/net/check", post(check_policy))
        .route("/api/net/policy/port-range", get(port_range_decompose))
        .route("/api/net/policy/port-range/check", post(port_range_check))
        .with_state(state)
}

/// Parse a protocol name (case-insensitive) into an [`L4Proto`].
/// Unknown names return `None` so callers can fail closed (deny).
fn parse_l4_proto(s: &str) -> Option<L4Proto> {
    match s.to_ascii_lowercase().as_str() {
        "tcp" => Some(L4Proto::Tcp),
        "udp" => Some(L4Proto::Udp),
        "icmp" => Some(L4Proto::Icmp),
        "sctp" => Some(L4Proto::Sctp),
        _ => None,
    }
}

/// Build the JSON view of a port range's masked-port decomposition.
/// Exposes Cilium's `PortRangeToMaskedPorts` (v1.19.3) so the portal can
/// render how an L4 policy range tiles the datapath LPM trie.
pub fn port_range_decomposition_json(start: u16, end: u16) -> serde_json::Value {
    let prefixes: Vec<serde_json::Value> = port_range_to_masked_ports(start, end)
        .into_iter()
        .map(|mp| {
            serde_json::json!({
                "port": format!("0x{:04x}", mp.port),
                "mask": format!("0x{:04x}", mp.mask),
                "port_dec": mp.port,
                "covered": mp.covered(),
            })
        })
        .collect();
    serde_json::json!({
        "start": start,
        "end": end,
        "prefix_count": prefixes.len(),
        "prefixes": prefixes,
    })
}

/// Build the JSON verdict for probing `probe_port` against an L4 policy
/// range `[start, end]` for `peer_identity`. Mirrors the datapath's
/// longest-prefix-match resolution via [`RangePolicyMap`]. An
/// unparseable protocol fails closed to `deny`.
pub fn port_range_verdict_json(
    peer_identity: u32,
    start: u16,
    end: u16,
    proto: &str,
    direction: &str,
    probe_port: u16,
) -> serde_json::Value {
    let dir = if direction.eq_ignore_ascii_case("egress") {
        Direction::Egress
    } else {
        Direction::Ingress
    };
    let verdict = match parse_l4_proto(proto) {
        Some(p) => {
            let mut m = RangePolicyMap::new();
            m.insert_range(peer_identity, start, end, p, dir, HostVerdict::Allow);
            m.lookup(peer_identity, probe_port, p, dir)
        }
        None => HostVerdict::Deny,
    };
    let verdict_str = match verdict {
        HostVerdict::Allow => "allow",
        HostVerdict::Deny => "deny",
        HostVerdict::Audit => "audit",
    };
    serde_json::json!({
        "peer_identity": peer_identity,
        "start": start,
        "end": end,
        "proto": proto,
        "direction": direction,
        "probe_port": probe_port,
        "verdict": verdict_str,
    })
}

#[derive(Deserialize)]
struct PortRangeQuery {
    start: u16,
    end: u16,
}

async fn port_range_decompose(Query(q): Query<PortRangeQuery>) -> Json<serde_json::Value> {
    Json(port_range_decomposition_json(q.start, q.end))
}

#[derive(Deserialize)]
struct PortRangeCheckReq {
    peer_identity: u32,
    start: u16,
    end: u16,
    proto: String,
    direction: String,
    probe_port: u16,
}

async fn port_range_check(Json(req): Json<PortRangeCheckReq>) -> Json<serde_json::Value> {
    Json(port_range_verdict_json(
        req.peer_identity,
        req.start,
        req.end,
        &req.proto,
        &req.direction,
        req.probe_port,
    ))
}

async fn health() -> Json<serde_json::Value> {
    Json(
        serde_json::json!({"module":"cave-net","status":"ok","upstream":"cilium","features":["pod-ip","clusterip","network-policy","flow-records"]}),
    )
}

async fn list_pod_ips(State(s): State<Arc<NetState>>) -> Json<Vec<PodNetwork>> {
    Json(s.pods.iter().map(|r| r.value().clone()).collect())
}

#[derive(Deserialize)]
struct AllocReq {
    pod_name: String,
    namespace: String,
    node_name: String,
    labels: HashMap<String, String>,
}

async fn allocate_pod_ip(
    State(s): State<Arc<NetState>>,
    Json(req): Json<AllocReq>,
) -> (StatusCode, Json<PodNetwork>) {
    let pn = s.allocate_pod_ip(&req.pod_name, &req.namespace, &req.node_name, req.labels);
    (StatusCode::CREATED, Json(pn))
}

async fn release_pod_ip(
    State(s): State<Arc<NetState>>,
    Path((ns, name)): Path<(String, String)>,
) -> StatusCode {
    s.release_pod_ip(&name, &ns);
    StatusCode::OK
}

async fn list_services(State(s): State<Arc<NetState>>) -> Json<Vec<ServiceEntry>> {
    Json(s.services.iter().map(|r| r.value().clone()).collect())
}

async fn register_service(
    State(s): State<Arc<NetState>>,
    Json(svc): Json<ServiceEntry>,
) -> (StatusCode, Json<ServiceEntry>) {
    s.register_service(svc.clone());
    (StatusCode::CREATED, Json(svc))
}

async fn remove_service(
    State(s): State<Arc<NetState>>,
    Path((ns, name)): Path<(String, String)>,
) -> StatusCode {
    s.remove_service(&name, &ns);
    StatusCode::OK
}

async fn list_policies(State(s): State<Arc<NetState>>) -> Json<Vec<NetworkPolicy>> {
    Json(s.policies.iter().map(|r| r.value().clone()).collect())
}

async fn apply_policy(
    State(s): State<Arc<NetState>>,
    Json(policy): Json<NetworkPolicy>,
) -> (StatusCode, Json<NetworkPolicy>) {
    s.apply_policy(policy.clone());
    (StatusCode::CREATED, Json(policy))
}

async fn remove_policy(
    State(s): State<Arc<NetState>>,
    Path((ns, name)): Path<(String, String)>,
) -> StatusCode {
    s.remove_policy(&name, &ns);
    StatusCode::OK
}

async fn list_flows(State(s): State<Arc<NetState>>) -> Json<Vec<FlowRecord>> {
    Json(s.flows.iter().map(|r| r.value().clone()).collect())
}

#[derive(Deserialize)]
struct CheckReq {
    src_pod: String,
    src_ns: String,
    dst_pod: String,
    dst_ns: String,
    dst_port: u16,
}

async fn check_policy(
    State(s): State<Arc<NetState>>,
    Json(req): Json<CheckReq>,
) -> Json<serde_json::Value> {
    let verdict = s.check_policy(
        &req.src_pod,
        &req.src_ns,
        &req.dst_pod,
        &req.dst_ns,
        req.dst_port,
    );
    Json(serde_json::json!({"verdict": verdict}))
}
