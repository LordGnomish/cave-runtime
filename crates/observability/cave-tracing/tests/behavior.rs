// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Portable-coverage TDD fills for `cave-tracing`, derived from the
//! OpenTelemetry/W3C-portable behaviors exercised by Jaeger's test suite
//! (upstream: jaegertracing/jaeger @ v2.17.0).
//!
//! cave-tracing is the sovereign OTel-compatible *producer* SDK; the vast
//! majority of Jaeger v2.17.0 collector/storage/query tests are scope-cut.
//! These tests target the narrow already-implemented, currently-untested
//! portable surface flagged by the gap audit:
//!
//!   * `exporter::OtlpHttpExporter::render_payload` — OTLP `links` array
//!     serialization (`traceId` / `spanId` / link attributes).
//!   * `exporter::OtlpHttpExporter::render_payload` — `parentSpanId` field
//!     (16-hex for a child span, empty string for a root span).
//!   * `exporter::OtlpHttpExporter::render_payload` — distinct tenants split
//!     into separate `resourceSpans` groups (tenant is part of the group key).
//!   * `propagation::TraceState::to_header` — HEAD-of-list ordering and
//!     `parse_tracestate(to_header())` round-trip via `get`.
//!
//! Every expected value below is derived from the source implementation:
//!   - `format_span_id(0xabcd)  == "000000000000abcd"`
//!   - `format_span_id(0x3344)  == "0000000000003344"`
//!   - `format_trace_id(0x1122) == "00000000000000000000000000001122"`
//!   - root span `parent_span_id: None` -> `parentSpanId == ""`
//!     (`Option::map(format_span_id).unwrap_or_default()`).
//!   - `TraceState::upsert` inserts the newest key at index 0, so
//!     `to_header()` emits "k=v" pairs joined by "," in HEAD-first order.

use std::collections::HashMap;

use cave_tracing::exporter::OtlpHttpExporter;
use cave_tracing::propagation::{parse_tracestate, TraceState};
use cave_tracing::types::{
    AttrValue, Event, Link, SpanContext, SpanData, SpanKind, Status,
};

/// Build a baseline root `SpanData` (no parent, no links, single resource attr).
fn base_span(name: &str) -> SpanData {
    let now = chrono::Utc::now();
    SpanData {
        name: name.into(),
        context: SpanContext::new(0x1111_2222_3333_4444_5555_6666_7777_8888, 0xfeedface, true),
        parent_span_id: None,
        kind: SpanKind::Server,
        start_time: now,
        end_time: now + chrono::Duration::milliseconds(5),
        attributes: HashMap::new(),
        events: Vec::<Event>::new(),
        links: Vec::<Link>::new(),
        status: Status::Ok,
        instrumentation_scope: "scope".into(),
        tenant_id: "anonymous".into(),
        resource: HashMap::from([("service.name".to_string(), "svc".to_string())]),
    }
}

