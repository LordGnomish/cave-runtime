// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Strict-TDD RED test for the Cassandra storage db-model codec
//! (jaeger `plugin/storage/cassandra/spanstore/dbmodel/` — model.go +
//! converter.go + tag_filter.go, v1.52.0).
//!
//! Ports the *pure* span ↔ Cassandra-schema conversion layer: the domain
//! Span is encoded to a `DbSpan` whose `start_time`/`duration` are in
//! **microseconds** (`model.TimeAsEpochMicroseconds` /
//! `model.DurationAsMicroseconds`), whose `TraceID` is the 16-byte
//! big-endian blob, whose span/process tags become typed `DbKeyValue`s with
//! the canonical value-type strings, and whose index-tag set is produced by
//! `GetAllUniqueTags` (combine process + span + log tags, sort, skip binary,
//! dedupe adjacent, `TagInsertion{service, key, AsString()}`).
//!
//! The live gocql session + CQL DDL execution stay scope_cut
//! (operational-storage-backends); this is the in-crate codec only.

use std::collections::HashMap;

use cave_trace::storage_cassandra::{
    self, DbSpan, TagInsertion, BINARY_TYPE, BOOL_TYPE, CHILD_OF, FLOAT64_TYPE, FOLLOWS_FROM,
    INT64_TYPE, STRING_TYPE,
};
use cave_trace::types::{Span, SpanEvent, SpanKind, SpanStatus, TagValue};

fn span_fixture() -> Span {
    let mut tags = HashMap::new();
    tags.insert("http.method".into(), TagValue::String("GET".into()));
    tags.insert("http.status_code".into(), TagValue::Int(200));
    tags.insert("retry".into(), TagValue::Bool(true));
    tags.insert("ratio".into(), TagValue::Float(0.5));
    tags.insert("blob".into(), TagValue::Binary(vec![0xde, 0xad]));

    let mut resource = HashMap::new();
    resource.insert("ip".into(), TagValue::String("10.0.0.1".into()));

    Span {
        trace_id: 0x0011223344556677_8899aabbccddeeff,
        span_id: 0xdeadbeefcafef00d,
        parent_span_id: Some(0x1111222233334444),
        operation_name: "GET /things".into(),
        service_name: "frontend".into(),
        start_time_unix_nano: 1_600_000_000_000_999_000, // 999_000 ns sub-micro tail
        end_time_unix_nano: 1_600_000_000_500_999_000,
        duration_ns: 500_000_000, // 500 ms
        status: SpanStatus::Ok,
        kind: SpanKind::Server,
        tags,
        events: vec![SpanEvent {
            time_unix_nano: 1_600_000_000_100_000_000,
            name: "event".into(),
            attributes: {
                let mut m = HashMap::new();
                m.insert("log.field".into(), TagValue::String("hello".into()));
                m
            },
        }],
        links: vec![],
        resource_attributes: resource,
        tenant_id: "default".into(),
        baggage: HashMap::new(),
        log_labels: HashMap::new(),
    }
}

#[test]
fn value_type_and_reftype_constants_match_jaeger() {
    assert_eq!(STRING_TYPE, "string");
    assert_eq!(BOOL_TYPE, "bool");
    assert_eq!(INT64_TYPE, "int64");
    assert_eq!(FLOAT64_TYPE, "float64");
    assert_eq!(BINARY_TYPE, "binary");
    assert_eq!(CHILD_OF, "child-of");
    assert_eq!(FOLLOWS_FROM, "follows-from");
}

#[test]
fn from_domain_converts_times_to_microseconds() {
    let db = DbSpan::from_domain(&span_fixture());
    // 1_600_000_000_000_999_000 ns → 1_600_000_000_000_999 µs (integer div by 1000).
    assert_eq!(db.start_time, 1_600_000_000_000_999);
    // 500 ms → 500_000 µs.
    assert_eq!(db.duration, 500_000);
}

