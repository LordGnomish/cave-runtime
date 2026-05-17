// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Hubble Prometheus metrics + `cilium monitor` event stream.
//!
//! Mirrors `pkg/hubble/metrics/api/api.go` (the metric registry +
//! per-handler `ProcessFlow`), `pkg/hubble/metrics/dns/dns.go`,
//! `pkg/hubble/metrics/http/http.go`, and the `cilium monitor`
//! event taxonomy from `pkg/monitor/api/types.go`.
//!
//! Surface (faithful to upstream):
//!
//! * [`MetricRegistry`] — tracks per-flow contributions to the named
//!   metrics: `flow`, `drop`, `dns`, `http`, `tcp`, `port-distribution`.
//! * Each metric exposes `samples()` returning `(labels, value)` pairs
//!   that map directly to the Prometheus exposition format that
//!   `pkg/hubble/metrics/server.go` serves.
//! * [`MonitorEvent`] enumerates the `MessageType*` codes emitted by
//!   the kernel-side `cilium monitor` ringbuffer (drop, debug, capture,
//!   trace, policy verdict, agent message, recorder).

use crate::cilium::hubble::{DropReason, FlowLog, Verdict};
use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ── MonitorEvent ─────────────────────────────────────────────────────────────

/// `cilium monitor` event types. Numeric codes match upstream
/// `pkg/monitor/api/types.go` constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MonitorEvent {
    /// `MessageTypeDrop` (1)
    Drop,
    /// `MessageTypeDebug` (2)
    Debug,
    /// `MessageTypeCapture` (3)
    Capture,
    /// `MessageTypeTrace` (4)
    Trace,
    /// `MessageTypeAgent` (5)
    Agent,
    /// `MessageTypeAccessLog` (6) — L7 access log entries.
    AccessLog,
    /// `MessageTypePolicyVerdict` (7)
    PolicyVerdict,
    /// `MessageTypeRecorder` (10) — pcap-style flow recorder.
    Recorder,
    /// `MessageTypeTraceSock` (11) — socket-level trace.
    TraceSock,
}

impl MonitorEvent {
    pub fn numeric(self) -> u8 {
        match self {
            MonitorEvent::Drop => 1,
            MonitorEvent::Debug => 2,
            MonitorEvent::Capture => 3,
            MonitorEvent::Trace => 4,
            MonitorEvent::Agent => 5,
            MonitorEvent::AccessLog => 6,
            MonitorEvent::PolicyVerdict => 7,
            MonitorEvent::Recorder => 10,
            MonitorEvent::TraceSock => 11,
        }
    }
    pub fn from_numeric(n: u8) -> Option<Self> {
        Some(match n {
            1 => MonitorEvent::Drop,
            2 => MonitorEvent::Debug,
            3 => MonitorEvent::Capture,
            4 => MonitorEvent::Trace,
            5 => MonitorEvent::Agent,
            6 => MonitorEvent::AccessLog,
            7 => MonitorEvent::PolicyVerdict,
            10 => MonitorEvent::Recorder,
            11 => MonitorEvent::TraceSock,
            _ => return None,
        })
    }
}

// ── Metric labels ────────────────────────────────────────────────────────────

/// A single Prometheus sample: ordered label-set + numeric value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricSample {
    pub name: String,
    pub labels: Vec<(String, String)>,
    pub value: f64,
}

impl MetricSample {
    pub fn new(name: impl Into<String>, labels: Vec<(String, String)>, value: f64) -> Self {
        Self { name: name.into(), labels, value }
    }
    /// Render in Prometheus exposition format: `name{k="v",...} value`.
    pub fn render(&self) -> String {
        let label_str = self.labels.iter()
            .map(|(k, v)| format!("{k}=\"{v}\""))
            .collect::<Vec<_>>()
            .join(",");
        if label_str.is_empty() {
            format!("{} {}", self.name, self.value)
        } else {
            format!("{}{{{}}} {}", self.name, label_str, self.value)
        }
    }
}

// ── Per-metric counters ──────────────────────────────────────────────────────

#[derive(Debug, Default, Clone)]
struct CounterMap {
    inner: BTreeMap<Vec<(String, String)>, u64>,
}

