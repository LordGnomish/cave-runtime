// SPDX-License-Identifier: AGPL-3.0-or-later
//! Integration tests for cave-streams.
//!
//! Covers: produce/consume, consumer groups, offsets, compaction, schema
//! registry, exactly-once semantics, tiered storage, connectors, and the
//! Streams API — at least 25 tests total.

use crate::admin::AdminClient;
use crate::compaction::CompactionEngine;
use crate::connect::{ConnectorRegistry, NoOpSinkConnector, NoOpSourceConnector, SinkConnector, SourceConnector};
use crate::consumer::{Consumer, GroupAdmin};
use crate::error::StreamError;
use crate::models::{
    AggregationType, CleanupPolicy, CompatibilityMode, ConnectorConfig, ConnectorDirection,
    ConnectorStatus, PartitionerStrategy, ProducerRecord, RebalanceProtocol, SchemaType,
    StorageTierConfig, StreamOperation, TopicConfig, TopicPartition,
};
use crate::producer::{Producer, ProducerRecordBuilder};
use crate::schema_registry::SchemaRegistry;
use crate::storage::{MemoryStorage, StreamStorage};
use crate::streams_api::{execute_batch, PipelineRegistry, StreamPipelineBuilder};
use crate::topic::TopicManager;

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn store() -> MemoryStorage {
    MemoryStorage::new()
}

fn make_topic(storage: &MemoryStorage, name: &str, partitions: u32) {
    TopicManager::new(storage.clone())
        .create(name, partitions, 1, None)
        .expect("create topic");
}

// ─── 1. Create topic — success ────────────────────────────────────────────────

#[test]
fn test_create_topic() {
    let s = store();
    let topic = TopicManager::new(s.clone())
        .create("orders", 3, 1, None)
        .unwrap();
    assert_eq!(topic.name, "orders");
    assert_eq!(topic.partitions, 3);
}

// ─── 2. Create topic — duplicate fails ───────────────────────────────────────

#[test]
fn test_create_topic_duplicate() {
    let s = store();
    make_topic(&s, "events", 1);
    let result = TopicManager::new(s).create("events", 1, 1, None);
    assert!(matches!(result, Err(StreamError::TopicExists(_))));
}

// ─── 3. Delete topic ─────────────────────────────────────────────────────────

#[test]
fn test_delete_topic() {
    let s = store();
    make_topic(&s, "tmp", 1);
    TopicManager::new(s.clone()).delete("tmp").unwrap();
    let result = s.get_topic("tmp").unwrap();
    assert!(result.is_none());
}

// ─── 4. Delete non-existent topic returns error ───────────────────────────────

#[test]
fn test_delete_missing_topic() {
    let s = store();
    let result = TopicManager::new(s).delete("ghost");
    assert!(matches!(result, Err(StreamError::TopicNotFound(_))));
}

// ─── 5. Produce to topic — basic ─────────────────────────────────────────────

#[test]
fn test_produce_basic() {
    let s = store();
    make_topic(&s, "clicks", 1);
    let producer = Producer::new(s.clone()).unwrap();
    let meta = producer
        .send(
            ProducerRecordBuilder::new("clicks")
                .key("k1")
                .value("hello")
                .partitioner(PartitionerStrategy::Manual(0))
                .build(),
        )
        .unwrap();
    assert_eq!(meta.topic, "clicks");
    assert_eq!(meta.partition, 0);
    assert_eq!(meta.offset, 0);
}

// ─── 6. Produce with key-hash partitioning ────────────────────────────────────

#[test]
fn test_produce_key_hash_partitioning() {
    let s = store();
    make_topic(&s, "events", 4);
    let producer = Producer::new(s.clone()).unwrap();

    for i in 0..8u32 {
        let r = ProducerRecordBuilder::new("events")
            .key(i.to_be_bytes().to_vec())
            .value(format!("val-{i}"))
            .partitioner(PartitionerStrategy::KeyHash)
            .build();
        let meta = producer.send(r).unwrap();
        assert!(meta.partition < 4, "partition out of range");
    }
}

// ─── 7. Produce with round-robin partitioning ────────────────────────────────

