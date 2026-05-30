// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Strict-TDD RED test for the Badger storage key-encoding store
//! (jaeger `plugin/storage/badger/spanstore/writer.go` + key layout,
//! v1.52.0).
//!
//! Ports the *pure* Badger key layout — the byte-exact primary key
//! `[0x80 | traceID.High | traceID.Low | startTime µs | spanID]` and the
//! secondary index key `[(prefix & 0x0F)|0x80 | value | startTime µs |
//! traceID.High | traceID.Low]` — plus the per-index value bytes
//! (service / service+operation / service+operation+duration / tag
//! service+key+AsString). It backs them with an in-process ordered map that
//! behaves like Badger's sorted LSM so reads (get_trace, trace_ids_by_service
//! via a sorted prefix scan) genuinely work.
//!
//! The live Badger mmap LSM + value-log + GC stay scope_cut
//! (operational-storage-backends); this is the in-crate key codec + ordered
//! store the writer/reader sit on top of.

use std::collections::HashMap;

use cave_trace::storage_badger::{
    self, BadgerStore, DURATION_INDEX_KEY, OPERATION_NAME_INDEX_KEY, SERVICE_NAME_INDEX_KEY,
    SPAN_KEY_PREFIX, TAG_INDEX_KEY,
};
use cave_trace::types::{Span, SpanEvent, SpanKind, SpanStatus, TagValue};

const START_NS: u64 = 1_600_000_000_000_000_000; // micros: 1_600_000_000_000_000

fn span_fixture() -> Span {
    let mut tags = HashMap::new();
    tags.insert("http.method".into(), TagValue::String("GET".into()));
    tags.insert("http.status_code".into(), TagValue::Int(200));

    let mut resource = HashMap::new();
    resource.insert("ip".into(), TagValue::String("10.0.0.1".into()));

    Span {
        trace_id: 0x0011223344556677_8899aabbccddeeff,
        span_id: 0xdeadbeefcafef00d,
        parent_span_id: None,
        operation_name: "GET /things".into(),
        service_name: "frontend".into(),
        start_time_unix_nano: START_NS,
        end_time_unix_nano: START_NS + 250_000_000,
        duration_ns: 250_000_000, // 250 ms → 250_000 µs
        status: SpanStatus::Ok,
        kind: SpanKind::Server,
        tags,
        events: vec![SpanEvent {
            time_unix_nano: START_NS + 1_000,
            name: "ev".into(),
            attributes: {
                let mut m = HashMap::new();
                m.insert("log.field".into(), TagValue::String("x".into()));
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
fn primary_key_is_byte_exact() {
    let key = storage_badger::primary_key(
        0x0011223344556677_8899aabbccddeeff,
        1_600_000_000_000_000,
        0xdeadbeefcafef00d,
    );
    assert_eq!(key.len(), 1 + 16 + 8 + 8);
    assert_eq!(key[0], SPAN_KEY_PREFIX);
    assert_eq!(key[0], 0x80);
    assert_eq!(&key[1..9], &0x0011223344556677u64.to_be_bytes()); // high
    assert_eq!(&key[9..17], &0x8899aabbccddeeffu64.to_be_bytes()); // low
    assert_eq!(&key[17..25], &1_600_000_000_000_000u64.to_be_bytes()); // start µs
    assert_eq!(&key[25..33], &0xdeadbeefcafef00du64.to_be_bytes()); // spanID
}

#[test]
fn index_key_prefix_masks_and_ors_0x80() {
    let val = b"frontend";
    let key = storage_badger::index_key(
        SERVICE_NAME_INDEX_KEY,
        val,
        1_600_000_000_000_000,
        0x0011223344556677_8899aabbccddeeff,
    );
    // prefix = (0x81 & 0x0f) | 0x80 = 0x81
    assert_eq!(key[0], (SERVICE_NAME_INDEX_KEY & 0x0f) | 0x80);
    assert_eq!(key[0], 0x81);
    assert_eq!(&key[1..1 + val.len()], val);
    let pos = 1 + val.len();
    assert_eq!(&key[pos..pos + 8], &1_600_000_000_000_000u64.to_be_bytes());
    assert_eq!(&key[pos + 8..pos + 16], &0x0011223344556677u64.to_be_bytes());
    assert_eq!(&key[pos + 16..pos + 24], &0x8899aabbccddeeffu64.to_be_bytes());
}

#[test]
fn index_prefix_constants_have_expected_low_nibbles() {
    assert_eq!(SERVICE_NAME_INDEX_KEY, 0x81);
    assert_eq!(OPERATION_NAME_INDEX_KEY, 0x82);
    assert_eq!(TAG_INDEX_KEY, 0x83);
    assert_eq!(DURATION_INDEX_KEY, 0x84);
}

#[test]
fn duration_index_value_is_service_op_plus_8_byte_micros() {
    let v = storage_badger::duration_index_value("frontend", "GET /things", 250_000);
    let mut expected = b"frontendGET /things".to_vec();
    expected.extend_from_slice(&250_000u64.to_be_bytes());
    assert_eq!(v, expected);
}

#[test]
fn tag_index_value_is_service_key_value() {
    let v = storage_badger::tag_index_value("frontend", "http.method", "GET");
    assert_eq!(v, b"frontendhttp.methodGET".to_vec());
}

#[test]
fn write_then_get_trace_round_trips() {
    let mut store = BadgerStore::new();
    store.write_span(&span_fixture());
    let spans = store.get_trace(0x0011223344556677_8899aabbccddeeff);
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].operation_name, "GET /things");
    assert_eq!(spans[0].service_name, "frontend");
    assert_eq!(spans[0].duration_ns, 250_000_000);
}

#[test]
fn trace_ids_by_service_prefix_scan_finds_only_matching_service() {
    let mut store = BadgerStore::new();
    store.write_span(&span_fixture());

    let hits = store.trace_ids_by_service("frontend", 0, u64::MAX);
    assert_eq!(hits, vec![0x0011223344556677_8899aabbccddeeff]);

    let misses = store.trace_ids_by_service("backend", 0, u64::MAX);
    assert!(misses.is_empty());
}

#[test]
fn write_span_emits_primary_plus_all_index_keys() {
    let mut store = BadgerStore::new();
    store.write_span(&span_fixture());
    // 1 primary + service + operation + duration + tag indexes.
    // tags: 2 span + 1 process + 1 log field = 4 tag-index keys.
    // total = 1 + 1 + 1 + 1 + 4 = 8.
    assert_eq!(store.key_count(), 8);
}
