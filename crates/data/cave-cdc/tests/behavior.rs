// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-cdc — portable-coverage TDD fills for the cave-streams sink and the
//! in-memory schema registry.
//!
//! Cite: debezium/debezium-server v3.5.0.Final. These tests mirror the only
//! portable, pure-logic JUnit units in the upstream (overwhelmingly Quarkus /
//! Testcontainers integration) suite:
//!   * `KinesisUnitTest.testBatchesAreCorrect` / `testBatchSplitting` /
//!     `testEmptyRecords` → `StreamsSink::dispatch_batch` success path.
//!   * Confluent Schema Registry `register` compatibility check (exercised in
//!     `DebeziumServerWith*RegistryIT`) → `SchemaRegistry::register` rejection
//!     branch via `check_compat`.
//!
//! Scope: success-path / rejection-branch coverage on public cave fns that are
//! already implemented and source-verified but lacked a direct assertion. No
//! new behavior.

use cave_cdc::routing::{RoutingPolicy, TopicRouter};
use cave_cdc::schema::{Compatibility, FieldDef, Schema, SchemaFormat, SchemaRegistry};
use cave_cdc::streams_sink::{MemorySink, StreamsSink};
use cave_cdc::SourceRecord;
use chrono::Utc;
use std::collections::HashMap;

const TENANT: &str = "tenant-acme-prod";
const SERVER: &str = "billing-pg";

fn record(topic: &str, key: &[u8], value: &[u8]) -> SourceRecord {
    SourceRecord {
        tenant_id: TENANT.into(),
        topic: topic.into(),
        partition: 0, // overwritten by the sink router
        key: key.to_vec(),
        value: value.to_vec(),
        headers: HashMap::new(),
        source_ts_ms: 1_700_000_000,
        created_at: Utc::now(),
    }
}

fn sink(partitions: i32) -> StreamsSink<MemorySink> {
    let router = TopicRouter::new(TENANT, SERVER, RoutingPolicy::SchemaTable);
    StreamsSink::new(router, partitions, MemorySink::new())
}

// ----------------------------------------------------------------------------
// StreamsSink::dispatch_batch — success path
// ----------------------------------------------------------------------------

/// Cite: `KinesisUnitTest.testBatchSplitting` — every record in a single-topic
/// batch is delivered and offsets are contiguous 0..N within its partition.
///
/// `dispatch_batch` loops `dispatch` over every record; `MemorySink::produce`
/// assigns a monotonic per-(topic,partition) offset starting at 0. With a fixed
/// key all N records hash to the same partition, so base_offsets are 0..N.
#[test]
fn dispatch_batch_delivers_all_records_with_contiguous_offsets() {
    let mut s = sink(4);
    let topic = format!("{TENANT}.{SERVER}.public.orders");
    let n = 6usize;
    let batch: Vec<SourceRecord> = (0..n)
        .map(|i| record(&topic, b"order-key", format!("val-{i}").as_bytes()))
        .collect();

    let results = s.dispatch_batch(&batch).unwrap();

    assert_eq!(results.len(), n, "one ProduceResult per input record");
    // Same key → same partition for every record.
    let part = results[0].partition;
    assert!(results.iter().all(|r| r.partition == part));
    // Offsets contiguous 0..n.
    for (i, r) in results.iter().enumerate() {
        assert_eq!(r.base_offset, i as i64, "offset {i} contiguous");
        assert_eq!(r.topic, topic);
        assert_eq!(r.records, 1);
    }
    assert_eq!(s.backend.count_for(&topic, part), n);
}

/// Cite: `KinesisUnitTest.testBatchesAreCorrect` — fanning a batch across two
/// distinct destination topics yields the correct per-topic counts.
///
/// Both topics share the tenant prefix so the tenant guard passes; each gets
/// its own monotonic offset sequence (the MemorySink keys offsets by
/// (topic, partition)), so each topic's first record is base_offset 0.
#[test]
fn dispatch_batch_fans_out_across_two_topics_with_independent_offsets() {
    let mut s = sink(4);
    let orders = format!("{TENANT}.{SERVER}.public.orders");
    let payments = format!("{TENANT}.{SERVER}.public.payments");

    let batch = vec![
        record(&orders, b"o-1", b"a"),
        record(&payments, b"p-1", b"b"),
        record(&orders, b"o-2", b"c"),
        record(&payments, b"p-2", b"d"),
        record(&orders, b"o-3", b"e"),
    ];

    let results = s.dispatch_batch(&batch).unwrap();
    assert_eq!(results.len(), 5);

    let orders_total: usize = (0..4).map(|p| s.backend.count_for(&orders, p)).sum();
    let payments_total: usize = (0..4).map(|p| s.backend.count_for(&payments, p)).sum();
    assert_eq!(orders_total, 3, "three records routed to orders");
    assert_eq!(payments_total, 2, "two records routed to payments");

    // Each destination topic starts its own offset sequence at 0.
    let first_orders = results.iter().find(|r| r.topic == orders).unwrap();
    let first_payments = results.iter().find(|r| r.topic == payments).unwrap();
    assert_eq!(first_orders.base_offset, 0);
    assert_eq!(first_payments.base_offset, 0);
}

/// Cite: `KinesisUnitTest.testEmptyRecords` — an empty batch returns Ok with no
/// results and never touches the backend.
#[test]
fn dispatch_batch_empty_returns_ok_empty_and_writes_nothing() {
    let mut s = sink(4);
    let results = s.dispatch_batch(&[]).unwrap();
    assert!(results.is_empty(), "empty batch → empty result vec");
    assert!(
        s.backend.log.is_empty(),
        "no backend produce for an empty batch"
    );
}