impl CounterMap {
    fn add(&mut self, labels: Vec<(String, String)>, by: u64) {
        let mut sorted = labels;
        sorted.sort();
        *self.inner.entry(sorted).or_default() += by;
    }
    fn samples(&self, name: &str) -> Vec<MetricSample> {
        self.inner
            .iter()
            .map(|(k, v)| MetricSample::new(name, k.clone(), *v as f64))
            .collect()
    }
}

#[derive(Debug, Default)]
pub struct MetricRegistry {
    flow: CounterMap,
    drop: CounterMap,
    dns: CounterMap,
    http: CounterMap,
    tcp_flags: CounterMap,
    port_distribution: CounterMap,
}

impl MetricRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Process one flow log, contributing to all enabled metrics.
    /// Mirrors `pkg/hubble/metrics/api/api.go::ProcessFlow`.
    pub fn process_flow(&mut self, f: &FlowLog) {
        let src_ns = namespace_of(&f.source_pod).to_string();
        let dst_ns = namespace_of(&f.destination_pod).to_string();
        let verdict_label = match f.verdict {
            Verdict::Forwarded => "FORWARDED",
            Verdict::Dropped => "DROPPED",
            Verdict::Error => "ERROR",
            Verdict::Audit => "AUDIT",
        };

        // hubble_flows_processed_total
        self.flow.add(
            vec![
                ("source_namespace".into(), src_ns.clone()),
                ("destination_namespace".into(), dst_ns.clone()),
                ("verdict".into(), verdict_label.into()),
            ],
            1,
        );

        // hubble_drop_total
        if matches!(f.verdict, Verdict::Dropped) {
            self.drop.add(
                vec![
                    ("source_namespace".into(), src_ns.clone()),
                    ("destination_namespace".into(), dst_ns.clone()),
                    ("reason".into(), drop_reason_label(f.drop_reason).into()),
                ],
                1,
            );
        }
    }

    /// Record an HTTP request observation for the L7 metrics.
    pub fn record_http(&mut self, src_ns: &str, dst_ns: &str, method: &str, status: u16) {
        self.http.add(
            vec![
                ("source_namespace".into(), src_ns.into()),
                ("destination_namespace".into(), dst_ns.into()),
                ("method".into(), method.into()),
                ("status".into(), status.to_string()),
            ],
            1,
        );
    }

    /// Record a DNS query observation.
    pub fn record_dns(&mut self, src_ns: &str, qtype: &str, rcode: &str) {
        self.dns.add(
            vec![
                ("source_namespace".into(), src_ns.into()),
                ("qtype".into(), qtype.into()),
                ("rcode".into(), rcode.into()),
            ],
            1,
        );
    }

    /// Record a TCP flag observation.
    pub fn record_tcp_flag(&mut self, flag: &str) {
        self.tcp_flags.add(vec![("flag".into(), flag.into())], 1);
    }

    /// Record port-distribution sample.
    pub fn record_port(&mut self, protocol: &str, port: u16) {
        self.port_distribution.add(
            vec![
                ("protocol".into(), protocol.into()),
                ("port".into(), port.to_string()),
            ],
            1,
        );
    }

    /// Render all metric families as Prometheus samples.
    pub fn samples(&self) -> Vec<MetricSample> {
        let mut out = Vec::new();
        out.extend(self.flow.samples("hubble_flows_processed_total"));
        out.extend(self.drop.samples("hubble_drop_total"));
        out.extend(self.dns.samples("hubble_dns_queries_total"));
        out.extend(self.http.samples("hubble_http_requests_total"));
        out.extend(self.tcp_flags.samples("hubble_tcp_flags_total"));
        out.extend(self.port_distribution.samples("hubble_port_distribution_total"));
        out
    }

    pub fn render_text(&self) -> String {
        self.samples().iter().map(|s| s.render()).collect::<Vec<_>>().join("\n")
    }
}

fn namespace_of(pod: &str) -> &str {
    pod.split('/').next().unwrap_or("")
}

fn drop_reason_label(r: DropReason) -> &'static str {
    match r {
        DropReason::None => "none",
        DropReason::PolicyDeny => "policy_denied",
        DropReason::Invalid => "invalid_packet",
        DropReason::CtInvalid => "ct_invalid",
        DropReason::FragmentationNeeded => "fragmentation_needed",
        DropReason::NatNoMapping => "nat_no_mapping",
        DropReason::AuthRequired => "auth_required",
        DropReason::Unknown(_) => "unknown",
    }
}

