// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Strict-TDD RED test for the Elasticsearch storage db-model codec
//! (jaeger `plugin/storage/es/spanstore/dbmodel/` — model.go +
//! from_domain.go, v1.52.0).
//!
//! Ports the *pure* span → ES-document conversion: trace/span identifiers
//! rendered as `model.TraceID.String()` / `SpanID.String()` hex, time fields
//! in micros + the ES-routing `startTimeMillis`, references with the
//! UPPERCASE `CHILD_OF` / `FOLLOWS_FROM` reftypes, the structured `tags`
//! array, the Kibana-friendly flattened `tag` map (dots → `@`, binary
//! excluded, gated on `allTagsAsFields`), and the date-rotated index name
//! `[prefix-]jaeger-span-YYYY-MM-DD`.
//!
//! The live ES HTTP bulk client + index templates stay scope_cut
//! (operational-storage-backends); this is the in-crate document codec.

use std::collections::HashMap;

use cave_trace::storage_es::{
    self, EsConfig, EsSpan, DEFAULT_TAG_DOT_REPLACEMENT, SERVICE_INDEX_BASE, SPAN_INDEX_BASE,
};
use cave_trace::types::{Span, SpanKind, SpanStatus, TagValue};

// 2020-09-13 12:26:40 UTC.
const START_NS: u64 = 1_600_000_000_000_000_000;

fn span_fixture() -> Span {
    let mut tags = HashMap::new();
    tags.insert("http.method".into(), TagValue::String("GET".into()));
    tags.insert("http.status_code".into(), TagValue::Int(200));
    tags.insert("blob".into(), TagValue::Binary(vec![0xde, 0xad]));

    Span {
        trace_id: 0x0011223344556677_8899aabbccddeeff,
        span_id: 0x00000000cafef00d,
        parent_span_id: Some(0x1111222233334444),
        operation_name: "GET /things".into(),
        service_name: "frontend".into(),
        start_time_unix_nano: START_NS,
        end_time_unix_nano: START_NS + 250_000_000,
        duration_ns: 250_000_000, // 250 ms
        status: SpanStatus::Ok,
        kind: SpanKind::Server,
        tags,
        events: vec![],
        links: vec![],
        resource_attributes: HashMap::new(),
        tenant_id: "default".into(),
        baggage: HashMap::new(),
        log_labels: HashMap::new(),
    }
}

#[test]
fn ids_render_as_jaeger_hex_strings() {
    let cfg = EsConfig::default();
    let doc = EsSpan::from_domain(&span_fixture(), &cfg);
    // 128-bit trace with non-zero high word → 32 hex chars.
    assert_eq!(doc.trace_id, "00112233445566778899aabbccddeeff");
    // span id always 16 hex chars, zero-padded.
    assert_eq!(doc.span_id, "00000000cafef00d");
}

#[test]
fn trace_id_drops_high_word_when_zero() {
    let cfg = EsConfig::default();
    let mut s = span_fixture();
    s.trace_id = 0x00000000cafef00d; // high word zero
    let doc = EsSpan::from_domain(&s, &cfg);
    // model.TraceID.String() prints only the low word when high == 0.
    assert_eq!(doc.trace_id, "00000000cafef00d");
}

#[test]
fn times_use_micros_and_start_time_millis() {
    let cfg = EsConfig::default();
    let doc = EsSpan::from_domain(&span_fixture(), &cfg);
    assert_eq!(doc.start_time, 1_600_000_000_000_000); // micros
    assert_eq!(doc.start_time_millis, 1_600_000_000_000); // micros / 1000
    assert_eq!(doc.duration, 250_000); // micros
}

#[test]
fn parent_becomes_uppercase_child_of_reference() {
    let cfg = EsConfig::default();
    let doc = EsSpan::from_domain(&span_fixture(), &cfg);
    assert_eq!(doc.references.len(), 1);
    let r = &doc.references[0];
    assert_eq!(r.ref_type, "CHILD_OF");
    assert_eq!(r.trace_id, "00112233445566778899aabbccddeeff");
    assert_eq!(r.span_id, "1111222233334444");
}

#[test]
fn structured_tags_array_keeps_binary_and_types() {
    let cfg = EsConfig::default();
    let doc = EsSpan::from_domain(&span_fixture(), &cfg);
    let by_key: HashMap<&str, &storage_es::EsKeyValue> =
        doc.tags.iter().map(|kv| (kv.key.as_str(), kv)).collect();
    assert_eq!(by_key["http.method"].value_type, "string");
    assert_eq!(by_key["http.status_code"].value_type, "int64");
    // binary tag stays in the structured array.
    assert_eq!(by_key["blob"].value_type, "binary");
}

#[test]
fn flattened_tag_map_empty_unless_all_tags_as_fields() {
    let cfg = EsConfig::default(); // all_tags_as_fields = false
    let doc = EsSpan::from_domain(&span_fixture(), &cfg);
    assert!(doc.tag.is_empty(), "default config keeps `tag` map empty");
}

#[test]
fn flattened_tag_map_replaces_dots_and_drops_binary() {
    let cfg = EsConfig {
        all_tags_as_fields: true,
        ..EsConfig::default()
    };
    assert_eq!(DEFAULT_TAG_DOT_REPLACEMENT, '@');
    let doc = EsSpan::from_domain(&span_fixture(), &cfg);
    // "http.method" → "http@method" in the flattened map.
    assert!(doc.tag.contains_key("http@method"));
    assert_eq!(doc.tag["http@method"], serde_json::json!("GET"));
    assert!(doc.tag.contains_key("http@status_code"));
    // binary excluded from the flattened map (stays only in `tags`).
    assert!(!doc.tag.contains_key("blob"));
}

#[test]
fn index_name_is_date_rotated_with_optional_prefix() {
    assert_eq!(SPAN_INDEX_BASE, "jaeger-span-");
    assert_eq!(SERVICE_INDEX_BASE, "jaeger-service-");

    let no_prefix = storage_es::index_name(SPAN_INDEX_BASE, START_NS, None);
    assert_eq!(no_prefix, "jaeger-span-2020-09-13");

    let prefixed = storage_es::index_name(SPAN_INDEX_BASE, START_NS, Some("prod"));
    assert_eq!(prefixed, "prod-jaeger-span-2020-09-13");
}

#[test]
fn process_carries_service_name() {
    let cfg = EsConfig::default();
    let doc = EsSpan::from_domain(&span_fixture(), &cfg);
    assert_eq!(doc.process.service_name, "frontend");
}