#[test]
fn test_produce_round_robin() {
    let s = store();
    make_topic(&s, "fanout", 3);
    let producer = Producer::new(s.clone()).unwrap();
    let partitions: Vec<u32> = (0..6)
        .map(|_| {
            producer
                .send(
                    ProducerRecordBuilder::new("fanout")
                        .value("x")
                        .partitioner(PartitionerStrategy::RoundRobin)
                        .build(),
                )
                .unwrap()
                .partition
        })
        .collect();

    // All three partitions should be used.
    assert!(partitions.contains(&0));
    assert!(partitions.contains(&1));
    assert!(partitions.contains(&2));
}

// ─── 8. Consume from topic — basic fetch ─────────────────────────────────────

#[test]
fn test_consume_basic() {
    let s = store();
    make_topic(&s, "inbox", 1);
    let producer = Producer::new(s.clone()).unwrap();
    producer
        .send(
            ProducerRecordBuilder::new("inbox")
                .value("msg-1")
                .partitioner(PartitionerStrategy::Manual(0))
                .build(),
        )
        .unwrap();
    producer
        .send(
            ProducerRecordBuilder::new("inbox")
                .value("msg-2")
                .partitioner(PartitionerStrategy::Manual(0))
                .build(),
        )
        .unwrap();

    let records = s.fetch_from_partition("inbox", 0, 0, 10).unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].value.as_deref().unwrap(), b"msg-1");
    assert_eq!(records[1].value.as_deref().unwrap(), b"msg-2");
}

// ─── 9. Consume from offset ───────────────────────────────────────────────────

#[test]
fn test_consume_from_offset() {
    let s = store();
    make_topic(&s, "log", 1);
    let producer = Producer::new(s.clone()).unwrap();
    for i in 0..5u32 {
        producer
            .send(
                ProducerRecordBuilder::new("log")
                    .value(i.to_string())
                    .partitioner(PartitionerStrategy::Manual(0))
                    .build(),
            )
            .unwrap();
    }

    let records = s.fetch_from_partition("log", 0, 3, 10).unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].offset, 3);
    assert_eq!(records[1].offset, 4);
}

// ─── 10. Consumer group — join and sync ──────────────────────────────────────

#[test]
fn test_consumer_group_join_sync() {
    let s = store();
    make_topic(&s, "topic-a", 3);

    let mut consumer = Consumer::new(
        s.clone(),
        "group-1",
        "client-1",
        vec!["topic-a".into()],
        RebalanceProtocol::Eager,
    );

    let generation = consumer.join().unwrap();
    assert!(generation >= 1);

    let assignments = consumer.sync().unwrap();
    // Should have got at least one partition.
    assert!(!assignments.is_empty());
}

// ─── 11. Consumer group — two members rebalance ───────────────────────────────

#[test]
fn test_consumer_group_rebalance_two_members() {
    let s = store();
    make_topic(&s, "shared", 4);

    let mut c1 = Consumer::new(
        s.clone(),
        "grp",
        "c1",
        vec!["shared".into()],
        RebalanceProtocol::Eager,
    );
    let mut c2 = Consumer::new(
        s.clone(),
        "grp",
        "c2",
        vec!["shared".into()],
        RebalanceProtocol::Eager,
    );

    c1.join().unwrap();
    c2.join().unwrap();

    let a1 = c1.sync().unwrap();
    let a2 = c2.sync().unwrap();

    // Together they must cover all 4 partitions without overlap.
    let total = a1.len() + a2.len();
    assert_eq!(total, 4);

    let all: Vec<_> = a1.iter().chain(a2.iter()).collect();
    for p in 0..4u32 {
        assert!(
            all.iter().any(|tp| tp.topic == "shared" && tp.partition == p),
            "partition {p} not assigned"
        );
    }
}

// ─── 12. Manual offset commit and fetch ──────────────────────────────────────

#[test]
fn test_manual_offset_commit() {
    let s = store();
    make_topic(&s, "t", 1);
    s.commit_offset("grp-a", "t", 0, 42).unwrap();
    let off = s.get_offset("grp-a", "t", 0).unwrap();
    assert_eq!(off, 42);
}

// ─── 13. Consumer auto-commit ────────────────────────────────────────────────

