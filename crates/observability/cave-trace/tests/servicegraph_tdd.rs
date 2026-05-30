// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Strict-TDD parity port — Grafana Tempo service-graph metrics processor.
//!
//! Upstream: grafana/tempo modules/generator/processor/servicegraphs
//!   - servicegraphs.go (Processor.consume: client/server edge matching)
//!   - store/store.go    (edge store + expiration / onComplete callbacks)
//!   - edge.go           (Edge.isComplete: both client+server services set)
//!
//! The processor matches a CLIENT span to its corresponding SERVER span by
//! the connecting span id (server.parentSpanID == client.spanID, within the
//! same trace). When both halves of an edge arrive it is "complete" → the
//! request_total / request_failed_total counters and the server/client
//! latency histograms are observed for the (client, server) pair and the edge
//! is evicted. Edges that never complete expire after the configured TTL.
//!
//! RED commit: this file references `cave_trace::servicegraph::*` which does
//! not exist yet — the crate fails to compile, so every test is RED.

use cave_trace::servicegraph::{ServiceGraphProcessor, DEFAULT_LATENCY_BUCKETS_SEC};
use cave_trace::types::{Span, SpanKind, SpanStatus, TagValue};
use std::collections::HashMap;

fn span(
    trace_id: u128,
    span_id: u64,
    parent: Option<u64>,
    service: &str,
    kind: SpanKind,
    dur_ns: u64,
    error: bool,
) -> Span {
    Span {
        trace_id,
        span_id,
        parent_span_id: parent,
        operation_name: "op".into(),
        service_name: service.into(),
        start_time_unix_nano: 1_000_000_000,
        end_time_unix_nano: 1_000_000_000 + dur_ns,
        duration_ns: dur_ns,
        status: if error {
            SpanStatus::Error
        } else {
            SpanStatus::Ok
        },
        kind,
        tags: HashMap::new(),
        events: vec![],
        links: vec![],
        resource_attributes: HashMap::new(),
        tenant_id: "default".into(),
        baggage: HashMap::new(),
        log_labels: HashMap::new(),
    }
}

// ── 1. A matching client+server pair completes exactly one edge ──────────────

#[test]
fn client_server_pair_completes_one_edge() {
    let mut p = ServiceGraphProcessor::new();
    // client span (id=10) in service "frontend"
    let client = span(1, 10, None, "frontend", SpanKind::Client, 50_000_000, false);
    // server span whose parent is the client span (id=10) in service "backend"
    let server = span(1, 11, Some(10), "backend", SpanKind::Server, 30_000_000, false);

    p.consume_at(&[client, server], 0);

    let edges = p.edge_metrics();
    assert_eq!(edges.len(), 1, "exactly one completed edge expected");
    let e = &edges[0];
    assert_eq!(e.client, "frontend");
    assert_eq!(e.server, "backend");
    assert_eq!(e.count, 1);
    assert_eq!(e.failed, 0);
    // the edge was completed → no longer pending in the store
    assert_eq!(p.pending_len(), 0);
    assert_eq!(p.total_completed(), 1);
}

// ── 2. A lone client span stays pending (no completed edge) ──────────────────

#[test]
fn lone_client_span_stays_pending() {
    let mut p = ServiceGraphProcessor::new();
    let client = span(1, 10, None, "frontend", SpanKind::Client, 50_000_000, false);
    p.consume_at(&[client], 0);

    assert_eq!(p.edge_metrics().len(), 0, "no edge completes without a server half");
    assert_eq!(p.pending_len(), 1, "the client half is held pending");
    assert_eq!(p.total_completed(), 0);
}

// ── 3. Edge.failed is set if either half errored ─────────────────────────────

#[test]
fn edge_failed_when_server_errors() {
    let mut p = ServiceGraphProcessor::new();
    let client = span(2, 20, None, "web", SpanKind::Client, 10_000_000, false);
    let server = span(2, 21, Some(20), "api", SpanKind::Server, 9_000_000, true);
    p.consume_at(&[client, server], 0);

    let edges = p.edge_metrics();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].failed, 1, "server error must mark the edge failed");
    assert_eq!(edges[0].count, 1);
}

// ── 4. Matching is keyed on (trace_id, connecting span id) ───────────────────
//      A server with the same parent id but a DIFFERENT trace must not match.

#[test]
fn matching_is_scoped_to_trace_id() {
    let mut p = ServiceGraphProcessor::new();
    let client = span(100, 10, None, "frontend", SpanKind::Client, 5_000_000, false);
    // same connecting id (10) but a different trace → must NOT complete the edge
    let server_other_trace =
        span(999, 11, Some(10), "backend", SpanKind::Server, 4_000_000, false);
    p.consume_at(&[client, server_other_trace], 0);

    assert_eq!(p.edge_metrics().len(), 0);
    assert_eq!(p.pending_len(), 2, "two unrelated halves remain pending");
}

// ── 5. Pending edges expire after the TTL and are counted as expired ─────────

#[test]
fn pending_edge_expires_after_ttl() {
    let mut p = ServiceGraphProcessor::with_ttl_nanos(10_000_000_000); // 10s
    let client = span(3, 30, None, "frontend", SpanKind::Client, 1_000_000, false);
    p.consume_at(&[client], 0);
    assert_eq!(p.pending_len(), 1);

    // not yet expired (5s < 10s)
    p.expire_at(5_000_000_000);
    assert_eq!(p.pending_len(), 1);
    assert_eq!(p.total_expired(), 0);

    // past the TTL (11s > 10s) → dropped + counted
    p.expire_at(11_000_000_000);
    assert_eq!(p.pending_len(), 0);
    assert_eq!(p.total_expired(), 1);
}

