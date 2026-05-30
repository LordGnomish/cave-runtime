// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Service-graph metrics processor — Grafana Tempo parity.
//!
//! Line-port of grafana/tempo `modules/generator/processor/servicegraphs`
//! (servicegraphs.go + store/store.go + edge.go). The processor reconstructs
//! a service dependency graph with RED-style edge metrics directly from spans,
//! the way Tempo's metrics-generator does:
//!
//!   * A CLIENT (or PRODUCER) span contributes the *caller* half of an edge,
//!     keyed by `(trace_id, span_id)` — the connecting span.
//!   * A SERVER (or CONSUMER) span contributes the *callee* half, keyed by
//!     `(trace_id, parent_span_id)` — i.e. the server's parent is the client
//!     span that called it.
//!   * When both halves land the edge is *complete*: the `request_total` /
//!     `request_failed_total` counters and the server/client latency
//!     histograms are observed for the `(client, server)` pair and the edge
//!     is evicted from the in-flight store.
//!   * Edges that never complete expire after a TTL (Tempo default `wait`
//!     = 10 s) and are counted as expired (dropped).
//!
//! The live Prometheus remote-write exporter + the generator's WAL stay in
//! cave-metrics / Phase 3; this module is the pure edge-matching + metric
//! accumulation algorithm, exposed through Prometheus text exposition.

use std::collections::HashMap;

use crate::types::{Span, SpanKind};

/// Tempo's default service-graph latency buckets, in seconds.
/// (`modules/generator/processor/servicegraphs/config.go` HistogramBuckets.)
pub const DEFAULT_LATENCY_BUCKETS_SEC: &[f64] =
    &[0.1, 0.2, 0.4, 0.8, 1.6, 3.2, 6.4, 12.8];

/// Tempo's default `wait` before an unmatched edge expires (10 s), in ns.
pub const DEFAULT_EDGE_TTL_NANOS: u64 = 10_000_000_000;

// ─── Latency histogram ──────────────────────────────────────────────────────

/// A fixed-bucket latency histogram observed in seconds, mirroring the
/// Prometheus client histogram Tempo emits per edge.
#[derive(Debug, Clone)]
pub struct LatencyHistogram {
    /// Upper bounds in seconds (exclusive of the implicit +Inf bucket).
    pub buckets_sec: Vec<f64>,
    /// Per-bucket (non-cumulative) counts; `len == buckets_sec.len() + 1`,
    /// the last slot being the implicit `+Inf` bucket.
    counts: Vec<u64>,
    /// Number of observations.
    pub count: u64,
    /// Sum of observed values in seconds.
    pub sum_sec: f64,
}

impl LatencyHistogram {
    pub fn new(buckets_sec: &[f64]) -> Self {
        LatencyHistogram {
            buckets_sec: buckets_sec.to_vec(),
            counts: vec![0; buckets_sec.len() + 1],
            count: 0,
            sum_sec: 0.0,
        }
    }

    /// Observe a latency value (seconds) into the first bucket whose upper
    /// bound it does not exceed; values larger than every bound land in the
    /// implicit `+Inf` bucket.
    pub fn observe(&mut self, value_sec: f64) {
        let idx = self
            .buckets_sec
            .iter()
            .position(|&le| value_sec <= le)
            .unwrap_or(self.buckets_sec.len());
        self.counts[idx] += 1;
        self.count += 1;
        self.sum_sec += value_sec;
    }

    /// Cumulative bucket counts (Prometheus `le` semantics), including the
    /// trailing `+Inf` bucket; `len == buckets_sec.len() + 1`.
    pub fn cumulative_counts(&self) -> Vec<u64> {
        let mut acc = 0u64;
        self.counts
            .iter()
            .map(|&c| {
                acc += c;
                acc
            })
            .collect()
    }
}

// ─── Completed-edge metric ────────────────────────────────────────────────────

/// Accumulated metrics for a `(client, server)` edge.
#[derive(Debug, Clone)]
pub struct EdgeMetric {
    pub client: String,
    pub server: String,
    /// `traces_service_graph_request_total`.
    pub count: u64,
    /// `traces_service_graph_request_failed_total`.
    pub failed: u64,
    pub server_latency: LatencyHistogram,
    pub client_latency: LatencyHistogram,
}

// ─── In-flight pending edge ───────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
struct PendingEdge {
    client_service: String,
    server_service: String,
    client_latency_sec: f64,
    server_latency_sec: f64,
    failed: bool,
    expires_at: u64,
}

impl PendingEdge {
    fn is_complete(&self) -> bool {
        !self.client_service.is_empty() && !self.server_service.is_empty()
    }
}

/// Which half of an edge a span contributes.
enum Half {
    /// Caller (CLIENT / PRODUCER): keyed by the span's own id.
    Client,
    /// Callee (SERVER / CONSUMER): keyed by the span's parent id.
    Server,
    /// INTERNAL and unspecified spans do not form graph edges.
    Skip,
}

fn half_for(kind: SpanKind) -> Half {
    match kind {
        SpanKind::Client | SpanKind::Producer => Half::Client,
        SpanKind::Server | SpanKind::Consumer => Half::Server,
        SpanKind::Internal => Half::Skip,
    }
}

fn edge_key(trace_id: u128, connecting_span: u64) -> String {
    format!("{:032x}{:016x}", trace_id, connecting_span)
}

// ─── Processor ────────────────────────────────────────────────────────────────

