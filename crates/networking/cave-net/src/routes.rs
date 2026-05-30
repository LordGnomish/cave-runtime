// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! REST API routes for networking.

use crate::dataplane::NetState;
use crate::ebpf_sim::bpf_host_sim::{Direction, HostVerdict};
use crate::ebpf_sim::policy_lpm::RangePolicyMap;
use crate::ebpf_sim::port_range::port_range_to_masked_ports;
use crate::ebpf_sim::program::{Ipv4, L4Proto};
use crate::ebpf_sim::{
    build_v4_in_v6, build_v4_in_v6_rfc6052, edt_sched_departure, get_v4_from_v6, EdtInfo,
    EdtVerdict, V6Addr,
};
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
        .route("/api/net/bandwidth/schedule", post(bandwidth_schedule))
        .route("/api/net/nat64/translate", get(nat64_translate))
        .with_state(state)
}

/// Render an IPv6 address as RFC 5952 hextet notation (lower-case, the
/// longest run of >=2 zero groups collapsed to `::`). We format hextets
/// rather than deferring to `std::net::Ipv6Addr`'s Display, which
/// special-cases IPv4-mapped addresses into mixed `::ffff:a.b.c.d`
/// notation — here we want the raw embedding visible.
fn format_v6_hextets(addr: &V6Addr) -> String {
    let groups: [u16; 8] = std::array::from_fn(|i| {
        u16::from_be_bytes([addr.0[i * 2], addr.0[i * 2 + 1]])
    });
    // Find the longest run of consecutive zero groups (length >= 2).
    let (mut best_start, mut best_len) = (usize::MAX, 0usize);
    let (mut cur_start, mut cur_len) = (0usize, 0usize);
    for (i, &g) in groups.iter().enumerate() {
        if g == 0 {
            if cur_len == 0 {
                cur_start = i;
            }
            cur_len += 1;
            if cur_len > best_len {
                best_len = cur_len;
                best_start = cur_start;
            }
        } else {
            cur_len = 0;
        }
    }
    if best_len < 2 {
        return groups
            .iter()
            .map(|g| format!("{g:x}"))
            .collect::<Vec<_>>()
            .join(":");
    }
    let head: Vec<String> = groups[..best_start].iter().map(|g| format!("{g:x}")).collect();
    let tail: Vec<String> = groups[best_start + best_len..]
        .iter()
        .map(|g| format!("{g:x}"))
        .collect();
    format!("{}::{}", head.join(":"), tail.join(":"))
}

/// Parse a dotted-quad IPv4 string into [`Ipv4`], rejecting malformed input.
fn parse_v4(s: &str) -> Option<Ipv4> {
    let o: Vec<u8> = s.split('.').filter_map(|p| p.parse::<u8>().ok()).collect();
    if o.len() == 4 && s.split('.').count() == 4 {
        Some(Ipv4::from_octets(o[0], o[1], o[2], o[3]))
    } else {
        None
    }
}

/// Build the JSON verdict for scheduling a packet against an EDT
/// aggregate. Mirrors Cilium's `edt_sched_departure` (`bpf/lib/edt.h`):
/// reports the pacing `verdict` (`pass`/`drop`), the stamped departure
/// `tstamp`, and the computed transmission `delay_ns`.
pub fn edt_schedule_json(
    bps: u64,
    t_last: u64,
    t_horizon_drop: u64,
    packet_len: u64,
    now_ns: u64,
    tstamp_ns: u64,
) -> serde_json::Value {
    let mut info = EdtInfo::with_t_last(bps, t_horizon_drop, t_last);
    let (verdict, tstamp) = edt_sched_departure(&mut info, packet_len, now_ns, tstamp_ns);
    let delay_ns = if bps == 0 {
        0
    } else {
        packet_len * crate::ebpf_sim::NSEC_PER_SEC / bps
    };
    let verdict_str = match verdict {
        EdtVerdict::Pass => "pass",
        EdtVerdict::Drop => "drop",
    };
    serde_json::json!({
        "verdict": verdict_str,
        "tstamp": tstamp,
        "delay_ns": delay_ns,
        "bps": bps,
        "t_last": info.t_last,
        "t_horizon_drop": t_horizon_drop,
    })
}

/// Build the JSON view of a NAT46/64 address translation. Encodes the
/// IPv4 in either the `mapped` (`::ffff:`) or `rfc6052` (`64:ff9b::/96`)
/// form and recovers it back, mirroring `bpf/lib/nat_46x64.h`. A
/// malformed IPv4 reports an `error` and fails closed.
pub fn nat64_translate_json(v4: &str, encoding: &str) -> serde_json::Value {
    let ip = match parse_v4(v4) {
        Some(ip) => ip,
        None => return serde_json::json!({"error": "invalid IPv4 address"}),
    };
    let v6 = if encoding.eq_ignore_ascii_case("rfc6052") {
        build_v4_in_v6_rfc6052(ip)
    } else {
        build_v4_in_v6(ip)
    };
    let encoding_str = if encoding.eq_ignore_ascii_case("rfc6052") {
        "rfc6052"
    } else {
        "mapped"
    };
    let recovered = get_v4_from_v6(&v6).map(|r| r.to_string());
    serde_json::json!({
        "v4": ip.to_string(),
        "encoding": encoding_str,
        "v6": format_v6_hextets(&v6),
        "recovered_v4": recovered,
    })
}

#[derive(Deserialize)]
struct BandwidthScheduleReq {
    bps: u64,
    t_last: u64,
    #[serde(default = "default_horizon")]
    t_horizon_drop: u64,
    packet_len: u64,
    now_ns: u64,
    tstamp_ns: u64,
}

fn default_horizon() -> u64 {
    crate::ebpf_sim::DEFAULT_DROP_HORIZON_NS
}

async fn bandwidth_schedule(Json(req): Json<BandwidthScheduleReq>) -> Json<serde_json::Value> {
    Json(edt_schedule_json(
        req.bps,
        req.t_last,
        req.t_horizon_drop,
        req.packet_len,
        req.now_ns,
        req.tstamp_ns,
    ))
}

#[derive(Deserialize)]
struct Nat64Query {
    v4: String,
    #[serde(default = "default_encoding")]
    encoding: String,
}

fn default_encoding() -> String {
    "mapped".to_string()
}

async fn nat64_translate(Query(q): Query<Nat64Query>) -> Json<serde_json::Value> {
    Json(nat64_translate_json(&q.v4, &q.encoding))
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