// ── 6. Server latency is observed into the correct cumulative bucket ─────────

#[test]
fn server_latency_lands_in_histogram_bucket() {
    let mut p = ServiceGraphProcessor::new();
    // server latency 0.3s → buckets [0.1,0.2,0.4,...] cumulative count at le>=0.4
    let client = span(4, 40, None, "a", SpanKind::Client, 500_000_000, false);
    let server = span(4, 41, Some(40), "b", SpanKind::Server, 300_000_000, false);
    p.consume_at(&[client, server], 0);

    let e = &p.edge_metrics()[0];
    // cumulative bucket count: 0 at le=0.1, 0 at le=0.2, 1 at le=0.4 ... 1 at +Inf
    let cumulative = e.server_latency.cumulative_counts();
    assert_eq!(cumulative.len(), DEFAULT_LATENCY_BUCKETS_SEC.len() + 1); // +Inf bucket
    let idx_0_2 = DEFAULT_LATENCY_BUCKETS_SEC
        .iter()
        .position(|&b| (b - 0.2).abs() < 1e-9)
        .unwrap();
    let idx_0_4 = DEFAULT_LATENCY_BUCKETS_SEC
        .iter()
        .position(|&b| (b - 0.4).abs() < 1e-9)
        .unwrap();
    assert_eq!(cumulative[idx_0_2], 0, "0.3s must not fall in the le=0.2 bucket");
    assert_eq!(cumulative[idx_0_4], 1, "0.3s falls in the le=0.4 bucket");
    assert_eq!(*cumulative.last().unwrap(), 1, "+Inf bucket counts all");
    assert!((e.server_latency.sum_sec - 0.3).abs() < 1e-6);
    assert_eq!(e.server_latency.count, 1);
}

// ── 7. Repeated identical edges accumulate; distinct edges separate ──────────

#[test]
fn repeated_and_distinct_edges_accumulate() {
    let mut p = ServiceGraphProcessor::new();
    // two frontend→backend calls (distinct traces) + one frontend→cache call
    p.consume_at(
        &[
            span(1, 1, None, "frontend", SpanKind::Client, 1_000_000, false),
            span(1, 2, Some(1), "backend", SpanKind::Server, 1_000_000, false),
        ],
        0,
    );
    p.consume_at(
        &[
            span(2, 1, None, "frontend", SpanKind::Client, 1_000_000, true),
            span(2, 2, Some(1), "backend", SpanKind::Server, 1_000_000, false),
        ],
        0,
    );
    p.consume_at(
        &[
            span(3, 1, None, "frontend", SpanKind::Client, 1_000_000, false),
            span(3, 2, Some(1), "cache", SpanKind::Server, 1_000_000, false),
        ],
        0,
    );

    let mut edges = p.edge_metrics();
    edges.sort_by(|a, b| a.server.cmp(&b.server));
    assert_eq!(edges.len(), 2, "frontend→backend and frontend→cache");
    let backend = edges.iter().find(|e| e.server == "backend").unwrap();
    assert_eq!(backend.count, 2);
    assert_eq!(backend.failed, 1, "one of the two backend calls failed");
    let cache = edges.iter().find(|e| e.server == "cache").unwrap();
    assert_eq!(cache.count, 1);
    assert_eq!(cache.failed, 0);
}

// ── 8. Prometheus exposition carries the Tempo metric names ──────────────────

#[test]
fn prometheus_exposition_has_tempo_metric_names() {
    let mut p = ServiceGraphProcessor::new();
    p.consume_at(
        &[
            span(1, 1, None, "frontend", SpanKind::Client, 250_000_000, false),
            span(1, 2, Some(1), "backend", SpanKind::Server, 200_000_000, true),
        ],
        0,
    );
    let prom = p.to_prometheus();
    assert!(prom.contains("traces_service_graph_request_total"));
    assert!(prom.contains("traces_service_graph_request_failed_total"));
    assert!(prom.contains("traces_service_graph_request_server_seconds_bucket"));
    assert!(prom.contains("traces_service_graph_request_client_seconds_bucket"));
    assert!(prom.contains(r#"client="frontend""#));
    assert!(prom.contains(r#"server="backend""#));
    // failed counter must read 1 for this errored edge
    assert!(
        prom.lines()
            .any(|l| l.starts_with("traces_service_graph_request_failed_total") && l.ends_with(" 1"))
    );
}

// ── 9. A producer/consumer (messaging) pair also forms an edge ───────────────
//      Tempo treats CONSUMER spans like SERVER (callee) for matching.

#[test]
fn producer_consumer_messaging_edge() {
    let mut p = ServiceGraphProcessor::new();
    let producer = span(5, 50, None, "shipping", SpanKind::Producer, 2_000_000, false);
    let consumer = span(5, 51, Some(50), "email", SpanKind::Consumer, 3_000_000, false);
    p.consume_at(&[producer, consumer], 0);

    let edges = p.edge_metrics();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].client, "shipping");
    assert_eq!(edges[0].server, "email");
}

// keep the unused import meaningful even if a test path changes
#[allow(dead_code)]
fn _tagvalue_anchor() -> TagValue {
    TagValue::Bool(true)
}