/// Stateful service-graph processor: feed it spans, read edge metrics.
pub struct ServiceGraphProcessor {
    ttl_nanos: u64,
    pending: HashMap<String, PendingEdge>,
    metrics: HashMap<(String, String), EdgeMetric>,
    completed: u64,
    expired: u64,
}

impl Default for ServiceGraphProcessor {
    fn default() -> Self {
        Self::with_ttl_nanos(DEFAULT_EDGE_TTL_NANOS)
    }
}

impl ServiceGraphProcessor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_ttl_nanos(ttl_nanos: u64) -> Self {
        ServiceGraphProcessor {
            ttl_nanos,
            pending: HashMap::new(),
            metrics: HashMap::new(),
            completed: 0,
            expired: 0,
        }
    }

    /// Consume a batch of spans at wall-clock `now_nanos` (used for the edge
    /// expiry deadline). Completed edges are folded into metrics immediately.
    pub fn consume_at(&mut self, spans: &[Span], now_nanos: u64) {
        for span in spans {
            let (key, is_client) = match half_for(span.kind) {
                Half::Client => (edge_key(span.trace_id, span.span_id), true),
                Half::Server => match span.parent_span_id {
                    // A root server span has no caller in this trace → no edge.
                    Some(parent) => (edge_key(span.trace_id, parent), false),
                    None => continue,
                },
                Half::Skip => continue,
            };

            let entry = self.pending.entry(key.clone()).or_insert_with(|| PendingEdge {
                expires_at: now_nanos.saturating_add(self.ttl_nanos),
                ..PendingEdge::default()
            });

            if is_client {
                entry.client_service = span.service_name.clone();
                entry.client_latency_sec = span.duration_sec();
            } else {
                entry.server_service = span.service_name.clone();
                entry.server_latency_sec = span.duration_sec();
            }
            entry.failed |= span.has_error();

            if entry.is_complete() {
                let edge = self.pending.remove(&key).expect("just inserted");
                self.record(edge);
            }
        }
    }

    /// Convenience: consume with `now_nanos` taken from the system clock.
    pub fn consume(&mut self, spans: &[Span]) {
        self.consume_at(spans, now_ns());
    }

    /// Drop any pending edge whose deadline has been reached at `now_nanos`,
    /// counting each as expired.
    pub fn expire_at(&mut self, now_nanos: u64) {
        let stale: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, e)| now_nanos >= e.expires_at)
            .map(|(k, _)| k.clone())
            .collect();
        for k in stale {
            self.pending.remove(&k);
            self.expired += 1;
        }
    }

    fn record(&mut self, edge: PendingEdge) {
        let buckets = DEFAULT_LATENCY_BUCKETS_SEC;
        let m = self
            .metrics
            .entry((edge.client_service.clone(), edge.server_service.clone()))
            .or_insert_with(|| EdgeMetric {
                client: edge.client_service.clone(),
                server: edge.server_service.clone(),
                count: 0,
                failed: 0,
                server_latency: LatencyHistogram::new(buckets),
                client_latency: LatencyHistogram::new(buckets),
            });
        m.count += 1;
        if edge.failed {
            m.failed += 1;
        }
        m.server_latency.observe(edge.server_latency_sec);
        m.client_latency.observe(edge.client_latency_sec);
        self.completed += 1;
    }

    /// Snapshot of all accumulated edge metrics.
    pub fn edge_metrics(&self) -> Vec<EdgeMetric> {
        self.metrics.values().cloned().collect()
    }

    /// Number of in-flight (incomplete) edges still awaiting their other half.
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// Total edges completed since construction.
    pub fn total_completed(&self) -> u64 {
        self.completed
    }

    /// Total edges expired (dropped without completing) since construction.
    pub fn total_expired(&self) -> u64 {
        self.expired
    }

    /// Prometheus text exposition mirroring Tempo's service-graph metric names.
    pub fn to_prometheus(&self) -> String {
        let mut out = String::new();
        let mut edges = self.edge_metrics();
        edges.sort_by(|a, b| (&a.client, &a.server).cmp(&(&b.client, &b.server)));
        for e in &edges {
            let labels = format!(
                r#"client="{}",server="{}""#,
                escape_label(&e.client),
                escape_label(&e.server)
            );
            out.push_str(&format!(
                "traces_service_graph_request_total{{{}}} {}\n",
                labels, e.count
            ));
            out.push_str(&format!(
                "traces_service_graph_request_failed_total{{{}}} {}\n",
                labels, e.failed
            ));
            write_histogram(
                &mut out,
                "traces_service_graph_request_server_seconds",
                &labels,
                &e.server_latency,
            );
            write_histogram(
                &mut out,
                "traces_service_graph_request_client_seconds",
                &labels,
                &e.client_latency,
            );
        }
        out
    }
}

fn write_histogram(out: &mut String, name: &str, labels: &str, h: &LatencyHistogram) {
    let cum = h.cumulative_counts();
    for (i, &c) in cum.iter().enumerate() {
        let le = if i < h.buckets_sec.len() {
            format!("{}", h.buckets_sec[i])
        } else {
            "+Inf".to_owned()
        };
        out.push_str(&format!("{}_bucket{{{},le=\"{}\"}} {}\n", name, labels, le, c));
    }
    out.push_str(&format!("{}_sum{{{}}} {}\n", name, labels, h.sum_sec));
    out.push_str(&format!("{}_count{{{}}} {}\n", name, labels, h.count));
}

fn escape_label(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

fn now_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}