// ── Node summary (cluster-wide aggregate) ───────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeSummary {
    pub node: String,
    pub flows_total: u64,
    pub flows_forwarded: u64,
    pub flows_dropped: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub unique_identities: usize,
}

#[derive(Debug)]
pub struct NodeAggregator {
    pub tenant: TenantId,
    nodes: BTreeMap<String, NodeSummary>,
}

impl NodeAggregator {
    pub fn new(tenant: TenantId) -> Self {
        Self { tenant, nodes: BTreeMap::new() }
    }
    pub fn ingest(&mut self, node: &str, flow: &FlowLog) {
        if flow.tenant != self.tenant {
            return;
        }
        let summary = self.nodes.entry(node.to_string()).or_insert_with(|| NodeSummary {
            node: node.to_string(), ..Default::default()
        });
        summary.flows_total += 1;
        summary.bytes_out += flow.bytes;
        summary.bytes_in += flow.bytes;
        match flow.verdict {
            Verdict::Forwarded => summary.flows_forwarded += 1,
            Verdict::Dropped => summary.flows_dropped += 1,
            _ => {}
        }
    }
    pub fn summary(&self, node: &str) -> Option<&NodeSummary> {
        self.nodes.get(node)
    }
    pub fn all_summaries(&self) -> Vec<&NodeSummary> {
        self.nodes.values().collect()
    }
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/hubble/metrics/api/api.go", "MetricRegistry");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium::hubble::{DropReason, FlowLog, Verdict};
    use crate::cilium_test_ctx;
    use chrono::Utc;

    fn flow(tenant: &str, src_pod: &str, dst_pod: &str, src_id: u32, dst_id: u32, v: Verdict, dr: DropReason, bytes: u64) -> FlowLog {
        FlowLog {
            tenant: TenantId::new(tenant).expect("test fixture"), time: Utc::now(),
            source_identity: src_id, destination_identity: dst_id,
            source_pod: src_pod.into(), destination_pod: dst_pod.into(),
            verdict: v, drop_reason: dr, bytes,
        }
    }

    // ── MonitorEvent ─────────────────────────────────────────────────────────

    #[test]
    fn monitor_event_numeric_codes_match_upstream() {
        let (_c, _t) = cilium_test_ctx!("pkg/monitor/api/types.go", "MessageType", "tenant-mon-num");
        assert_eq!(MonitorEvent::Drop.numeric(), 1);
        assert_eq!(MonitorEvent::Debug.numeric(), 2);
        assert_eq!(MonitorEvent::Capture.numeric(), 3);
        assert_eq!(MonitorEvent::Trace.numeric(), 4);
        assert_eq!(MonitorEvent::Agent.numeric(), 5);
        assert_eq!(MonitorEvent::AccessLog.numeric(), 6);
        assert_eq!(MonitorEvent::PolicyVerdict.numeric(), 7);
        assert_eq!(MonitorEvent::Recorder.numeric(), 10);
        assert_eq!(MonitorEvent::TraceSock.numeric(), 11);
    }

