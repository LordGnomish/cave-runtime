//! cave-cdc — cave-streams sink tests (no Kafka Connect).

use cave_cdc::routing::{RoutingPolicy, TopicRouter};
use cave_cdc::streams_sink::{MemorySink, SinkBackend, StreamsSink};
use cave_cdc::{CdcError, SourceRecord};
use chrono::Utc;
use std::collections::HashMap;

const TENANT: &str = "tenant-acme-prod";

fn record(topic: &str, key: &[u8], value: &[u8]) -> SourceRecord {
    SourceRecord {
        tenant_id: TENANT.into(),
        topic: topic.into(),
        partition: 0,                   // overwritten by sink router
        key: key.to_vec(),
        value: value.to_vec(),
        headers: HashMap::new(),
        source_ts_ms: 1_700_000_000,
        created_at: Utc::now(),
    }
}

fn sink() -> StreamsSink<MemorySink> {
    let router = TopicRouter::new(TENANT, "billing-pg", RoutingPolicy::SchemaTable);
    StreamsSink::new(router, 4, MemorySink::new())
}

/// Cite: debezium `EmbeddedEngine` test harness — produce assigns a
/// monotonic offset per (topic, partition); the in-memory backend is
/// the cave equivalent.
#[test]
fn dispatch_assigns_monotonic_offsets_per_topic_partition() {
    let mut s = sink();
    let topic = format!("{}.billing-pg.public.orders", TENANT);
    let r1 = s.dispatch(&record(&topic, b"key-1", b"val-1")).unwrap();
    let r2 = s.dispatch(&record(&topic, b"key-1", b"val-2")).unwrap();
    let r3 = s.dispatch(&record(&topic, b"key-1", b"val-3")).unwrap();
    assert_eq!(r1.partition, r2.partition);
    assert_eq!(r1.partition, r3.partition);
    assert_eq!(r1.base_offset, 0);
    assert_eq!(r2.base_offset, 1);
    assert_eq!(r3.base_offset, 2);
    assert_eq!(s.backend.count_for(&topic, r1.partition), 3);
}

/// Cite: cave multi-tenant invariant — a SourceRecord whose tenant_id
/// does not match the sink's MUST be rejected before any backend
/// produce is attempted.
#[test]
fn dispatch_rejects_cross_tenant_records() {
    let mut s = sink();
    let mut foreign = record(&format!("{}.billing-pg.public.orders", TENANT), b"k", b"v");
    foreign.tenant_id = "tenant-other".into();
    let err = s.dispatch(&foreign).unwrap_err();
    assert!(matches!(err, CdcError::CrossTenantDenied { .. }));
    assert!(s.backend.log.is_empty(), "no backend produce on rejection");
}

/// Cite: cave routing invariant — every emitted topic MUST start with
/// `<tenant>.`. A record whose topic lacks the prefix is rejected.
#[test]
fn dispatch_rejects_topic_without_tenant_prefix() {
    let mut s = sink();
    let bad = record("noprefix.billing-pg.public.orders", b"k", b"v");
    let err = s.dispatch(&bad).unwrap_err();
    assert!(matches!(err, CdcError::CrossTenantDenied { .. }));
}

/// Cite: debezium `EventDispatcher::dispatch` batched fan-out — the
/// batch helper short-circuits on first error, returning per-record
/// results up to that point.
#[test]
fn dispatch_batch_short_circuits_on_first_failure() {
    let mut s = sink();
    let topic = format!("{}.billing-pg.public.orders", TENANT);
    let mut bad = record(&topic, b"k", b"v");
    bad.tenant_id = "tenant-other".into();

    let batch = vec![
        record(&topic, b"k1", b"v1"),
        record(&topic, b"k2", b"v2"),
        bad,
        record(&topic, b"k3", b"v3"),
    ];
    let err = s.dispatch_batch(&batch).unwrap_err();
    assert!(matches!(err, CdcError::CrossTenantDenied { .. }));
    // First two records were produced; the bad one and anything after
    // were not.
    assert_eq!(s.backend.count_for(&topic, 0)
        + s.backend.count_for(&topic, 1)
        + s.backend.count_for(&topic, 2)
        + s.backend.count_for(&topic, 3),
        2);
}