/// Per-key partitioning is deterministic, so the offset a record lands on is
/// exactly the count of prior records already produced to that partition.
/// Two records with the *same* key in one batch must occupy offsets 0 and 1 of
/// the same partition.
#[test]
fn dispatch_batch_same_key_records_share_partition_and_advance_offset() {
    let mut s = sink(8);
    let topic = format!("{TENANT}.{SERVER}.public.orders");
    let batch = vec![
        record(&topic, b"dup", b"first"),
        record(&topic, b"dup", b"second"),
    ];
    let results = s.dispatch_batch(&batch).unwrap();

    assert_eq!(results[0].partition, results[1].partition);
    assert_eq!(results[0].base_offset, 0);
    assert_eq!(results[1].base_offset, 1);
    assert_eq!(s.backend.count_for(&topic, results[0].partition), 2);
}

/// The router-computed partition is consistent with `TopicRouter::partition_for`
/// for the same tenant + key, confirming the sink delegates partition selection
/// to the router rather than honoring the record's own `partition` field.
#[test]
fn dispatch_batch_partition_matches_router_partition_for() {
    let partitions = 4;
    let mut s = sink(partitions);
    let router = TopicRouter::new(TENANT, SERVER, RoutingPolicy::SchemaTable);
    let topic = format!("{TENANT}.{SERVER}.public.orders");

    // Record carries a bogus `partition = 99`; the sink must ignore it.
    let mut rec = record(&topic, b"route-me", b"v");
    rec.partition = 99;
    let expected = router.partition_for(b"route-me", partitions);

    let results = s.dispatch_batch(std::slice::from_ref(&rec)).unwrap();
    assert_eq!(results[0].partition, expected);
    assert!(
        (0..partitions).contains(&results[0].partition),
        "partition within [0, partitions)"
    );
}

// ----------------------------------------------------------------------------
// SchemaRegistry::register — rejection branch (check_compat error propagation)
// ----------------------------------------------------------------------------

fn schema(subject: &str, fields: Vec<FieldDef>) -> Schema {
    Schema {
        subject: subject.into(),
        format: SchemaFormat::Avro,
        version: 1,
        fields,
    }
}

fn req(name: &str, ty: &str) -> FieldDef {
    FieldDef {
        name: name.into(),
        field_type: ty.into(),
        nullable: false,
        default: None,
    }
}

fn opt(name: &str, ty: &str) -> FieldDef {
    FieldDef {
        name: name.into(),
        field_type: ty.into(),
        nullable: true,
        default: None,
    }
}

/// Cite: Confluent `register` compatibility gate — under BACKWARD, evolving a
/// subject by adding a NEW REQUIRED field with no default is rejected.
///
/// In `register`, `check_compat(&latest, &schema)` runs `check_backward(next,
/// prev)`: the new schema (reader) has a required field `currency` absent from
/// the latest (writer) and with no default → `SchemaIncompatibility` Err. The
/// version is NOT appended, so `version_count` stays at 1 and `latest` keeps
/// the single-field shape.
#[test]
fn register_rejects_backward_incompatible_required_field_addition() {
    let mut r = SchemaRegistry::new(TENANT, Compatibility::Backward);
    let v1 = r.register(schema("orders.value", vec![req("id", "int64")])).unwrap();
    assert_eq!(v1, 1);

    let bad = r.register(schema(
        "orders.value",
        vec![req("id", "int64"), req("currency", "string")],
    ));
    assert!(bad.is_err(), "required-field addition breaks BACKWARD");
    assert_eq!(
        r.version_count("orders.value"),
        1,
        "rejected schema must not be appended"
    );
    let latest = r.latest("orders.value").unwrap();
    assert_eq!(latest.version, 1);
    assert_eq!(latest.fields.len(), 1, "latest unchanged after rejection");
}

/// Cite: Confluent BACKWARD also rejects an incompatible *type change* on an
/// existing field. `check_backward(next, prev)` finds `id` in both schemas with
/// differing `field_type` → Err; registry rejects and version_count is unchanged.
#[test]
fn register_rejects_backward_incompatible_type_change() {
    let mut r = SchemaRegistry::new(TENANT, Compatibility::Backward);
    r.register(schema("orders.value", vec![req("id", "int64")])).unwrap();

    let bad = r.register(schema("orders.value", vec![req("id", "string")]));
    assert!(bad.is_err(), "type change on existing field breaks BACKWARD");
    assert_eq!(r.version_count("orders.value"), 1);
}

/// Cite: Confluent FORWARD `register` gate — under FORWARD, `check_forward(next,
/// prev)` rejects when the NEW schema (writer) adds a required field with no
/// default. Confirms the registry routes FORWARD compatibility through the
/// correct `check_compat` arm and propagates its Err.
#[test]
fn register_rejects_forward_incompatible_required_field_addition() {
    let mut r = SchemaRegistry::new(TENANT, Compatibility::Forward);
    let v1 = r.register(schema("orders.value", vec![req("id", "int64")])).unwrap();
    assert_eq!(v1, 1);

    let bad = r.register(schema(
        "orders.value",
        vec![req("id", "int64"), req("memo", "string")],
    ));
    assert!(bad.is_err(), "required-field addition breaks FORWARD");
    assert_eq!(r.version_count("orders.value"), 1);

    // An OPTIONAL addition is accepted under FORWARD → version advances to 2.
    let v2 = r
        .register(schema(
            "orders.value",
            vec![req("id", "int64"), opt("memo", "string")],
        ))
        .unwrap();
    assert_eq!(v2, 2);
    assert_eq!(r.version_count("orders.value"), 2);
}