/// `render_payload` serializes the `links` array: each link emits the formatted
/// 32-hex `traceId`, 16-hex `spanId`, and its attributes. Derived from
/// `span_to_otlp`'s link branch + `format_trace_id`/`format_span_id` padding.
#[test]
fn test_otlp_payload_serializes_link_trace_and_span_ids_and_attrs() {
    let exp = OtlpHttpExporter::new("http://localhost");
    let mut s = base_span("op");
    let mut link_attrs: HashMap<String, AttrValue> = HashMap::new();
    link_attrs.insert("rel".into(), AttrValue::String("follows-from".into()));
    s.links.push(Link {
        // link points at trace_id 0x1122, span_id 0x3344
        context: SpanContext::new(0x1122, 0x3344, true),
        attributes: link_attrs,
    });

    let v = exp.render_payload(&[s]);
    let link = &v["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["links"][0];

    assert_eq!(link["traceId"], "00000000000000000000000000001122");
    assert_eq!(link["spanId"], "0000000000003344");

    let attrs = link["attributes"].as_array().unwrap();
    let rel = attrs.iter().find(|a| a["key"] == "rel").unwrap();
    assert_eq!(rel["value"]["stringValue"], "follows-from");
}

/// A child span (`parent_span_id = Some(0xabcd)`) renders `parentSpanId` as the
/// 16-hex zero-padded parent id. Derived from `format_span_id(0xabcd)`.
#[test]
fn test_otlp_payload_emits_parent_span_id_for_child() {
    let exp = OtlpHttpExporter::new("http://localhost");
    let mut s = base_span("child");
    s.parent_span_id = Some(0xabcd);

    let v = exp.render_payload(&[s]);
    let span = &v["resourceSpans"][0]["scopeSpans"][0]["spans"][0];
    assert_eq!(span["parentSpanId"], "000000000000abcd");
}

/// A root span (`parent_span_id = None`) renders `parentSpanId` as the empty
/// string via `Option::map(...).unwrap_or_default()`.
#[test]
fn test_otlp_payload_emits_empty_parent_span_id_for_root() {
    let exp = OtlpHttpExporter::new("http://localhost");
    let s = base_span("root"); // parent_span_id: None
    let v = exp.render_payload(&[s]);
    let span = &v["resourceSpans"][0]["scopeSpans"][0]["spans"][0];
    assert_eq!(span["parentSpanId"], "");
}

/// Two spans sharing scope + resource but with *different* `tenant_id` land in
/// two distinct `resourceSpans` groups, because tenant is the first element of
/// the grouping key `(tenant_id, scope, resource_hash)`. Each group's resource
/// carries its own `tenant_id` label.
#[test]
fn test_otlp_payload_splits_distinct_tenants_into_separate_resource_spans() {
    let exp = OtlpHttpExporter::new("http://localhost");
    let mut a = base_span("a");
    a.tenant_id = "acme".into();
    let mut b = base_span("b");
    b.tenant_id = "globex".into();
    // identical scope + resource on both -> only the tenant differs
    assert_eq!(a.instrumentation_scope, b.instrumentation_scope);

    let v = exp.render_payload(&[a, b]);
    let groups = v["resourceSpans"].as_array().unwrap();
    assert_eq!(groups.len(), 2, "distinct tenants -> two resource_spans");

    // BTreeMap key order: "acme" < "globex", so group 0 is acme, group 1 globex.
    let tenant_label = |g: &serde_json::Value| -> String {
        g["resource"]["attributes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|attr| attr["key"] == "tenant_id")
            .unwrap()["value"]["stringValue"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_eq!(tenant_label(&groups[0]), "acme");
    assert_eq!(tenant_label(&groups[1]), "globex");
}

/// `TraceState::to_header` emits entries in HEAD-of-list order. After two
/// upserts, the most recently upserted key sits at index 0, so the header is
/// "newest=...,older=...". Derived from `upsert` inserting at index 0.
#[test]
fn test_tracestate_to_header_is_head_of_list_ordered() {
    let mut s = TraceState::new();
    s.upsert("rojo", "1");
    s.upsert("congo", "2"); // congo becomes the head

    assert_eq!(s.to_header(), "congo=2,rojo=1");
}

/// `parse_tracestate(state.to_header())` round-trips the entries, and `get`
/// returns the upserted values. Empty state serializes to the empty string.
#[test]
fn test_tracestate_to_header_round_trips_via_parse_and_get() {
    let empty = TraceState::new();
    assert_eq!(empty.to_header(), "");

    let mut s = TraceState::new();
    s.upsert("rojo", "00f067aa0ba902b7");
    s.upsert("congo", "t61rcWkgMzE"); // head

    let reparsed = parse_tracestate(&s.to_header());
    assert_eq!(reparsed, s);
    assert_eq!(reparsed.get("congo"), Some("t61rcWkgMzE"));
    assert_eq!(reparsed.get("rojo"), Some("00f067aa0ba902b7"));
    // HEAD ordering survives the round-trip.
    assert_eq!(reparsed.entries[0].0, "congo");
}
