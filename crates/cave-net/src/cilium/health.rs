// SPDX-License-Identifier: AGPL-3.0-or-later
//! cilium-health — cluster-wide connectivity probes.
//!
//! Mirrors `pkg/health/probe/probe.go` (the prober that pings every
//! known node + endpoint) and `pkg/health/server/server.go` (the
//! agent-side aggregator that surfaces results via the
//! `cilium-health` CLI).
//!
//! Probe types:
//!
//! * `Icmp` — ICMP echo to the remote node IP / health endpoint IP.
//! * `Http` — HTTP GET to the remote health endpoint
//!   (`http://<ip>:<port>/hello`).
//!
//! Each node has a per-cluster probe matrix recording the freshest
//! result per (target, protocol). The aggregator answers
//! `node_status(name)` and `cluster_status()` queries that mirror
//! `cilium-health status`.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ProbeProto {
    Icmp,
    Http,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProbeStatus {
    Ok,
    Unreachable,
    Degraded,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeResult {
    pub target_node: String,
    pub target_ip: IpAddr,
    pub proto: ProbeProto,
    pub status: ProbeStatus,
    pub latency_us: Option<u64>,
    pub timestamp_ns: u64,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeStatus {
    pub node: String,
    pub host: BTreeMap<String, ProbeStatus>,    // proto.name() → status (host IP)
    pub endpoint: BTreeMap<String, ProbeStatus>, // proto.name() → status (health endpoint IP)
    pub last_probed_ns: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterHealth {
    pub total: u64,
    pub healthy: u64,
    pub degraded: u64,
    pub unreachable: u64,
}

impl ProbeProto {
    pub fn name(self) -> &'static str {
        match self {
            ProbeProto::Icmp => "icmp",
            ProbeProto::Http => "http",
        }
    }
}

impl ProbeStatus {
    pub fn is_healthy(self) -> bool {
        matches!(self, ProbeStatus::Ok)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HealthError {
    #[error("node `{0}` not found")]
    NodeNotFound(String),
    #[error("tenant {tenant} cannot mutate health server owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct HealthServer {
    pub tenant: TenantId,
    /// (node, target_ip, proto) → latest result.
    results: BTreeMap<(String, IpAddr, ProbeProto), ProbeResult>,
    /// node → (host_ip, endpoint_ip).
    node_addrs: BTreeMap<String, (IpAddr, IpAddr)>,
}

impl HealthServer {
    pub fn new(tenant: TenantId) -> Self {
        Self { tenant, results: BTreeMap::new(), node_addrs: BTreeMap::new() }
    }

    pub fn register_node(&mut self, name: impl Into<String>, host_ip: IpAddr, endpoint_ip: IpAddr) {
        self.node_addrs.insert(name.into(), (host_ip, endpoint_ip));
    }

    pub fn unregister_node(&mut self, name: &str) -> Result<(), HealthError> {
        self.node_addrs.remove(name).ok_or_else(|| HealthError::NodeNotFound(name.to_string()))?;
        // Drop any outstanding probe results for this node.
        self.results.retain(|(n, _, _), _| n != name);
        Ok(())
    }

    pub fn record(&mut self, result: ProbeResult) {
        self.results.insert((result.target_node.clone(), result.target_ip, result.proto), result);
    }

    pub fn node_status(&self, name: &str) -> Result<NodeStatus, HealthError> {
        let (host_ip, endpoint_ip) = self.node_addrs.get(name)
            .copied()
            .ok_or_else(|| HealthError::NodeNotFound(name.to_string()))?;
        let mut status = NodeStatus {
            node: name.to_string(),
            host: BTreeMap::new(),
            endpoint: BTreeMap::new(),
            last_probed_ns: 0,
        };
        for proto in [ProbeProto::Icmp, ProbeProto::Http] {
            let host_key = (name.to_string(), host_ip, proto);
            let ep_key = (name.to_string(), endpoint_ip, proto);
            let host = self.results.get(&host_key).map(|r| r.status).unwrap_or(ProbeStatus::Unknown);
            let ep = self.results.get(&ep_key).map(|r| r.status).unwrap_or(ProbeStatus::Unknown);
            status.host.insert(proto.name().into(), host);
            status.endpoint.insert(proto.name().into(), ep);
            for r in [self.results.get(&host_key), self.results.get(&ep_key)].into_iter().flatten() {
                if r.timestamp_ns > status.last_probed_ns {
                    status.last_probed_ns = r.timestamp_ns;
                }
            }
        }
        Ok(status)
    }

    pub fn cluster_status(&self) -> ClusterHealth {
        let mut total = 0u64;
        let mut healthy = 0u64;
        let mut degraded = 0u64;
        let mut unreachable = 0u64;
        for (name, _) in &self.node_addrs {
            let status = match self.node_status(name) {
                Ok(s) => s,
                Err(_) => continue,
            };
            total += 1;
            // A node is healthy when the host icmp probe is OK.
            let host_icmp = status.host.get("icmp").copied().unwrap_or(ProbeStatus::Unknown);
            match host_icmp {
                ProbeStatus::Ok => healthy += 1,
                ProbeStatus::Degraded => degraded += 1,
                ProbeStatus::Unreachable => unreachable += 1,
                ProbeStatus::Unknown => {}
            }
        }
        ClusterHealth { total, healthy, degraded, unreachable }
    }

    pub fn known_nodes(&self) -> BTreeSet<&String> {
        self.node_addrs.keys().collect()
    }

    pub fn node_count(&self) -> usize {
        self.node_addrs.len()
    }

    pub fn result_count(&self) -> usize {
        self.results.len()
    }
}

/// Decide a probe status from a latency observation. Mirrors
/// `pkg/health/server/server.go::statusFromLatency`.
pub fn status_from_latency(latency_us: Option<u64>, timeout_us: u64) -> ProbeStatus {
    match latency_us {
        Some(l) if l >= timeout_us => ProbeStatus::Unreachable,
        Some(l) if l >= timeout_us / 2 => ProbeStatus::Degraded,
        Some(_) => ProbeStatus::Ok,
        None => ProbeStatus::Unreachable,
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/health/server/server.go", "Server");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    fn server(tenant: TenantId) -> HealthServer {
        HealthServer::new(tenant)
    }

    fn make_result(node: &str, target_ip: IpAddr, proto: ProbeProto, status: ProbeStatus, ts: u64) -> ProbeResult {
        ProbeResult {
            target_node: node.into(),
            target_ip, proto, status,
            latency_us: if matches!(status, ProbeStatus::Ok) { Some(100) } else { None },
            timestamp_ns: ts,
            message: None,
        }
    }

    // ── ProbeProto / Status ─────────────────────────────────────────────────

    #[test]
    fn probe_proto_name_is_lowercase() {
        let (_c, _t) = cilium_test_ctx!("pkg/health/probe/probe.go", "Proto.Name", "tenant-h-pn");
        assert_eq!(ProbeProto::Icmp.name(), "icmp");
        assert_eq!(ProbeProto::Http.name(), "http");
    }

    #[test]
    fn probe_status_is_healthy_only_for_ok() {
        let (_c, _t) = cilium_test_ctx!("pkg/health/server/server.go", "Status.IsHealthy", "tenant-h-ihy");
        assert!(ProbeStatus::Ok.is_healthy());
        assert!(!ProbeStatus::Degraded.is_healthy());
        assert!(!ProbeStatus::Unreachable.is_healthy());
        assert!(!ProbeStatus::Unknown.is_healthy());
    }

    // ── status_from_latency ─────────────────────────────────────────────────

    #[test]
    fn status_from_latency_ok_when_below_half_timeout() {
        let (_c, _t) = cilium_test_ctx!("pkg/health/server/server.go", "StatusFromLatency.Ok", "tenant-h-sok");
        assert_eq!(status_from_latency(Some(100), 1000), ProbeStatus::Ok);
    }

    #[test]
    fn status_from_latency_degraded_when_between_half_and_full_timeout() {
        let (_c, _t) = cilium_test_ctx!("pkg/health/server/server.go", "StatusFromLatency.Degraded", "tenant-h-sdeg");
        assert_eq!(status_from_latency(Some(700), 1000), ProbeStatus::Degraded);
    }

    #[test]
    fn status_from_latency_unreachable_at_or_past_timeout() {
        let (_c, _t) = cilium_test_ctx!("pkg/health/server/server.go", "StatusFromLatency.Unreachable", "tenant-h-sunr");
        assert_eq!(status_from_latency(Some(1500), 1000), ProbeStatus::Unreachable);
        assert_eq!(status_from_latency(Some(1000), 1000), ProbeStatus::Unreachable);
    }

    #[test]
    fn status_from_latency_none_is_unreachable() {
        let (_c, _t) = cilium_test_ctx!("pkg/health/server/server.go", "StatusFromLatency.None", "tenant-h-snone");
        assert_eq!(status_from_latency(None, 1000), ProbeStatus::Unreachable);
    }

    // ── Node lifecycle ──────────────────────────────────────────────────────

    #[test]
    fn health_register_node_adds_to_known() {
        let (_c, tenant) = cilium_test_ctx!("pkg/health/server/server.go", "RegisterNode", "tenant-h-reg");
        let mut s = server(tenant);
        s.register_node("node-a", ip(10, 0, 0, 1), ip(10, 0, 1, 1));
        assert_eq!(s.node_count(), 1);
        assert!(s.known_nodes().contains(&"node-a".to_string()));
    }

    #[test]
    fn health_unregister_node_removes_results() {
        let (_c, tenant) = cilium_test_ctx!("pkg/health/server/server.go", "UnregisterNode", "tenant-h-unr");
        let mut s = server(tenant);
        s.register_node("node-a", ip(10, 0, 0, 1), ip(10, 0, 1, 1));
        s.record(make_result("node-a", ip(10, 0, 0, 1), ProbeProto::Icmp, ProbeStatus::Ok, 100));
        s.unregister_node("node-a").unwrap();
        assert_eq!(s.node_count(), 0);
        assert_eq!(s.result_count(), 0);
    }

    #[test]
    fn health_unregister_unknown_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!("pkg/health/server/server.go", "UnregisterNode.NotFound", "tenant-h-unrnf");
        let mut s = server(tenant);
        let err = s.unregister_node("ghost").unwrap_err();
        assert!(matches!(err, HealthError::NodeNotFound(_)));
    }

    // ── Record + node status ────────────────────────────────────────────────

    #[test]
    fn health_record_then_node_status_reports_ok() {
        let (_c, tenant) = cilium_test_ctx!("pkg/health/server/server.go", "Record.NodeStatus", "tenant-h-rec");
        let mut s = server(tenant);
        s.register_node("node-a", ip(10, 0, 0, 1), ip(10, 0, 1, 1));
        s.record(make_result("node-a", ip(10, 0, 0, 1), ProbeProto::Icmp, ProbeStatus::Ok, 100));
        let st = s.node_status("node-a").unwrap();
        assert_eq!(st.host.get("icmp").copied(), Some(ProbeStatus::Ok));
    }

    #[test]
    fn health_node_status_unknown_for_unprobed_protocol() {
        let (_c, tenant) = cilium_test_ctx!("pkg/health/server/server.go", "NodeStatus.Unknown", "tenant-h-nsunk");
        let mut s = server(tenant);
        s.register_node("node-a", ip(10, 0, 0, 1), ip(10, 0, 1, 1));
        s.record(make_result("node-a", ip(10, 0, 0, 1), ProbeProto::Icmp, ProbeStatus::Ok, 100));
        let st = s.node_status("node-a").unwrap();
        assert_eq!(st.host.get("http").copied(), Some(ProbeStatus::Unknown));
    }

    #[test]
    fn health_node_status_unknown_node_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!("pkg/health/server/server.go", "NodeStatus.NotFound", "tenant-h-nsnf");
        let s = server(tenant);
        let err = s.node_status("ghost").unwrap_err();
        assert!(matches!(err, HealthError::NodeNotFound(_)));
    }

    #[test]
    fn health_node_status_records_latest_timestamp() {
        let (_c, tenant) = cilium_test_ctx!("pkg/health/server/server.go", "NodeStatus.LastProbed", "tenant-h-nsts");
        let mut s = server(tenant);
        s.register_node("node-a", ip(10, 0, 0, 1), ip(10, 0, 1, 1));
        s.record(make_result("node-a", ip(10, 0, 0, 1), ProbeProto::Icmp, ProbeStatus::Ok, 100));
        s.record(make_result("node-a", ip(10, 0, 1, 1), ProbeProto::Http, ProbeStatus::Ok, 200));
        let st = s.node_status("node-a").unwrap();
        assert_eq!(st.last_probed_ns, 200);
    }

    #[test]
    fn health_record_overwrites_prior_result_for_same_target() {
        let (_c, tenant) = cilium_test_ctx!("pkg/health/server/server.go", "Record.Overwrite", "tenant-h-recov");
        let mut s = server(tenant);
        s.register_node("node-a", ip(10, 0, 0, 1), ip(10, 0, 1, 1));
        s.record(make_result("node-a", ip(10, 0, 0, 1), ProbeProto::Icmp, ProbeStatus::Ok, 100));
        s.record(make_result("node-a", ip(10, 0, 0, 1), ProbeProto::Icmp, ProbeStatus::Unreachable, 200));
        let st = s.node_status("node-a").unwrap();
        assert_eq!(st.host.get("icmp").copied(), Some(ProbeStatus::Unreachable));
    }

    #[test]
    fn health_node_status_endpoint_status_recorded() {
        let (_c, tenant) = cilium_test_ctx!("pkg/health/server/server.go", "NodeStatus.Endpoint", "tenant-h-nsep");
        let mut s = server(tenant);
        s.register_node("node-a", ip(10, 0, 0, 1), ip(10, 0, 1, 1));
        s.record(make_result("node-a", ip(10, 0, 1, 1), ProbeProto::Http, ProbeStatus::Ok, 100));
        let st = s.node_status("node-a").unwrap();
        assert_eq!(st.endpoint.get("http").copied(), Some(ProbeStatus::Ok));
    }

    // ── Cluster status ──────────────────────────────────────────────────────

    #[test]
    fn health_cluster_status_counts_by_host_icmp() {
        let (_c, tenant) = cilium_test_ctx!("pkg/health/server/server.go", "ClusterStatus", "tenant-h-cs");
        let mut s = server(tenant);
        s.register_node("a", ip(10, 0, 0, 1), ip(10, 0, 1, 1));
        s.register_node("b", ip(10, 0, 0, 2), ip(10, 0, 1, 2));
        s.register_node("c", ip(10, 0, 0, 3), ip(10, 0, 1, 3));
        s.record(make_result("a", ip(10, 0, 0, 1), ProbeProto::Icmp, ProbeStatus::Ok, 0));
        s.record(make_result("b", ip(10, 0, 0, 2), ProbeProto::Icmp, ProbeStatus::Degraded, 0));
        s.record(make_result("c", ip(10, 0, 0, 3), ProbeProto::Icmp, ProbeStatus::Unreachable, 0));
        let cluster = s.cluster_status();
        assert_eq!(cluster.total, 3);
        assert_eq!(cluster.healthy, 1);
        assert_eq!(cluster.degraded, 1);
        assert_eq!(cluster.unreachable, 1);
    }

    #[test]
    fn health_cluster_status_unprobed_nodes_not_counted_in_buckets() {
        let (_c, tenant) = cilium_test_ctx!("pkg/health/server/server.go", "ClusterStatus.Unprobed", "tenant-h-csu");
        let mut s = server(tenant);
        s.register_node("a", ip(10, 0, 0, 1), ip(10, 0, 1, 1));
        let cluster = s.cluster_status();
        assert_eq!(cluster.total, 1);
        assert_eq!(cluster.healthy, 0);
        assert_eq!(cluster.degraded, 0);
        assert_eq!(cluster.unreachable, 0);
    }

    #[test]
    fn health_cluster_status_empty() {
        let (_c, tenant) = cilium_test_ctx!("pkg/health/server/server.go", "ClusterStatus.Empty", "tenant-h-cse");
        let s = server(tenant);
        let cluster = s.cluster_status();
        assert_eq!(cluster.total, 0);
    }

    // ── Multi-protocol per node ──────────────────────────────────────────────

    #[test]
    fn health_independent_results_per_protocol() {
        let (_c, tenant) = cilium_test_ctx!("pkg/health/server/server.go", "Record.MultiProto", "tenant-h-mp");
        let mut s = server(tenant);
        s.register_node("a", ip(10, 0, 0, 1), ip(10, 0, 1, 1));
        s.record(make_result("a", ip(10, 0, 0, 1), ProbeProto::Icmp, ProbeStatus::Ok, 100));
        s.record(make_result("a", ip(10, 0, 0, 1), ProbeProto::Http, ProbeStatus::Unreachable, 100));
        let st = s.node_status("a").unwrap();
        assert_eq!(st.host.get("icmp").copied(), Some(ProbeStatus::Ok));
        assert_eq!(st.host.get("http").copied(), Some(ProbeStatus::Unreachable));
    }

    // ── Lifecycle ────────────────────────────────────────────────────────────

    #[test]
    fn health_known_nodes_returns_registered_set() {
        let (_c, tenant) = cilium_test_ctx!("pkg/health/server/server.go", "KnownNodes", "tenant-h-kn");
        let mut s = server(tenant);
        for n in ["a", "b", "c"] {
            s.register_node(n, ip(10, 0, 0, 1), ip(10, 0, 1, 1));
        }
        let known: std::collections::HashSet<String> = s.known_nodes().into_iter().cloned().collect();
        assert_eq!(known.len(), 3);
    }

    #[test]
    fn health_register_replaces_existing_addresses() {
        let (_c, tenant) = cilium_test_ctx!("pkg/health/server/server.go", "RegisterNode.Replace", "tenant-h-rrep");
        let mut s = server(tenant);
        s.register_node("a", ip(10, 0, 0, 1), ip(10, 0, 1, 1));
        s.register_node("a", ip(10, 0, 0, 99), ip(10, 0, 1, 99));
        assert_eq!(s.node_count(), 1);
    }

    // ── Result count ─────────────────────────────────────────────────────────

    #[test]
    fn health_result_count_tracks_records() {
        let (_c, tenant) = cilium_test_ctx!("pkg/health/server/server.go", "Record.Count", "tenant-h-rcnt");
        let mut s = server(tenant);
        s.register_node("a", ip(10, 0, 0, 1), ip(10, 0, 1, 1));
        s.record(make_result("a", ip(10, 0, 0, 1), ProbeProto::Icmp, ProbeStatus::Ok, 100));
        s.record(make_result("a", ip(10, 0, 1, 1), ProbeProto::Http, ProbeStatus::Ok, 100));
        assert_eq!(s.result_count(), 2);
    }

    // ── Serde ────────────────────────────────────────────────────────────────

    #[test]
    fn probe_result_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/health/probe/probe.go", "Result.Serde", "tenant-h-rserde");
        let r = make_result("a", ip(10, 0, 0, 1), ProbeProto::Icmp, ProbeStatus::Ok, 100);
        let s = serde_json::to_string(&r).unwrap();
        let back: ProbeResult = serde_json::from_str(&s).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn cluster_health_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/health/server/server.go", "ClusterHealth.Serde", "tenant-h-cserde");
        let c = ClusterHealth { total: 10, healthy: 7, degraded: 2, unreachable: 1 };
        let s = serde_json::to_string(&c).unwrap();
        let back: ClusterHealth = serde_json::from_str(&s).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn node_status_serde_round_trip() {
        let (_c, tenant) = cilium_test_ctx!("pkg/health/server/server.go", "NodeStatus.Serde", "tenant-h-nserde");
        let mut s = server(tenant);
        s.register_node("a", ip(10, 0, 0, 1), ip(10, 0, 1, 1));
        s.record(make_result("a", ip(10, 0, 0, 1), ProbeProto::Icmp, ProbeStatus::Ok, 100));
        let st = s.node_status("a").unwrap();
        let json = serde_json::to_string(&st).unwrap();
        let back: NodeStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, st);
    }

    #[test]
    fn probe_status_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/health/server/server.go", "Status.Serde", "tenant-h-pserde");
        for st in [ProbeStatus::Ok, ProbeStatus::Unreachable, ProbeStatus::Degraded, ProbeStatus::Unknown] {
            let s = serde_json::to_string(&st).unwrap();
            let back: ProbeStatus = serde_json::from_str(&s).unwrap();
            assert_eq!(back, st);
        }
    }
}
