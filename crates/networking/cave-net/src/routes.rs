// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! REST API routes for networking.

use crate::dataplane::NetState;
use crate::ebpf_sim::bpf_host_sim::{Direction, HostVerdict};
use crate::ebpf_sim::dsr_sim::{dsr_extract_opt4, dsr_set_opt4, DsrDrop, DsrSetOutcome, Ipv4Hdr};
use crate::ebpf_sim::lb_sim::{LbAlgo, LbBackend, LbMaps, LbTuple};
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
        .route("/api/net/dsr/encode", get(dsr_encode))
        .route("/api/net/lb/affinity/simulate", post(lb_affinity_simulate))
        .route("/api/net/lb/source-range/check", post(lb_source_range_check))
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

/// Parse a dotted-quad into the numeric (network-order) `u32` the
/// datapath sims key on. Returns `None` on a malformed address.
fn parse_ipv4(s: &str) -> Option<u32> {
    let a: std::net::Ipv4Addr = s.parse().ok()?;
    Some(u32::from(a))
}

/// Build the JSON view of a DSR IPv4-option encode + round-trip decode.
/// Mirrors Cilium's `dsr_set_opt4` / `dsr_extract_opt4`
/// (`bpf/lib/nodeport.h`): embed the service VIP+port in the 8-byte
/// option, grow the header, then recover it on the backend node.
pub fn dsr_encode_json(
    svc_addr: &str,
    svc_port: u16,
    proto: &str,
    ihl: u8,
    tot_len: u16,
    tcp_syn: bool,
    mtu: Option<u16>,
) -> serde_json::Value {
    let addr = match parse_ipv4(svc_addr) {
        Some(a) => a,
        None => return serde_json::json!({"error": "invalid svc_addr"}),
    };
    let protocol = match proto.to_ascii_lowercase().as_str() {
        "tcp" => 6,
        _ => 17, // default UDP
    };
    let mut ip4 = Ipv4Hdr { ihl, tot_len, protocol };
    let outcome = dsr_set_opt4(&mut ip4, addr, svc_port, tcp_syn, mtu);
    match outcome {
        DsrSetOutcome::Set { opt } => {
            let recovered = dsr_extract_opt4(&ip4, &opt.to_bytes());
            serde_json::json!({
                "result": "set",
                "option_bytes": opt.to_bytes().iter().map(|b| format!("0x{b:02x}")).collect::<Vec<_>>(),
                "new_ihl": ip4.ihl,
                "new_tot_len": ip4.tot_len,
                "svc_addr": svc_addr,
                "svc_port": svc_port,
                "recovered": recovered.map(|(a, p)| serde_json::json!({
                    "svc_addr": std::net::Ipv4Addr::from(a).to_string(),
                    "svc_port": p,
                })),
            })
        }
        DsrSetOutcome::SkipNonSyn => serde_json::json!({"result": "skip_non_syn"}),
        DsrSetOutcome::Drop(DsrDrop::CtInvalidHdr) => {
            serde_json::json!({"result": "drop", "reason": "ct_invalid_hdr"})
        }
        DsrSetOutcome::Drop(DsrDrop::FragNeeded) => {
            serde_json::json!({"result": "drop", "reason": "frag_needed"})
        }
    }
}

/// Build the JSON view of a session-affinity walk: a 3-backend service
/// receives a sequence of probes from one client, and we report which
/// backend each probe lands on plus whether it was a sticky reuse.
/// Mirrors `lb4_local_affinity` (`bpf/lib/lb.h`).
pub fn lb_affinity_simulate_json(
    client: &str,
    affinity_timeout: u32,
    probes: &[(u32, u64)],
) -> serde_json::Value {
    let saddr = match parse_ipv4(client) {
        Some(a) => a,
        None => return serde_json::json!({"error": "invalid client"}),
    };
    let vip = u32::from_be_bytes([172, 20, 0, 1]);
    let mut maps = LbMaps::new();
    maps.add_service_with_affinity(
        vip,
        80,
        L4Proto::Tcp.proto_num(),
        9,
        LbAlgo::Random,
        &[
            (1, LbBackend { address: u32::from_be_bytes([10, 1, 0, 1]), port: 8080 }),
            (2, LbBackend { address: u32::from_be_bytes([10, 1, 0, 2]), port: 8080 }),
            (3, LbBackend { address: u32::from_be_bytes([10, 1, 0, 3]), port: 8080 }),
        ],
        affinity_timeout,
    );
    let mut last_backend = 0u32;
    let steps: Vec<serde_json::Value> = probes
        .iter()
        .map(|&(prandom, now_ns)| {
            let tuple = LbTuple {
                saddr,
                daddr: vip,
                sport: 5000,
                dport: 80,
                nexthdr: L4Proto::Tcp.proto_num(),
            };
            let xlate = maps.lb4_local_affinity(&tuple, prandom, now_ns);
            let backend_id = xlate.map(|x| x.backend_id).unwrap_or(0);
            let sticky = backend_id != 0 && backend_id == last_backend;
            last_backend = backend_id;
            serde_json::json!({
                "prandom": prandom,
                "now_ns": now_ns,
                "backend_id": backend_id,
                "sticky_reuse": sticky,
            })
        })
        .collect();
    serde_json::json!({
        "client": client,
        "affinity_timeout_sec": affinity_timeout,
        "probes": steps,
    })
}