#[test]
fn test_consumer_commit_offsets() {
    let s = store();
    make_topic(&s, "stream", 1);
    let producer = Producer::new(s.clone()).unwrap();
    producer
        .send(
            ProducerRecordBuilder::new("stream")
                .value("data")
                .partitioner(PartitionerStrategy::Manual(0))
                .build(),
        )
        .unwrap();

    let mut consumer = Consumer::new(
        s.clone(),
        "auto-grp",
        "c",
        vec!["stream".into()],
        RebalanceProtocol::Eager,
    )
    .disable_auto_commit();

    consumer.join().unwrap();
    consumer.sync().unwrap();
    let records = consumer.poll(10).unwrap();
    assert_eq!(records.len(), 1);
    consumer.commit_offsets().unwrap();

    let committed = s.get_offset("auto-grp", "stream", 0).unwrap();
    assert_eq!(committed, 1);
}

// ─── 14. Consumer seek ───────────────────────────────────────────────────────

#[test]
fn test_consumer_seek() {
    let s = store();
    make_topic(&s, "seekable", 1);
    let producer = Producer::new(s.clone()).unwrap();
    for i in 0..5u32 {
        producer
            .send(
                ProducerRecordBuilder::new("seekable")
                    .value(i.to_string())
                    .partitioner(PartitionerStrategy::Manual(0))
                    .build(),
            )
            .unwrap();
    }

    let mut consumer = Consumer::new(
        s.clone(),
        "seek-grp",
        "c",
        vec!["seekable".into()],
        RebalanceProtocol::Eager,
    )
    .disable_auto_commit();

    consumer.join().unwrap();
    consumer.sync().unwrap();

    let tp = TopicPartition::new("seekable", 0);
    consumer.seek(tp, 3);
    let records = consumer.poll(10).unwrap();
    assert_eq!(records[0].offset, 3);
}

// ─── 15. Log compaction — basic ──────────────────────────────────────────────

#[test]
fn test_log_compaction_basic() {
    let s = store();
    TopicManager::new(s.clone())
        .create(
            "compact-topic",
            1,
            1,
            Some(TopicConfig {
                cleanup_policy: CleanupPolicy::Compact,
                ..Default::default()
            }),
        )
        .unwrap();

    let producer = Producer::new(s.clone()).unwrap();
    // Write key "a" three times — only the last should survive.
    for val in &["v1", "v2", "v3"] {
        producer
            .send(
                ProducerRecordBuilder::new("compact-topic")
                    .key("a")
                    .value(*val)
                    .partitioner(PartitionerStrategy::Manual(0))
                    .build(),
            )
            .unwrap();
    }

    let engine = CompactionEngine::new(s.clone());
    let result = engine.compact_partition("compact-topic", 0).unwrap();

    assert_eq!(result.records_before, 3);
    assert_eq!(result.records_after, 1);

    let remaining = s.fetch_from_partition("compact-topic", 0, 0, 10).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].value.as_deref().unwrap(), b"v3");
}

// ─── 16. Log compaction — keeps latest per key ───────────────────────────────

#[test]
fn test_log_compaction_multi_key() {
    let s = store();
    TopicManager::new(s.clone())
        .create(
            "multi-key",
            1,
            1,
            Some(TopicConfig {
                cleanup_policy: CleanupPolicy::Compact,
                ..Default::default()
            }),
        )
        .unwrap();

    let producer = Producer::new(s.clone()).unwrap();
    let ops = [("a", "a1"), ("b", "b1"), ("a", "a2"), ("c", "c1"), ("b", "b2")];
    for (k, v) in &ops {
        producer
            .send(
                ProducerRecordBuilder::new("multi-key")
                    .key(*k)
                    .value(*v)
                    .partitioner(PartitionerStrategy::Manual(0))
                    .build(),
            )
            .unwrap();
    }

    CompactionEngine::new(s.clone())
        .compact_partition("multi-key", 0)
        .unwrap();

    let remaining = s.fetch_from_partition("multi-key", 0, 0, 10).unwrap();
    assert_eq!(remaining.len(), 3, "Should have exactly 3 unique keys");

    // Validate latest values.
    let val = |key: &[u8]| {
        remaining
            .iter()
            .find(|r| r.key.as_deref() == Some(key))
            .and_then(|r| r.value.as_deref().map(|v| v.to_vec()))
    };
    assert_eq!(val(b"a").unwrap(), b"a2");
    assert_eq!(val(b"b").unwrap(), b"b2");
    assert_eq!(val(b"c").unwrap(), b"c1");
}

