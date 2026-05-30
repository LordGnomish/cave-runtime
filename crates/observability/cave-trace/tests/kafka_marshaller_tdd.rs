// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Strict-TDD parity port — Jaeger Kafka span marshaller codec.
//!
//! Upstream: jaegertracing/jaeger v1.52.0
//!   plugin/storage/kafka/marshaller.go   (jsonMarshaller / protobufMarshaller)
//!   plugin/storage/kafka/unmarshaller.go (jsonUnmarshaller)
//!   plugin/storage/kafka/writer.go       (Key = span.TraceID.String())
//!
//! The collector→ingester Kafka topic carries spans serialised by a
//! `Marshaller`. The JSON marshaller emits the jsonpb `model.Span`
//! representation: hex `traceID`/`spanID`, `references[{refType,traceID,
//! spanID}]`, `startTime` as an RFC3339 timestamp, `duration` as a
//! fractional-seconds string, and `tags`/`process.tags` as `vType`-typed
//! key/value objects. The producer partitions by `span.TraceID.String()`
//! so all spans of a trace land on one partition (in-trace ordering).
//!
//! This is the pure marshal/unmarshal codec + partition key; the live sarama
//! AsyncProducer + the ingester consumer group stay scope_cut
//! (operational-storage-backends).
//!
//! RED commit: references `cave_trace::storage_kafka::*` (absent) → the crate
//! fails to compile.

use cave_trace::storage_kafka::{format_proto_duration, parse_proto_duration, KafkaSpanCodec};
use cave_trace::types::{Span, SpanKind, SpanStatus, TagValue};
use std::collections::HashMap;

fn span() -> Span {
    let mut tags = HashMap::new();
    tags.insert("http.method".into(), TagValue::String("GET".into()));
    tags.insert("http.status_code".into(), TagValue::Int(200));
    tags.insert("error".into(), TagValue::Bool(false));
    Span {
        trace_id: 0xdeadbeefcafe1234_0011223344556677,
        span_id: 0x00ab_cdef_0011_2233,
        parent_span_id: Some(0x1111_2222_3333_4444),
        operation_name: "HTTP GET /api".into(),
        service_name: "frontend".into(),
        start_time_unix_nano: 1_600_000_000_000_123_000,
        end_time_unix_nano: 1_600_000_000_000_246_000,
        duration_ns: 123_000,
        status: SpanStatus::Ok,
        kind: SpanKind::Client,
        tags,
        events: vec![],
        links: vec![],
        resource_attributes: HashMap::new(),
        tenant_id: "default".into(),
        baggage: HashMap::new(),
        log_labels: HashMap::new(),
    }
}

// ── 1. JSON marshal emits hex IDs + operationName ────────────────────────────

#[test]
fn marshal_emits_hex_ids() {
    let bytes = KafkaSpanCodec::marshal_json(&span());
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["traceID"], "deadbeefcafe12340011223344556677");
    assert_eq!(v["spanID"], "00abcdef00112233");
    assert_eq!(v["operationName"], "HTTP GET /api");
    assert_eq!(v["process"]["serviceName"], "frontend");
}

// ── 2. Tags carry the jsonpb vType discriminator + typed value field ─────────

#[test]
fn tags_use_vtype_typed_encoding() {
    let bytes = KafkaSpanCodec::marshal_json(&span());
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let tags = v["tags"].as_array().unwrap();

    let method = tags.iter().find(|t| t["key"] == "http.method").unwrap();
    assert_eq!(method["vType"], "STRING");
    assert_eq!(method["vStr"], "GET");

    let code = tags.iter().find(|t| t["key"] == "http.status_code").unwrap();
    assert_eq!(code["vType"], "INT64");
    // jsonpb renders int64 as a string
    assert_eq!(code["vInt64"], "200");

    let err = tags.iter().find(|t| t["key"] == "error").unwrap();
    assert_eq!(err["vType"], "BOOL");
    assert_eq!(err["vBool"], false);
}

// ── 3. parent_span_id becomes a CHILD_OF reference ───────────────────────────

#[test]
fn parent_becomes_child_of_reference() {
    let bytes = KafkaSpanCodec::marshal_json(&span());
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let refs = v["references"].as_array().unwrap();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0]["refType"], "CHILD_OF");
    assert_eq!(refs[0]["traceID"], "deadbeefcafe12340011223344556677");
    assert_eq!(refs[0]["spanID"], "1111222233334444");
}

// ── 4. duration renders as a fractional-seconds string ───────────────────────

#[test]
fn duration_is_fractional_seconds_string() {
    let bytes = KafkaSpanCodec::marshal_json(&span());
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    // 123_000 ns = 0.000123 s
    assert_eq!(v["duration"], "0.000123s");
}

// ── 5. Partition key is the trace ID hex (writer.go Key) ─────────────────────

#[test]
fn partition_key_is_trace_id_hex() {
    assert_eq!(
        KafkaSpanCodec::partition_key(&span()),
        "deadbeefcafe12340011223344556677"
    );
}

// ── 6. Round-trip marshal → unmarshal preserves the span ─────────────────────

#[test]
fn json_round_trip_preserves_span() {
    let original = span();
    let bytes = KafkaSpanCodec::marshal_json(&original);
    let back = KafkaSpanCodec::unmarshal_json(&bytes).expect("unmarshal");

    assert_eq!(back.trace_id, original.trace_id);
    assert_eq!(back.span_id, original.span_id);
    assert_eq!(back.parent_span_id, original.parent_span_id);
    assert_eq!(back.operation_name, original.operation_name);
    assert_eq!(back.service_name, original.service_name);
    assert_eq!(back.duration_ns, original.duration_ns);
    assert_eq!(back.tags.get("http.method"), Some(&TagValue::String("GET".into())));
    assert_eq!(back.tags.get("http.status_code"), Some(&TagValue::Int(200)));
    assert_eq!(back.tags.get("error"), Some(&TagValue::Bool(false)));
}

// ── 7. proto Duration format: integer second, sub-ms, sub-second groups ──────

#[test]
fn proto_duration_formatting() {
    assert_eq!(format_proto_duration(0), "0s");
    assert_eq!(format_proto_duration(1_000_000_000), "1s");
    assert_eq!(format_proto_duration(123_000), "0.000123s");
    assert_eq!(format_proto_duration(1_000_000), "0.001s");
    // 0.12 s pads the fraction to a group of three digits
    assert_eq!(format_proto_duration(120_000_000), "0.120s");
    // 1.5 s
    assert_eq!(format_proto_duration(1_500_000_000), "1.500s");
}

// ── 8. proto Duration parses back to nanoseconds ─────────────────────────────

#[test]
fn proto_duration_round_trips() {
    for ns in [0u64, 1, 123_000, 1_000_000, 120_000_000, 1_000_000_000, 1_500_000_000] {
        let s = format_proto_duration(ns);
        assert_eq!(parse_proto_duration(&s).unwrap(), ns, "round trip for {} ({})", ns, s);
    }
}