    #[test]
    fn monitor_event_from_numeric_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/monitor/api/types.go", "MessageType.Parse", "tenant-mon-rt");
        for e in [
            MonitorEvent::Drop, MonitorEvent::Debug, MonitorEvent::Capture,
            MonitorEvent::Trace, MonitorEvent::Agent, MonitorEvent::AccessLog,
            MonitorEvent::PolicyVerdict, MonitorEvent::Recorder, MonitorEvent::TraceSock,
        ] {
            assert_eq!(MonitorEvent::from_numeric(e.numeric()), Some(e));
        }
    }

    #[test]
    fn monitor_event_unknown_code_returns_none() {
        let (_c, _t) = cilium_test_ctx!("pkg/monitor/api/types.go", "MessageType.Unknown", "tenant-mon-unk");
        assert!(MonitorEvent::from_numeric(99).is_none());
    }

    #[test]
    fn monitor_event_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/monitor/api/types.go", "MessageType.Serde", "tenant-mon-serde");
        for e in [MonitorEvent::Drop, MonitorEvent::PolicyVerdict, MonitorEvent::Recorder] {
            let s = serde_json::to_string(&e).unwrap();
            let back: MonitorEvent = serde_json::from_str(&s).unwrap();
            assert_eq!(back, e);
        }
    }

    // ── MetricSample render ──────────────────────────────────────────────────

    #[test]
    fn metric_sample_renders_no_labels() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/metrics/server.go", "Render.NoLabels", "tenant-met-nolab");
        let s = MetricSample::new("hubble_flows_total", vec![], 42.0);
        assert_eq!(s.render(), "hubble_flows_total 42");
    }

    #[test]
    fn metric_sample_renders_with_labels() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/metrics/server.go", "Render.Labels", "tenant-met-lab");
        let s = MetricSample::new("hubble_drop_total", vec![
            ("namespace".into(), "prod".into()),
            ("reason".into(), "policy_denied".into()),
        ], 7.0);
        assert!(s.render().starts_with("hubble_drop_total{"));
        assert!(s.render().contains("namespace=\"prod\""));
        assert!(s.render().contains("reason=\"policy_denied\""));
        assert!(s.render().ends_with(" 7"));
    }

    // ── flow processed metric ───────────────────────────────────────────────

    #[test]
    fn metrics_flow_processed_increments_counter() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/metrics/api/api.go", "ProcessFlow.FlowsTotal", "tenant-met-fp");
        let mut r = MetricRegistry::new();
        r.process_flow(&flow("t", "ns/a", "ns/b", 1, 2, Verdict::Forwarded, DropReason::None, 100));
        let samples = r.samples();
        let flow_sample = samples.iter().find(|s| s.name == "hubble_flows_processed_total").unwrap();
        assert_eq!(flow_sample.value, 1.0);
    }

    #[test]
    fn metrics_flow_processed_carries_verdict_label() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/metrics/api/api.go", "ProcessFlow.VerdictLabel", "tenant-met-fpv");
        let mut r = MetricRegistry::new();
        r.process_flow(&flow("t", "ns/a", "ns/b", 1, 2, Verdict::Dropped, DropReason::PolicyDeny, 100));
        let samples = r.samples();
        let flow_sample = samples.iter().find(|s| s.name == "hubble_flows_processed_total").unwrap();
        assert!(flow_sample.labels.iter().any(|(k, v)| k == "verdict" && v == "DROPPED"));
    }

    #[test]
    fn metrics_flow_processed_aggregates_repeated_label_sets() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/metrics/api/api.go", "ProcessFlow.Aggregate", "tenant-met-fpa");
        let mut r = MetricRegistry::new();
        for _ in 0..5 {
            r.process_flow(&flow("t", "ns/a", "ns/b", 1, 2, Verdict::Forwarded, DropReason::None, 100));
        }
        let s = r.samples();
        let flow_sample = s.iter().find(|s| s.name == "hubble_flows_processed_total").unwrap();
        assert_eq!(flow_sample.value, 5.0);
    }

    // ── drop metric ─────────────────────────────────────────────────────────

    #[test]
    fn metrics_drop_only_emitted_for_dropped_flows() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/metrics/drop/drop.go", "ProcessFlow.OnlyDropped", "tenant-met-d-only");
        let mut r = MetricRegistry::new();
        r.process_flow(&flow("t", "ns/a", "ns/b", 1, 2, Verdict::Forwarded, DropReason::None, 100));
        let s = r.samples();
        assert!(!s.iter().any(|s| s.name == "hubble_drop_total"));
    }

    #[test]
    fn metrics_drop_carries_reason_label() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/metrics/drop/drop.go", "ProcessFlow.ReasonLabel", "tenant-met-dr");
        let mut r = MetricRegistry::new();
        r.process_flow(&flow("t", "ns/a", "ns/b", 1, 2, Verdict::Dropped, DropReason::CtInvalid, 100));
        let s = r.samples();
        let drop_sample = s.iter().find(|s| s.name == "hubble_drop_total").unwrap();
        assert!(drop_sample.labels.iter().any(|(k, v)| k == "reason" && v == "ct_invalid"));
    }

    #[test]
    fn metrics_drop_aggregates_by_reason() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/metrics/drop/drop.go", "ProcessFlow.AggregateByReason", "tenant-met-dragg");
        let mut r = MetricRegistry::new();
        for _ in 0..3 {
            r.process_flow(&flow("t", "ns/a", "ns/b", 1, 2, Verdict::Dropped, DropReason::PolicyDeny, 100));
        }
        for _ in 0..2 {
            r.process_flow(&flow("t", "ns/a", "ns/b", 1, 2, Verdict::Dropped, DropReason::CtInvalid, 100));
        }
        let s = r.samples();
        let drops: Vec<_> = s.iter().filter(|s| s.name == "hubble_drop_total").collect();
        assert_eq!(drops.len(), 2);
    }

    // ── DNS metric ──────────────────────────────────────────────────────────

    #[test]
    fn metrics_dns_recorded_with_qtype_and_rcode() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/metrics/dns/dns.go", "RecordDNS", "tenant-met-dns");
        let mut r = MetricRegistry::new();
        r.record_dns("ns", "A", "NOERROR");
        let s = r.samples();
        let dns_sample = s.iter().find(|s| s.name == "hubble_dns_queries_total").unwrap();
        assert!(dns_sample.labels.iter().any(|(k, v)| k == "qtype" && v == "A"));
        assert!(dns_sample.labels.iter().any(|(k, v)| k == "rcode" && v == "NOERROR"));
    }

    #[test]
    fn metrics_dns_aggregates_by_qtype() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/metrics/dns/dns.go", "RecordDNS.Aggregate", "tenant-met-dnsa");
        let mut r = MetricRegistry::new();
        r.record_dns("ns", "A", "NOERROR");
        r.record_dns("ns", "A", "NOERROR");
        r.record_dns("ns", "AAAA", "NOERROR");
        let s = r.samples();
        let dns_samples: Vec<_> = s.iter().filter(|s| s.name == "hubble_dns_queries_total").collect();
        assert_eq!(dns_samples.len(), 2);
        let a_sample = dns_samples.iter().find(|s| s.labels.iter().any(|(k, v)| k == "qtype" && v == "A")).unwrap();
        assert_eq!(a_sample.value, 2.0);
    }

    // ── HTTP metric ─────────────────────────────────────────────────────────

    #[test]
    fn metrics_http_recorded_with_method_and_status() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/metrics/http/http.go", "RecordHTTP", "tenant-met-http");
        let mut r = MetricRegistry::new();
        r.record_http("ns-a", "ns-b", "GET", 200);
        let s = r.samples();
        let http_sample = s.iter().find(|s| s.name == "hubble_http_requests_total").unwrap();
        assert!(http_sample.labels.iter().any(|(k, v)| k == "method" && v == "GET"));
        assert!(http_sample.labels.iter().any(|(k, v)| k == "status" && v == "200"));
    }

    #[test]
    fn metrics_http_aggregates_by_method_status() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/metrics/http/http.go", "RecordHTTP.Aggregate", "tenant-met-httpa");
        let mut r = MetricRegistry::new();
        r.record_http("ns", "ns", "GET", 200);
        r.record_http("ns", "ns", "POST", 201);
        r.record_http("ns", "ns", "GET", 200);
        let s = r.samples();
        let http_samples: Vec<_> = s.iter().filter(|s| s.name == "hubble_http_requests_total").collect();
        assert_eq!(http_samples.len(), 2);
    }

    // ── TCP flags ───────────────────────────────────────────────────────────

    #[test]
    fn metrics_tcp_flag_recorded() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/metrics/tcp/tcp.go", "RecordTcpFlag", "tenant-met-tcp");
        let mut r = MetricRegistry::new();
        r.record_tcp_flag("SYN");
        let s = r.samples();
        let tcp_sample = s.iter().find(|s| s.name == "hubble_tcp_flags_total").unwrap();
        assert_eq!(tcp_sample.labels[0], ("flag".into(), "SYN".into()));
    }

    // ── Port distribution ───────────────────────────────────────────────────

    #[test]
    fn metrics_port_distribution_recorded() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/metrics/port-distribution/port-distribution.go", "Record", "tenant-met-port");
        let mut r = MetricRegistry::new();
        r.record_port("TCP", 80);
        let s = r.samples();
        let port_sample = s.iter().find(|s| s.name == "hubble_port_distribution_total").unwrap();
        assert!(port_sample.labels.iter().any(|(k, v)| k == "port" && v == "80"));
    }

    // ── Render exposition format ────────────────────────────────────────────

    #[test]
    fn metrics_render_text_includes_all_families() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/metrics/server.go", "RenderText", "tenant-met-render");
        let mut r = MetricRegistry::new();
        r.process_flow(&flow("t", "ns/a", "ns/b", 1, 2, Verdict::Forwarded, DropReason::None, 100));
        r.process_flow(&flow("t", "ns/a", "ns/b", 1, 2, Verdict::Dropped, DropReason::PolicyDeny, 100));
        r.record_http("ns-a", "ns-b", "GET", 200);
        r.record_dns("ns", "A", "NOERROR");
        let text = r.render_text();
        assert!(text.contains("hubble_flows_processed_total"));
        assert!(text.contains("hubble_drop_total"));
        assert!(text.contains("hubble_http_requests_total"));
        assert!(text.contains("hubble_dns_queries_total"));
    }

    #[test]
    fn metrics_render_text_well_formed_lines() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/metrics/server.go", "RenderText.WellFormed", "tenant-met-rwf");
        let mut r = MetricRegistry::new();
        r.process_flow(&flow("t", "ns/a", "ns/b", 1, 2, Verdict::Forwarded, DropReason::None, 100));
        for line in r.render_text().lines() {
            assert!(line.contains(' '));
            // The metric name is the first token; values follow after the last space.
            let parts: Vec<&str> = line.rsplitn(2, ' ').collect();
            assert!(parts[0].parse::<f64>().is_ok());
        }
    }

    // ── Drop reason labels ──────────────────────────────────────────────────

    #[test]
    fn drop_reason_label_for_known_codes() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/metrics/drop/drop.go", "DropReasonLabel", "tenant-met-drlbl");
        assert_eq!(drop_reason_label(DropReason::None), "none");
        assert_eq!(drop_reason_label(DropReason::PolicyDeny), "policy_denied");
        assert_eq!(drop_reason_label(DropReason::Invalid), "invalid_packet");
        assert_eq!(drop_reason_label(DropReason::CtInvalid), "ct_invalid");
        assert_eq!(drop_reason_label(DropReason::FragmentationNeeded), "fragmentation_needed");
        assert_eq!(drop_reason_label(DropReason::NatNoMapping), "nat_no_mapping");
        assert_eq!(drop_reason_label(DropReason::AuthRequired), "auth_required");
        assert_eq!(drop_reason_label(DropReason::Unknown(42)), "unknown");
    }

    // ── Node aggregator ─────────────────────────────────────────────────────

    #[test]
    fn node_aggregator_ingests_flow_per_node() {
        let (_c, tenant) = cilium_test_ctx!("pkg/hubble/relay/aggregate.go", "NodeAggregator.Ingest", "tenant-na-ing");
        let mut a = NodeAggregator::new(tenant.clone());
        a.ingest("node-a", &flow(tenant.as_str(), "ns/a", "ns/b", 1, 2, Verdict::Forwarded, DropReason::None, 100));
        a.ingest("node-b", &flow(tenant.as_str(), "ns/a", "ns/b", 1, 2, Verdict::Forwarded, DropReason::None, 200));
        assert_eq!(a.node_count(), 2);
        assert_eq!(a.summary("node-a").unwrap().bytes_out, 100);
        assert_eq!(a.summary("node-b").unwrap().bytes_out, 200);
    }

    #[test]
    fn node_aggregator_filters_cross_tenant() {
        let (_c, tenant) = cilium_test_ctx!("pkg/hubble/relay/aggregate.go", "NodeAggregator.Tenant", "tenant-na-iso");
        let mut a = NodeAggregator::new(tenant);
        a.ingest("node-a", &flow("other", "ns/a", "ns/b", 1, 2, Verdict::Forwarded, DropReason::None, 100));
        assert_eq!(a.node_count(), 0);
    }

    #[test]
    fn node_aggregator_tracks_forwarded_and_dropped() {
        let (_c, tenant) = cilium_test_ctx!("pkg/hubble/relay/aggregate.go", "NodeAggregator.Verdict", "tenant-na-v");
        let mut a = NodeAggregator::new(tenant.clone());
        a.ingest("node-a", &flow(tenant.as_str(), "ns/a", "ns/b", 1, 2, Verdict::Forwarded, DropReason::None, 100));
        a.ingest("node-a", &flow(tenant.as_str(), "ns/a", "ns/b", 1, 2, Verdict::Dropped, DropReason::PolicyDeny, 100));
        let s = a.summary("node-a").unwrap();
        assert_eq!(s.flows_total, 2);
        assert_eq!(s.flows_forwarded, 1);
        assert_eq!(s.flows_dropped, 1);
    }

    #[test]
    fn node_aggregator_summary_unknown_node_returns_none() {
        let (_c, tenant) = cilium_test_ctx!("pkg/hubble/relay/aggregate.go", "NodeAggregator.NotFound", "tenant-na-nf");
        let a = NodeAggregator::new(tenant);
        assert!(a.summary("unknown").is_none());
    }

    #[test]
    fn node_aggregator_all_summaries_returns_each_node() {
        let (_c, tenant) = cilium_test_ctx!("pkg/hubble/relay/aggregate.go", "NodeAggregator.All", "tenant-na-all");
        let mut a = NodeAggregator::new(tenant.clone());
        for n in ["node-a", "node-b", "node-c"] {
            a.ingest(n, &flow(tenant.as_str(), "ns/a", "ns/b", 1, 2, Verdict::Forwarded, DropReason::None, 100));
        }
        assert_eq!(a.all_summaries().len(), 3);
    }

    // ── Serde ───────────────────────────────────────────────────────────────

    #[test]
    fn metric_sample_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/metrics/api/api.go", "MetricSample.Serde", "tenant-met-mss");
        let s = MetricSample::new("foo", vec![("k".into(), "v".into())], 42.0);
        let json = serde_json::to_string(&s).unwrap();
        let back: MetricSample = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, s.name);
        assert_eq!(back.labels, s.labels);
    }

    #[test]
    fn node_summary_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/relay/aggregate.go", "NodeSummary.Serde", "tenant-na-ss");
        let s = NodeSummary {
            node: "node-a".into(),
            flows_total: 100, flows_forwarded: 80, flows_dropped: 20,
            bytes_in: 1000, bytes_out: 1000, unique_identities: 5,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: NodeSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    // ── Bytes accumulation ──────────────────────────────────────────────────

    #[test]
    fn node_summary_bytes_accumulate_across_flows() {
        let (_c, tenant) = cilium_test_ctx!("pkg/hubble/relay/aggregate.go", "NodeSummary.Bytes", "tenant-na-bytes");
        let mut a = NodeAggregator::new(tenant.clone());
        for i in 1..=5u64 {
            a.ingest("node-a", &flow(tenant.as_str(), "ns/a", "ns/b", 1, 2, Verdict::Forwarded, DropReason::None, i * 100));
        }
        let s = a.summary("node-a").unwrap();
        assert_eq!(s.bytes_out, 100 + 200 + 300 + 400 + 500);
    }

    // ── Combined: registry + render ─────────────────────────────────────────

    #[test]
    fn metrics_registry_label_set_ordering_is_deterministic() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/metrics/api/api.go", "ProcessFlow.LabelOrder", "tenant-met-lord");
        let mut r1 = MetricRegistry::new();
        let mut r2 = MetricRegistry::new();
        // Process the same flow twice → same key, count = 2.
        r1.process_flow(&flow("t", "ns/a", "ns/b", 1, 2, Verdict::Forwarded, DropReason::None, 100));
        r1.process_flow(&flow("t", "ns/a", "ns/b", 1, 2, Verdict::Forwarded, DropReason::None, 100));
        r2.process_flow(&flow("t", "ns/a", "ns/b", 1, 2, Verdict::Forwarded, DropReason::None, 100));
        r2.process_flow(&flow("t", "ns/a", "ns/b", 1, 2, Verdict::Forwarded, DropReason::None, 100));
        assert_eq!(r1.render_text(), r2.render_text());
    }

    #[test]
    fn metrics_drop_aggregates_namespace_label_too() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/metrics/drop/drop.go", "ProcessFlow.NamespaceLabel", "tenant-met-drns");
        let mut r = MetricRegistry::new();
        r.process_flow(&flow("t", "prod/a", "prod/b", 1, 2, Verdict::Dropped, DropReason::PolicyDeny, 100));
        r.process_flow(&flow("t", "stage/a", "stage/b", 1, 2, Verdict::Dropped, DropReason::PolicyDeny, 100));
        let s = r.samples();
        let drops: Vec<_> = s.iter().filter(|s| s.name == "hubble_drop_total").collect();
        assert_eq!(drops.len(), 2);
    }

    #[test]
    fn monitor_event_unknown_codes_skip_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/monitor/api/types.go", "MessageType.UnknownPath", "tenant-mon-unkp");
        for c in [0u8, 8, 9, 12, 99, 200] {
            assert!(MonitorEvent::from_numeric(c).is_none());
        }
    }
}