// ─── 17. Schema registry — register and retrieve ─────────────────────────────

#[test]
fn test_schema_register_and_get() {
    let s = store();
    let registry = SchemaRegistry::new(s.clone());
    let id = registry
        .register(
            "orders-value",
            SchemaType::JsonSchema,
            r#"{"type":"object","properties":{"id":{"type":"integer"}}}"#,
        )
        .unwrap();
    assert!(id > 0);

    let schema = registry.get_by_id(id).unwrap();
    assert_eq!(schema.subject, "orders-value");
    assert_eq!(schema.version, 1);
}

// ─── 18. Schema registry — deduplication ─────────────────────────────────────

#[test]
fn test_schema_deduplication() {
    let s = store();
    let registry = SchemaRegistry::new(s.clone());
    let def = r#"{"type":"object","properties":{"id":{"type":"integer"}}}"#;
    let id1 = registry
        .register("dedup-subject", SchemaType::JsonSchema, def)
        .unwrap();
    let id2 = registry
        .register("dedup-subject", SchemaType::JsonSchema, def)
        .unwrap();
    assert_eq!(id1, id2, "Same schema should return same id");
}

// ─── 19. Schema registry — BACKWARD compatibility check ──────────────────────

#[test]
fn test_schema_backward_compatibility() {
    let s = store();
    let registry = SchemaRegistry::new(s.clone());

    // v1: requires "id"
    registry
        .register(
            "compat-subject",
            SchemaType::JsonSchema,
            r#"{"type":"object","required":["id"],"properties":{"id":{"type":"integer"}}}"#,
        )
        .unwrap();

    // v2: adds optional "name" → backward compatible (new field, no default needed in JSON Schema)
    let id2 = registry
        .register(
            "compat-subject",
            SchemaType::JsonSchema,
            r#"{"type":"object","required":["id"],"properties":{"id":{"type":"integer"},"name":{"type":"string"}}}"#,
        )
        .unwrap();
    assert!(id2 > 0);
}

// ─── 20. Schema registry — BACKWARD incompatibility (add required field) ──────

#[test]
fn test_schema_backward_incompatible() {
    let s = store();
    let registry = SchemaRegistry::new(s.clone());

    registry
        .register(
            "incompat-subject",
            SchemaType::JsonSchema,
            r#"{"type":"object","properties":{"id":{"type":"integer"}}}"#,
        )
        .unwrap();

    // Add a required field — old data won't have it → BACKWARD incompatible.
    let result = registry.register(
        "incompat-subject",
        SchemaType::JsonSchema,
        r#"{"type":"object","required":["new_required"],"properties":{"id":{"type":"integer"},"new_required":{"type":"string"}}}"#,
    );
    assert!(
        result.is_err(),
        "Should fail: new required field breaks BACKWARD compat"
    );
}

// ─── 21. Exactly-once — idempotent producer deduplication ────────────────────

#[test]
fn test_idempotent_producer_sequence() {
    let s = store();
    make_topic(&s, "idem", 1);

    let producer = Producer::new_idempotent(s.clone()).unwrap();
    let pid = producer.producer_id();
    assert!(pid >= 0);

    // Two produces should get sequences 0 and 1.
    let m1 = producer
        .send(
            ProducerRecordBuilder::new("idem")
                .value("a")
                .partitioner(PartitionerStrategy::Manual(0))
                .build(),
        )
        .unwrap();
    let m2 = producer
        .send(
            ProducerRecordBuilder::new("idem")
                .value("b")
                .partitioner(PartitionerStrategy::Manual(0))
                .build(),
        )
        .unwrap();

    assert_eq!(m1.offset, 0);
    assert_eq!(m2.offset, 1);

    // Sequences should be tracked in producer state.
    let state = s.get_producer_state(pid).unwrap().unwrap();
    assert!(!state.last_sequence.is_empty());
}