/// Build the JSON verdict for a client address against a service's
/// source ranges. Mirrors `lb4_src_range_ok` (`bpf/lib/lb.h`): LPM
/// membership, XOR'd with the deny flag. CIDRs are `"a.b.c.d/len"`.
pub fn lb_source_range_json(ranges: &[String], deny: bool, client: &str) -> serde_json::Value {
    let saddr = match parse_ipv4(client) {
        Some(a) => a,
        None => return serde_json::json!({"error": "invalid client"}),
    };
    let mut maps = LbMaps::new();
    let rev = 9u16;
    for cidr in ranges {
        let (net_s, len_s) = match cidr.split_once('/') {
            Some(parts) => parts,
            None => return serde_json::json!({"error": format!("invalid cidr: {cidr}")}),
        };
        let net = match parse_ipv4(net_s) {
            Some(a) => a,
            None => return serde_json::json!({"error": format!("invalid cidr: {cidr}")}),
        };
        let prefix_len: u8 = match len_s.parse() {
            Ok(p) if p <= 32 => p,
            _ => return serde_json::json!({"error": format!("invalid prefix: {cidr}")}),
        };
        maps.add_source_range(rev, net, prefix_len);
    }
    maps.set_source_range_deny(rev, deny);
    let ok = maps.lb4_src_range_ok(rev, saddr);
    serde_json::json!({
        "client": client,
        "ranges": ranges,
        "deny": deny,
        "verdict": if ok { "allow" } else { "deny" },
    })
}

#[derive(Deserialize)]
struct DsrEncodeQuery {
    svc_addr: String,
    svc_port: u16,
    #[serde(default = "default_udp")]
    proto: String,
    #[serde(default = "default_ihl")]
    ihl: u8,
    #[serde(default = "default_tot_len")]
    tot_len: u16,
    #[serde(default)]
    tcp_syn: bool,
    #[serde(default)]
    mtu: Option<u16>,
}

fn default_udp() -> String {
    "udp".to_string()
}
fn default_ihl() -> u8 {
    5
}
fn default_tot_len() -> u16 {
    40
}

async fn dsr_encode(Query(q): Query<DsrEncodeQuery>) -> Json<serde_json::Value> {
    Json(dsr_encode_json(
        &q.svc_addr,
        q.svc_port,
        &q.proto,
        q.ihl,
        q.tot_len,
        q.tcp_syn,
        q.mtu,
    ))
}

#[derive(Deserialize)]
struct AffinityProbe {
    #[serde(default)]
    prandom: u32,
    #[serde(default)]
    now_ns: u64,
}

#[derive(Deserialize)]
struct AffinitySimReq {
    client: String,
    affinity_timeout: u32,
    probes: Vec<AffinityProbe>,
}

async fn lb_affinity_simulate(Json(req): Json<AffinitySimReq>) -> Json<serde_json::Value> {
    let probes: Vec<(u32, u64)> = req.probes.iter().map(|p| (p.prandom, p.now_ns)).collect();
    Json(lb_affinity_simulate_json(&req.client, req.affinity_timeout, &probes))
}

#[derive(Deserialize)]
struct SourceRangeReq {
    client: String,
    #[serde(default)]
    ranges: Vec<String>,
    #[serde(default)]
    deny: bool,
}

async fn lb_source_range_check(Json(req): Json<SourceRangeReq>) -> Json<serde_json::Value> {
    Json(lb_source_range_json(&req.ranges, req.deny, &req.client))
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