#[test]
fn from_domain_encodes_trace_id_as_16_byte_big_endian() {
    let db = DbSpan::from_domain(&span_fixture());
    let expected = 0x0011223344556677_8899aabbccddeeff_u128.to_be_bytes();
    assert_eq!(db.trace_id, expected);
    // span_id is the bit-cast of the u64 into i64.
    assert_eq!(db.span_id, 0xdeadbeefcafef00d_u64 as i64);
}

#[test]
fn round_trip_preserves_core_fields_at_microsecond_granularity() {
    let original = span_fixture();
    let db = DbSpan::from_domain(&original);
    let back = db.to_domain();

    assert_eq!(back.trace_id, original.trace_id);
    assert_eq!(back.span_id, original.span_id);
    assert_eq!(back.operation_name, original.operation_name);
    assert_eq!(back.service_name, original.service_name);
    // microsecond granularity: nanos truncated to µs then back to ns.
    assert_eq!(back.start_time_unix_nano, 1_600_000_000_000_999_000);
    assert_eq!(back.duration_ns, 500_000_000);
}

#[test]
fn keyvalue_value_type_and_as_string() {
    let db = DbSpan::from_domain(&span_fixture());
    let by_key: HashMap<&str, &cave_trace::storage_cassandra::DbKeyValue> =
        db.tags.iter().map(|kv| (kv.key.as_str(), kv)).collect();

    assert_eq!(by_key["http.method"].value_type, STRING_TYPE);
    assert_eq!(by_key["http.method"].as_string(), "GET");
    assert_eq!(by_key["http.status_code"].value_type, INT64_TYPE);
    assert_eq!(by_key["http.status_code"].as_string(), "200");
    assert_eq!(by_key["retry"].value_type, BOOL_TYPE);
    assert_eq!(by_key["retry"].as_string(), "true");
    assert_eq!(by_key["ratio"].value_type, FLOAT64_TYPE);
    assert_eq!(by_key["blob"].value_type, BINARY_TYPE);
}

#[test]
fn get_all_unique_tags_combines_sorts_skips_binary_and_dedupes() {
    let span = span_fixture();
    let tags: Vec<TagInsertion> = storage_cassandra::get_all_unique_tags(&span);

    // Every insertion carries the span's process service name.
    assert!(tags.iter().all(|t| t.service_name == "frontend"));

    // Binary "blob" tag is NOT indexed.
    assert!(!tags.iter().any(|t| t.tag_key == "blob"));

    // Span tags + process tag "ip" + log field "log.field" are all present.
    let keys: Vec<&str> = tags.iter().map(|t| t.tag_key.as_str()).collect();
    assert!(keys.contains(&"http.method"));
    assert!(keys.contains(&"ip"));
    assert!(keys.contains(&"log.field"));

    // Sorted ascending by (key, value).
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted);

    // A specific insertion stringifies its value.
    let m = tags.iter().find(|t| t.tag_key == "http.status_code").unwrap();
    assert_eq!(m.tag_value, "200");
}

#[test]
fn get_all_unique_tags_dedupes_adjacent_identical() {
    let mut span = span_fixture();
    // Drop everything except two identical span/process tags to force a dup.
    span.tags.clear();
    span.events.clear();
    span.resource_attributes.clear();
    span.tags.insert("env".into(), TagValue::String("prod".into()));
    span.resource_attributes
        .insert("env".into(), TagValue::String("prod".into()));

    let tags = storage_cassandra::get_all_unique_tags(&span);
    let env_count = tags.iter().filter(|t| t.tag_key == "env").count();
    assert_eq!(env_count, 1, "identical (key,value) tag must be deduped");
}

#[test]
fn tag_insertion_display_is_colon_joined() {
    let t = TagInsertion {
        service_name: "frontend".into(),
        tag_key: "http.method".into(),
        tag_value: "GET".into(),
    };
    assert_eq!(t.display(), "frontend:http.method:GET");
}