// ─── 22. Exactly-once — transaction commit ────────────────────────────────────

#[test]
fn test_transaction_commit() {
    let s = store();
    make_topic(&s, "txn-topic", 1);

    let producer =
        Producer::new_transactional(s.clone(), "txn-1").unwrap();

    producer.begin_transaction().unwrap();
    producer
        .send(
            ProducerRecordBuilder::new("txn-topic")
                .value("buffered")
                .partitioner(PartitionerStrategy::Manual(0))
                .build(),
        )
        .unwrap();

    // Before commit: record should NOT be visible.
    let before = s.fetch_from_partition("txn-topic", 0, 0, 10).unwrap();
    assert_eq!(before.len(), 0, "Transactional records must not be visible before commit");

    let metas = producer.commit_transaction().unwrap();
    assert_eq!(metas.len(), 1);

    // After commit: record is visible.
    let after = s.fetch_from_partition("txn-topic", 0, 0, 10).unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].value.as_deref().unwrap(), b"buffered");
}

// ─── 23. Exactly-once — transaction abort ────────────────────────────────────

#[test]
fn test_transaction_abort() {
    let s = store();
    make_topic(&s, "abort-topic", 1);

    let producer =
        Producer::new_transactional(s.clone(), "txn-abort").unwrap();

    producer.begin_transaction().unwrap();
    producer
        .send(
            ProducerRecordBuilder::new("abort-topic")
                .value("should-be-discarded")
                .partitioner(PartitionerStrategy::Manual(0))
                .build(),
        )
        .unwrap();

    producer.abort_transaction().unwrap();

    let records = s.fetch_from_partition("abort-topic", 0, 0, 10).unwrap();
    assert_eq!(records.len(), 0, "Aborted records must not appear in the log");
}

// ─── 24. Tiered storage configuration ────────────────────────────────────────

#[test]
fn test_tiered_storage_config() {
    let s = store();
    let mut cfg = StorageTierConfig::default();
    cfg.enabled = true;
    cfg.cold.bucket = "my-archive-bucket".into();

    s.set_tier_config(cfg).unwrap();

    let loaded = s.get_tier_config().unwrap();
    assert!(loaded.enabled);
    assert_eq!(loaded.cold.bucket, "my-archive-bucket");
}

// ─── 25. Connector registration and lifecycle ─────────────────────────────────

#[test]
fn test_connector_lifecycle() {
    let s = store();
    let registry = ConnectorRegistry::new(s.clone());

    let cfg = ConnectorConfig {
        name: "my-sink".into(),
        connector_class: "com.example.MySinkConnector".into(),
        config: std::collections::HashMap::new(),
        topics: vec!["events".into()],
        direction: ConnectorDirection::Sink,
        status: ConnectorStatus::Running,
        tasks_max: 1,
    };

    registry.create(cfg.clone()).unwrap();

    let loaded = registry.get("my-sink").unwrap();
    assert_eq!(loaded.name, "my-sink");

    registry.pause("my-sink").unwrap();
    let paused = registry.get("my-sink").unwrap();
    assert_eq!(paused.status, ConnectorStatus::Paused);

    registry.resume("my-sink").unwrap();
    let resumed = registry.get("my-sink").unwrap();
    assert_eq!(resumed.status, ConnectorStatus::Running);

    registry.delete("my-sink").unwrap();
    assert!(matches!(
        registry.get("my-sink"),
        Err(StreamError::ConnectorNotFound(_))
    ));
}

// ─── 26. Streams API — map transformation ────────────────────────────────────

#[test]
fn test_streams_map() {
    let s = store();
    make_topic(&s, "input", 1);
    make_topic(&s, "output", 1);

    let producer = Producer::new(s.clone()).unwrap();
    producer
        .send(
            ProducerRecordBuilder::new("input")
                .value("hello")
                .partitioner(PartitionerStrategy::Manual(0))
                .build(),
        )
        .unwrap();

    let cfg = StreamPipelineBuilder::from("input")
        .map("uppercase")
        .to("output")
        .build();

    let result = execute_batch(&cfg, &s, 0, 10).unwrap();
    assert_eq!(result.records_processed, 1);
    assert_eq!(result.records_emitted, 1);

    // Verify the output record has the map header.
    let out = s.fetch_from_partition("output", 0, 0, 10).unwrap();
    assert_eq!(out.len(), 1);
    let has_map_header = out[0].headers.iter().any(|h| h.key == "cave.map");
    assert!(has_map_header);
}

// ─── 27. Streams API — filter transformation ─────────────────────────────────

#[test]
fn test_streams_filter() {
    let s = store();
    make_topic(&s, "raw", 1);
    make_topic(&s, "filtered", 1);

    let producer = Producer::new(s.clone()).unwrap();
    for msg in &["KEEP:this", "DISCARD:that", "KEEP:another"] {
        producer
            .send(
                ProducerRecordBuilder::new("raw")
                    .value(*msg)
                    .partitioner(PartitionerStrategy::Manual(0))
                    .build(),
            )
            .unwrap();
    }

    let cfg = StreamPipelineBuilder::from("raw")
        .filter("KEEP:")
        .to("filtered")
        .build();

    let result = execute_batch(&cfg, &s, 0, 10).unwrap();
    assert_eq!(result.records_processed, 2);
    assert_eq!(result.records_emitted, 2);
}

// ─── 28. Streams API — count aggregation ─────────────────────────────────────

#[test]
fn test_streams_count() {
    let s = store();
    make_topic(&s, "metrics", 1);
    make_topic(&s, "counts", 1);

    let producer = Producer::new(s.clone()).unwrap();
    for _ in 0..5 {
        producer
            .send(
                ProducerRecordBuilder::new("metrics")
                    .value("1")
                    .partitioner(PartitionerStrategy::Manual(0))
                    .build(),
            )
            .unwrap();
    }

    let cfg = StreamPipelineBuilder::from("metrics")
        .count(None)
        .to("counts")
        .build();

    let result = execute_batch(&cfg, &s, 0, 10).unwrap();
    assert_eq!(result.records_processed, 1); // count emits 1 record
    assert_eq!(result.records_emitted, 1);

    let out = s.fetch_from_partition("counts", 0, 0, 10).unwrap();
    let json: serde_json::Value =
        serde_json::from_slice(out[0].value.as_deref().unwrap()).unwrap();
    assert_eq!(json["count"].as_u64().unwrap(), 5);
}

// ─── 29. Admin — cluster info ─────────────────────────────────────────────────

#[test]
fn test_admin_cluster_info() {
    let s = store();
    make_topic(&s, "t1", 2);
    make_topic(&s, "t2", 4);

    let admin = AdminClient::new(s.clone());
    let info = admin.cluster_info();

    assert_eq!(info.topic_count, 2);
    assert_eq!(info.partition_count, 6);
    assert_eq!(info.broker_host, "localhost");
}

// ─── 30. Add partitions ───────────────────────────────────────────────────────

#[test]
fn test_add_partitions() {
    let s = store();
    make_topic(&s, "growable", 2);

    let admin = AdminClient::new(s.clone());
    let updated = admin.add_partitions("growable", 5).unwrap();
    assert_eq!(updated.partitions, 5);

    // Produce to the new partition.
    let producer = Producer::new(s.clone()).unwrap();
    let meta = producer
        .send(
            ProducerRecordBuilder::new("growable")
                .value("new-partition-msg")
                .partitioner(PartitionerStrategy::Manual(4))
                .build(),
        )
        .unwrap();
    assert_eq!(meta.partition, 4);
}

// ─── 31. Retention enforcement ────────────────────────────────────────────────

#[test]
fn test_retention_enforcement() {
    let s = store();
    TopicManager::new(s.clone())
        .create(
            "short-lived",
            1,
            1,
            Some(TopicConfig {
                retention_ms: Some(1), // 1 ms — everything will be stale
                ..Default::default()
            }),
        )
        .unwrap();

    let producer = Producer::new(s.clone()).unwrap();
    for _ in 0..3 {
        producer
            .send(
                ProducerRecordBuilder::new("short-lived")
                    .value("old-data")
                    .partitioner(PartitionerStrategy::Manual(0))
                    .build(),
            )
            .unwrap();
    }

    // Sleep 2ms so records are older than retention_ms.
    std::thread::sleep(std::time::Duration::from_millis(2));

    let stats = CompactionEngine::new(s.clone())
        .enforce_retention_all()
        .unwrap();

    assert!(stats.records_deleted >= 3);
}

// ─── 32. Offset out-of-range returns empty vec ────────────────────────────────

#[test]
fn test_fetch_offset_out_of_range() {
    let s = store();
    make_topic(&s, "bounded", 1);

    let records = s
        .fetch_from_partition("bounded", 0, 9999, 10)
        .unwrap();
    assert_eq!(records.len(), 0);
}

// ─── 33. Producer message too large ──────────────────────────────────────────

#[test]
fn test_message_too_large() {
    let s = store();
    TopicManager::new(s.clone())
        .create(
            "tiny",
            1,
            1,
            Some(TopicConfig {
                max_message_bytes: 10, // very small
                ..Default::default()
            }),
        )
        .unwrap();

    let producer = Producer::new(s.clone()).unwrap();
    let result = producer.send(
        ProducerRecordBuilder::new("tiny")
            .value("this message is way too large for the limit")
            .partitioner(PartitionerStrategy::Manual(0))
            .build(),
    );
    assert!(matches!(result, Err(StreamError::MessageTooLarge { .. })));
}

// ─── 34. Avro schema registration ────────────────────────────────────────────

#[test]
fn test_avro_schema_register() {
    let s = store();
    let registry = SchemaRegistry::new(s.clone());

    let avro_schema = r#"{
        "type": "record",
        "name": "User",
        "fields": [
            {"name": "id", "type": "int"},
            {"name": "name", "type": "string"}
        ]
    }"#;

    let id = registry
        .register("users-value", SchemaType::Avro, avro_schema)
        .unwrap();
    assert!(id > 0);

    let schema = registry.get_latest("users-value").unwrap();
    assert_eq!(schema.schema_type, SchemaType::Avro);
}

// ─── 35. Consumer group — cooperative sticky ──────────────────────────────────

#[test]
fn test_cooperative_sticky_rebalance() {
    let s = store();
    make_topic(&s, "sticky-topic", 4);

    let mut c1 = Consumer::new(
        s.clone(),
        "sticky-grp",
        "c1",
        vec!["sticky-topic".into()],
        RebalanceProtocol::CooperativeSticky,
    );

    c1.join().unwrap();
    let assignments = c1.sync().unwrap();
    assert!(!assignments.is_empty());
}

// ─── 36. NoOp connector — source and sink ────────────────────────────────────

#[test]
fn test_noop_connectors() {
    let mut source = NoOpSourceConnector::new("test-source");
    source.start(&std::collections::HashMap::new()).unwrap();
    assert_eq!(source.status(), ConnectorStatus::Running);
    let records = source.poll().unwrap();
    assert!(records.is_empty());
    source.stop().unwrap();
    assert_eq!(source.status(), ConnectorStatus::Stopped);

    let mut sink = NoOpSinkConnector::new("test-sink");
    sink.start(&std::collections::HashMap::new()).unwrap();
    sink.put(vec![
        crate::connect::ConnectorRecord {
            topic: "t".into(),
            partition: None,
            key: None,
            value: b"hello".to_vec(),
            headers: vec![],
            timestamp_ms: None,
        }
    ])
    .unwrap();
    assert_eq!(sink.received.len(), 1);
}

// ─── 37. High watermark ──────────────────────────────────────────────────────

#[test]
fn test_high_watermark() {
    let s = store();
    make_topic(&s, "hwm-topic", 1);

    assert_eq!(s.high_watermark("hwm-topic", 0).unwrap(), 0);

    let producer = Producer::new(s.clone()).unwrap();
    for _ in 0..7 {
        producer
            .send(
                ProducerRecordBuilder::new("hwm-topic")
                    .value("x")
                    .partitioner(PartitionerStrategy::Manual(0))
                    .build(),
            )
            .unwrap();
    }

    assert_eq!(s.high_watermark("hwm-topic", 0).unwrap(), 7);
}
