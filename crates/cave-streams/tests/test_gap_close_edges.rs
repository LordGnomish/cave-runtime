// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Gap-close edge tests for cave-streams.
//!
//! Focus areas, by intent:
//!  * compression: codec name/byte parsing + empty/single-byte round-trips
//!  * error: Display + kafka_error_code coverage for all mapped variants
//!  * consumer_group: enum serde round-trip, partition assignment
//!  * idempotent_producer: state transitions + sequence-check matrix
//!  * partitioned_topic + pulsar_topic: parse + routing edges
//!  * tenant: registry life-cycle, autocreate toggle
//!  * schema_registry: enum parsing + duplicate dedup + delete behaviour
//!  * broker: Default config, topic name validation, partition arithmetic
//!  * concurrency smoke (Broker is Send+Sync — exercise it under Arc)

use std::collections::HashMap;
use std::sync::Arc;

use cave_streams::broker::{Broker, BrokerConfig, CleanupPolicy as BrokerCleanupPolicy};
use cave_streams::compression::{Codec, compress, decompress};
use cave_streams::consumer_group::{GroupState, RebalanceProtocol};
use cave_streams::error::{KafkaErrorCode, StreamsError};
use cave_streams::idempotent_producer::{
    PartitionSequence, ProducerIdRegistry, SequenceCheck,
};
use cave_streams::partitioned_topic::{
    PartitionRoutingMode, PartitionedTopicRegistry, PersistencePolicy,
};
use cave_streams::pulsar_topic::{TopicDomain, TopicName};
use cave_streams::schema_registry::{CompatibilityLevel, SchemaFormat, SchemaRegistry};
use cave_streams::tenant::{
    DEFAULT_NAMESPACE, DEFAULT_TENANT, Namespace, Tenant, TenantRegistry,
};

// =============================================================================
//  compression
// =============================================================================

#[test]
fn codec_from_i8_handles_attributes_bits() {
    // Kafka encodes the codec in the low 3 bits of the attributes byte.
    assert_eq!(Codec::from_i8(0), Codec::None);
    assert_eq!(Codec::from_i8(1), Codec::Gzip);
    assert_eq!(Codec::from_i8(2), Codec::Snappy);
    assert_eq!(Codec::from_i8(3), Codec::Lz4);
    assert_eq!(Codec::from_i8(4), Codec::Zstd);
    // High bits must be ignored (Kafka uses them for transactional + control flags).
    assert_eq!(Codec::from_i8(0b1111_1001u8 as i8), Codec::Gzip);
    // Anything in the 0x07 mask that's not 1..=4 falls back to None (legacy slot 5..7).
    assert_eq!(Codec::from_i8(5), Codec::None);
    assert_eq!(Codec::from_i8(7), Codec::None);
}

#[test]
fn codec_from_name_is_case_insensitive_and_falls_back_to_none() {
    assert_eq!(Codec::from_name("GZIP"), Codec::Gzip);
    assert_eq!(Codec::from_name("Snappy"), Codec::Snappy);
    assert_eq!(Codec::from_name("LZ4"), Codec::Lz4);
    assert_eq!(Codec::from_name("ZSTD"), Codec::Zstd);
    assert_eq!(Codec::from_name("none"), Codec::None);
    // Unknown labels degrade to None rather than erroring.
    assert_eq!(Codec::from_name(""), Codec::None);
    assert_eq!(Codec::from_name("brotli"), Codec::None);
}

#[test]
fn codec_name_is_self_consistent_with_from_name() {
    for c in [Codec::None, Codec::Gzip, Codec::Snappy, Codec::Lz4, Codec::Zstd] {
        assert_eq!(Codec::from_name(c.name()), c);
    }
}

#[test]
fn compression_round_trip_empty_payload_all_codecs() {
    let empty: &[u8] = &[];
    for c in [Codec::None, Codec::Gzip, Codec::Snappy, Codec::Lz4, Codec::Zstd] {
        let compressed = compress(c, empty).expect("empty compress");
        let back = decompress(c, &compressed).expect("empty decompress");
        assert_eq!(&back[..], empty, "round trip failed for {:?}", c);
    }
}

#[test]
fn codec_serde_round_trip_round_trips() {
    for c in [Codec::None, Codec::Gzip, Codec::Snappy, Codec::Lz4, Codec::Zstd] {
        let s = serde_json::to_string(&c).unwrap();
        let back: Codec = serde_json::from_str(&s).unwrap();
        assert_eq!(back, c);
    }
}

#[test]
fn compression_round_trip_large_repeating_payload() {
    // Highly compressible — gzip/zstd/lz4 should all easily shrink it.
    let big: Vec<u8> = (0..16 * 1024).map(|i| (i % 251) as u8).collect();
    for c in [Codec::Gzip, Codec::Snappy, Codec::Lz4, Codec::Zstd] {
        let out = compress(c, &big).unwrap();
        let back = decompress(c, &out).unwrap();
        assert_eq!(back.len(), big.len(), "len mismatch for {:?}", c);
        assert_eq!(&back[..], &big[..], "byte mismatch for {:?}", c);
    }
}

#[test]
fn compression_decompress_garbage_is_error() {
    // None codec accepts arbitrary bytes; others must reject malformed data.
    for c in [Codec::Gzip, Codec::Lz4, Codec::Zstd] {
        let r = decompress(c, b"not a real compressed stream");
        assert!(r.is_err(), "{:?} accepted garbage input", c);
    }
}

// =============================================================================
//  error: Display + kafka_error_code mapping
// =============================================================================

#[test]
fn streams_error_display_includes_useful_context() {
    let e = StreamsError::UnknownTopicOrPartition {
        topic: "orders".into(),
        partition: 7,
    };
    let s = format!("{}", e);
    assert!(s.contains("orders"), "Display lost topic name");
    assert!(s.contains("7"), "Display lost partition number");

    let e = StreamsError::TopicAlreadyExists("dup".into());
    assert!(format!("{}", e).contains("dup"));

    let e = StreamsError::InvalidReplicationFactor {
        topic: "t".into(),
        factor: -3,
    };
    let s = format!("{}", e);
    assert!(s.contains("-3"));
    assert!(s.contains('t'));
}

#[test]
fn kafka_error_code_maps_known_variants() {
    assert_eq!(
        StreamsError::UnknownTopicOrPartition {
            topic: "a".into(),
            partition: 0,
        }
        .kafka_error_code(),
        KafkaErrorCode::UnknownTopicOrPartition as i16
    );
    assert_eq!(
        StreamsError::TopicAlreadyExists("x".into()).kafka_error_code(),
        KafkaErrorCode::TopicAlreadyExists as i16
    );
    assert_eq!(
        StreamsError::OffsetOutOfRange {
            topic: "t".into(),
            partition: 0,
            offset: 99
        }
        .kafka_error_code(),
        KafkaErrorCode::OffsetOutOfRange as i16
    );
    assert_eq!(
        StreamsError::RebalanceInProgress("g".into()).kafka_error_code(),
        KafkaErrorCode::RebalanceInProgress as i16
    );
    assert_eq!(
        StreamsError::IllegalGeneration {
            group: "g".into(),
            expected: 1,
            got: 0
        }
        .kafka_error_code(),
        KafkaErrorCode::IllegalGeneration as i16
    );
    assert_eq!(
        StreamsError::InconsistentGroupProtocol("p".into()).kafka_error_code(),
        KafkaErrorCode::InconsistentGroupProtocol as i16
    );
    assert_eq!(
        StreamsError::MemberNotFound {
            group: "g".into(),
            member: "m".into()
        }
        .kafka_error_code(),
        KafkaErrorCode::UnknownMemberId as i16
    );
    assert_eq!(
        StreamsError::DuplicateSequenceNumber {
            producer_id: 1,
            topic: "t".into(),
            partition: 0
        }
        .kafka_error_code(),
        KafkaErrorCode::DuplicateSequenceNumber as i16
    );
    assert_eq!(
        StreamsError::ProducerIdNotFound(42).kafka_error_code(),
        KafkaErrorCode::UnknownProducerId as i16
    );
    assert_eq!(
        StreamsError::SchemaIncompatible {
            subject: "s".into(),
            reason: "r".into()
        }
        .kafka_error_code(),
        KafkaErrorCode::SchemasNotCompatible as i16
    );
    assert_eq!(
        StreamsError::NotEnoughReplicas {
            required: 3,
            available: 1
        }
        .kafka_error_code(),
        KafkaErrorCode::NotEnoughReplicas as i16
    );
}

#[test]
fn kafka_error_code_unmapped_variants_return_minus_one() {
    // Internal/Compression/Io/etc. do not have direct Kafka counterparts.
    assert_eq!(StreamsError::Internal("oops".into()).kafka_error_code(), -1);
    assert_eq!(
        StreamsError::Compression {
            codec: "gzip".into(),
            message: "x".into(),
        }
        .kafka_error_code(),
        -1
    );
    assert_eq!(
        StreamsError::ProtocolDecode("bad".into()).kafka_error_code(),
        -1
    );
    assert_eq!(
        StreamsError::SubjectNotFound("missing".into()).kafka_error_code(),
        -1
    );
    assert_eq!(
        StreamsError::AclNotFound { resource: "x".into() }.kafka_error_code(),
        -1
    );
}

#[test]
fn io_error_can_be_wrapped_via_from() {
    let io = std::io::Error::new(std::io::ErrorKind::Other, "boom");
    let e: StreamsError = io.into();
    assert!(format!("{}", e).contains("boom"));
}

// =============================================================================
//  consumer_group: enum serde + assignment
// =============================================================================

#[test]
fn rebalance_protocol_serde_uses_kebab_case() {
    // serde(rename_all = "kebab-case")
    let s = serde_json::to_string(&RebalanceProtocol::CooperativeSticky).unwrap();
    assert_eq!(s, "\"cooperative-sticky\"");
    let s = serde_json::to_string(&RebalanceProtocol::RoundRobin).unwrap();
    assert_eq!(s, "\"round-robin\"");
    let back: RebalanceProtocol = serde_json::from_str("\"range\"").unwrap();
    assert_eq!(back, RebalanceProtocol::Range);
}

#[test]
fn rebalance_protocol_from_str_handles_aliases() {
    assert_eq!(RebalanceProtocol::from_str("range"), RebalanceProtocol::Range);
    assert_eq!(
        RebalanceProtocol::from_str("ROUND_ROBIN"),
        RebalanceProtocol::RoundRobin
    );
    assert_eq!(
        RebalanceProtocol::from_str("round-robin"),
        RebalanceProtocol::RoundRobin
    );
    assert_eq!(
        RebalanceProtocol::from_str("sticky"),
        RebalanceProtocol::Sticky
    );
    assert_eq!(
        RebalanceProtocol::from_str("cooperative-sticky"),
        RebalanceProtocol::CooperativeSticky
    );
    // Unknown → defaults to Range.
    assert_eq!(RebalanceProtocol::from_str("???"), RebalanceProtocol::Range);
}

#[test]
fn group_state_serde_uses_pascal_case() {
    // serde(rename_all = "PascalCase")
    let s = serde_json::to_string(&GroupState::PreparingRebalance).unwrap();
    assert_eq!(s, "\"PreparingRebalance\"");
    let back: GroupState = serde_json::from_str("\"Stable\"").unwrap();
    assert_eq!(back, GroupState::Stable);
    // Round-trip every variant.
    for st in [
        GroupState::Empty,
        GroupState::PreparingRebalance,
        GroupState::CompletingRebalance,
        GroupState::Stable,
        GroupState::Dead,
    ] {
        let s = serde_json::to_string(&st).unwrap();
        let back: GroupState = serde_json::from_str(&s).unwrap();
        assert_eq!(back, st);
    }
}

#[test]
fn rebalance_protocol_assign_handles_zero_members() {
    // Edge: assigning with no members must not panic and must return empty.
    let mut tp = HashMap::new();
    tp.insert("t".to_string(), 4_i32);
    let out = RebalanceProtocol::Range.assign(&[], &tp);
    assert!(out.is_empty(), "no members → no assignments");
    let out = RebalanceProtocol::RoundRobin.assign(&[], &tp);
    assert!(out.is_empty());
}

#[test]
fn rebalance_protocol_range_assigns_all_partitions_exactly_once() {
    // Property: every (topic, partition) is owned by exactly one member.
    let mut tp = HashMap::new();
    tp.insert("orders".to_string(), 7_i32);
    let members = vec!["a".to_string(), "b".into(), "c".into()];
    let out = RebalanceProtocol::Range.assign(&members, &tp);
    let mut seen: Vec<(String, i32)> = out.values().flat_map(|v| v.clone()).collect();
    seen.sort();
    let expected: Vec<(String, i32)> = (0..7).map(|p| ("orders".to_string(), p)).collect();
    assert_eq!(seen, expected, "partitions not partitioned exactly once");
}

#[test]
fn rebalance_protocol_roundrobin_balances_across_members() {
    let mut tp = HashMap::new();
    tp.insert("t".to_string(), 6_i32);
    let members = vec!["a".to_string(), "b".into(), "c".into()];
    let out = RebalanceProtocol::RoundRobin.assign(&members, &tp);
    // 6 / 3 = 2 per member exactly.
    for m in &members {
        assert_eq!(out[m].len(), 2, "member {} got {:?}", m, out[m]);
    }
}

// =============================================================================
//  idempotent_producer state transitions
// =============================================================================

#[test]
fn producer_id_registry_default_is_empty() {
    let r = ProducerIdRegistry::default();
    assert_eq!(r.active_producers(), 0);
    assert!(r.epoch(0).is_none());
}

#[test]
fn producer_id_registry_allocate_then_forget() {
    let r = ProducerIdRegistry::new();
    let id = r.allocate();
    assert_eq!(r.epoch(id), Some(0));
    assert_eq!(r.active_producers(), 1);
    r.forget(id);
    assert!(r.epoch(id).is_none());
    assert_eq!(r.active_producers(), 0);
}

#[test]
fn producer_id_registry_bump_epoch_for_unknown_errors() {
    let r = ProducerIdRegistry::new();
    let res = r.bump_epoch(9999);
    assert!(matches!(res, Err(StreamsError::ProducerIdNotFound(9999))));
}

#[test]
fn partition_sequence_default_is_minus_one() {
    let s = PartitionSequence::default();
    assert_eq!(s.last_sequence, -1);
    assert_eq!(s.wrap_count, 0);
}

#[test]
fn sequence_check_accept_duplicate_outoforder_matrix() {
    let r = ProducerIdRegistry::new();
    let pid = r.allocate();
    // First batch (base_sequence=0, 5 records) → Accepted.
    assert_eq!(
        r.check(pid, 0, "t", 0, 0, 5).unwrap(),
        SequenceCheck::Accepted
    );
    // Next consecutive (base=5, 3 records) → Accepted.
    assert_eq!(
        r.check(pid, 0, "t", 0, 5, 3).unwrap(),
        SequenceCheck::Accepted
    );
    // Replay an earlier batch (base=0) → Duplicate.
    assert_eq!(
        r.check(pid, 0, "t", 0, 0, 5).unwrap(),
        SequenceCheck::Duplicate
    );
    // Jump ahead (base=100) → OutOfOrder.
    assert_eq!(
        r.check(pid, 0, "t", 0, 100, 1).unwrap(),
        SequenceCheck::OutOfOrder
    );
    // The OutOfOrder probe must NOT mutate last_sequence.
    let st = r.partition_state(pid, "t", 0).unwrap();
    assert_eq!(st.last_sequence, 7, "OutOfOrder leaked state mutation");
}

#[test]
fn sequence_check_rejects_negative_base_sequence_on_first_batch() {
    let r = ProducerIdRegistry::new();
    let pid = r.allocate();
    let res = r.check(pid, 0, "t", 0, -5, 1);
    assert!(res.is_err());
}

#[test]
fn sequence_check_rejects_stale_epoch() {
    let r = ProducerIdRegistry::new();
    let pid = r.allocate();
    r.bump_epoch(pid).unwrap(); // now epoch=1
    let res = r.check(pid, 0, "t", 0, 0, 1);
    assert!(res.is_err(), "epoch=0 vs current=1 must error");
}

#[test]
fn sequence_check_new_higher_epoch_is_persisted() {
    let r = ProducerIdRegistry::new();
    let pid = r.allocate();
    // Client jumps to epoch=3 (fenced predecessor).
    r.check(pid, 3, "t", 0, 0, 1).unwrap();
    assert_eq!(r.epoch(pid), Some(3));
}

#[test]
fn forget_isolates_other_producers() {
    let r = ProducerIdRegistry::new();
    let p1 = r.allocate();
    let p2 = r.allocate();
    r.check(p1, 0, "t", 0, 0, 1).unwrap();
    r.check(p2, 0, "t", 0, 0, 1).unwrap();
    r.forget(p1);
    assert!(r.partition_state(p1, "t", 0).is_none());
    assert!(r.partition_state(p2, "t", 0).is_some());
    assert_eq!(r.active_producers(), 1);
}

#[test]
fn broker_cleanup_policy_serializes_lowercase() {
    // broker::CleanupPolicy has #[serde(rename_all = "lowercase")]
    let s = serde_json::to_string(&BrokerCleanupPolicy::Delete).unwrap();
    assert_eq!(s, "\"delete\"");
    let s = serde_json::to_string(&BrokerCleanupPolicy::Compact).unwrap();
    assert_eq!(s, "\"compact\"");
    let back: BrokerCleanupPolicy = serde_json::from_str("\"compactdelete\"").unwrap();
    assert_eq!(back, BrokerCleanupPolicy::CompactDelete);
}

// =============================================================================
//  pulsar_topic + partitioned_topic edges
// =============================================================================

#[test]
fn topic_domain_round_trip_via_strings() {
    assert_eq!(TopicDomain::parse("persistent").unwrap(), TopicDomain::Persistent);
    assert_eq!(
        TopicDomain::parse("non-persistent").unwrap(),
        TopicDomain::NonPersistent
    );
    assert!(TopicDomain::parse("memory").is_err());
    assert_eq!(TopicDomain::Persistent.as_scheme(), "persistent");
    assert_eq!(TopicDomain::NonPersistent.as_scheme(), "non-persistent");
}

#[test]
fn topic_name_namespace_fqn_format() {
    let t = TopicName::persistent("acme", "events", "orders").unwrap();
    assert_eq!(t.namespace_fqn(), "acme/events");
    assert_eq!(format!("{}", t), "persistent://acme/events/orders");
}

#[test]
fn topic_name_to_kafka_uses_default_prefix_skip() {
    // public/default → just the local name (no prefix).
    let t = TopicName::persistent(DEFAULT_TENANT, DEFAULT_NAMESPACE, "orders").unwrap();
    assert_eq!(t.to_kafka_topic(), "orders");
    // Non-default tenant/ns → full path.
    let t = TopicName::persistent("acme", "ns", "orders").unwrap();
    assert_eq!(t.to_kafka_topic(), "acme/ns/orders");
}

#[test]
fn topic_name_from_kafka_rejects_empty_input() {
    assert!(TopicName::from_kafka_topic("").is_err());
}

#[test]
fn topic_name_partition_of_zero_is_ok() {
    let root = TopicName::persistent("public", "default", "t").unwrap();
    let p = root.partition_of(0).unwrap();
    assert_eq!(p.partition, Some(0));
    assert!(p.to_string_full().ends_with("-partition-0"));
}

#[test]
fn partitioned_topic_route_single_partition_pins_to_zero() {
    let r = PartitionedTopicRegistry::new(Arc::new(TenantRegistry::default()));
    let t = TopicName::persistent(DEFAULT_TENANT, DEFAULT_NAMESPACE, "single").unwrap();
    r.create_partitioned_topic(&t, 4).unwrap();
    let p1 = r
        .route_message(&t, PartitionRoutingMode::SinglePartition, None)
        .unwrap();
    let p2 = r
        .route_message(&t, PartitionRoutingMode::SinglePartition, Some(b"any"))
        .unwrap();
    assert_eq!(p1.partition, Some(0));
    assert_eq!(p2.partition, Some(0));
}

#[test]
fn partitioned_topic_create_duplicate_errors() {
    let r = PartitionedTopicRegistry::new(Arc::new(TenantRegistry::default()));
    let t = TopicName::persistent(DEFAULT_TENANT, DEFAULT_NAMESPACE, "dup").unwrap();
    r.create_partitioned_topic(&t, 2).unwrap();
    assert!(r.create_partitioned_topic(&t, 2).is_err());
}

#[test]
fn partitioned_topic_delete_missing_errors() {
    let r = PartitionedTopicRegistry::new(Arc::new(TenantRegistry::default()));
    let t = TopicName::persistent(DEFAULT_TENANT, DEFAULT_NAMESPACE, "ghost").unwrap();
    assert!(r.delete_partitioned_topic(&t).is_err());
    assert!(!r.is_partitioned(&t));
}

#[test]
fn persistence_policy_for_topic_matches_domain() {
    let p = TopicName::persistent("acme", "ns", "a").unwrap();
    assert_eq!(PersistencePolicy::for_topic(&p), PersistencePolicy::Persistent);
    let np = TopicName::parse("non-persistent://acme/ns/a").unwrap();
    assert_eq!(
        PersistencePolicy::for_topic(&np),
        PersistencePolicy::NonPersistent
    );
}

// =============================================================================
//  tenant registry edges
// =============================================================================

#[test]
fn tenant_delete_missing_errors() {
    let r = TenantRegistry::new();
    assert!(r.delete_tenant("ghost").is_err());
}

#[test]
fn namespace_create_duplicate_errors() {
    let r = TenantRegistry::new();
    r.create_tenant(Tenant::new("acme")).unwrap();
    r.create_namespace(Namespace::new("acme", "ns")).unwrap();
    assert!(r.create_namespace(Namespace::new("acme", "ns")).is_err());
}

#[test]
fn namespace_delete_missing_errors() {
    let r = TenantRegistry::new();
    assert!(r.delete_namespace("ghost/ns").is_err());
}

#[test]
fn list_tenants_is_sorted() {
    let r = TenantRegistry::new();
    r.create_tenant(Tenant::new("zeta")).unwrap();
    r.create_tenant(Tenant::new("alpha")).unwrap();
    r.create_tenant(Tenant::new("mu")).unwrap();
    let list = r.list_tenants();
    assert_eq!(list, vec!["alpha".to_string(), "mu".into(), "zeta".into()]);
}

#[test]
fn autocreate_toggle_round_trip() {
    let r = TenantRegistry::new();
    assert!(r.autocreate_default());
    r.set_autocreate_default(false);
    assert!(!r.autocreate_default());
    r.set_autocreate_default(true);
    assert!(r.autocreate_default());
}

// =============================================================================
//  schema_registry edges
// =============================================================================

#[test]
fn schema_format_from_str_handles_aliases() {
    assert_eq!(SchemaFormat::from_str("AVRO"), SchemaFormat::Avro);
    assert_eq!(SchemaFormat::from_str("avro"), SchemaFormat::Avro);
    assert_eq!(SchemaFormat::from_str("PROTOBUF"), SchemaFormat::Protobuf);
    assert_eq!(SchemaFormat::from_str("JSON"), SchemaFormat::JsonSchema);
    assert_eq!(SchemaFormat::from_str("JSON_SCHEMA"), SchemaFormat::JsonSchema);
    assert_eq!(SchemaFormat::from_str("JSONSCHEMA"), SchemaFormat::JsonSchema);
    // Unknown → defaults to Avro.
    assert_eq!(SchemaFormat::from_str("BOGUS"), SchemaFormat::Avro);
}

#[test]
fn compatibility_level_parse_supports_dashes_and_underscores() {
    assert_eq!(
        CompatibilityLevel::from_str("backward-transitive"),
        CompatibilityLevel::BackwardTransitive
    );
    assert_eq!(
        CompatibilityLevel::from_str("FORWARD_TRANSITIVE"),
        CompatibilityLevel::ForwardTransitive
    );
    assert_eq!(CompatibilityLevel::from_str("none"), CompatibilityLevel::None);
    assert_eq!(CompatibilityLevel::from_str("full"), CompatibilityLevel::Full);
    // Unknown → Backward fallback (Confluent's documented default).
    assert_eq!(
        CompatibilityLevel::from_str("???"),
        CompatibilityLevel::Backward
    );
    assert_eq!(CompatibilityLevel::default(), CompatibilityLevel::Backward);
}

#[test]
fn schema_registry_delete_missing_subject_errors() {
    let r = SchemaRegistry::new();
    assert!(r.delete_subject("ghost").is_err());
    assert!(r.list_versions("ghost").is_err());
    assert!(r.get_latest_schema("ghost").is_err());
}

#[test]
fn schema_registry_global_compat_can_be_changed() {
    let r = SchemaRegistry::new();
    assert_eq!(r.get_global_compatibility(), CompatibilityLevel::Backward);
    r.set_global_compatibility(CompatibilityLevel::Full);
    assert_eq!(r.get_global_compatibility(), CompatibilityLevel::Full);
}

#[test]
fn schema_registry_register_twice_returns_same_id() {
    let r = SchemaRegistry::new();
    let schema = r#"{"type":"record","name":"X","fields":[]}"#.to_string();
    let id1 = r
        .register_schema("subj", schema.clone(), SchemaFormat::Avro, vec![])
        .unwrap();
    let id2 = r
        .register_schema("subj", schema, SchemaFormat::Avro, vec![])
        .unwrap();
    assert_eq!(id1, id2);
}

#[test]
fn schema_registry_subject_compatibility_overrides_global() {
    let r = SchemaRegistry::new();
    r.set_global_compatibility(CompatibilityLevel::None);
    r.set_subject_compatibility("strict", CompatibilityLevel::Full);
    assert_eq!(
        r.get_subject_compatibility("strict"),
        Some(CompatibilityLevel::Full)
    );
    assert_eq!(r.get_subject_compatibility("other"), None);
}

// =============================================================================
//  broker config + name validation + concurrency
// =============================================================================

#[test]
fn broker_config_default_is_kafka_compatible() {
    let c = BrokerConfig::default();
    assert_eq!(c.port, 9092);
    assert!(c.broker_id >= 1);
    assert!(c.log_segment_bytes > 0);
    assert!(c.message_max_bytes > 0);
    assert_eq!(c.default_replication_factor, 1);
}

#[test]
fn broker_rejects_invalid_topic_names() {
    let b = Broker::new(BrokerConfig::default());
    // empty
    assert!(b.create_topic(String::new(), 1, 1, vec![]).is_err());
    // too long
    let huge: String = (0..300).map(|_| 'a').collect();
    assert!(b.create_topic(huge, 1, 1, vec![]).is_err());
    // forbidden chars
    assert!(b.create_topic("with space".into(), 1, 1, vec![]).is_err());
    assert!(b.create_topic("with/slash".into(), 1, 1, vec![]).is_err());
}

#[test]
fn broker_duplicate_topic_errors() {
    let b = Broker::new(BrokerConfig::default());
    b.create_topic("orders".into(), 3, 1, vec![]).unwrap();
    let again = b.create_topic("orders".into(), 3, 1, vec![]);
    assert!(matches!(again, Err(StreamsError::TopicAlreadyExists(_))));
}

#[test]
fn broker_partition_count_query_round_trip() {
    let b = Broker::new(BrokerConfig::default());
    b.create_topic("events".into(), 5, 1, vec![]).unwrap();
    assert_eq!(b.topic_partition_count("events").unwrap(), 5);
    assert!(b.topic_exists("events"));
    assert!(!b.topic_exists("nope"));
}

#[test]
fn broker_add_partitions_grows_count() {
    let b = Broker::new(BrokerConfig::default());
    b.create_topic("grow".into(), 2, 1, vec![]).unwrap();
    b.add_partitions("grow", 5).unwrap();
    assert_eq!(b.topic_partition_count("grow").unwrap(), 5);
    // Shrinking is rejected.
    assert!(b.add_partitions("grow", 3).is_err());
}

#[test]
fn broker_delete_topic_idempotent_only_first_call() {
    let b = Broker::new(BrokerConfig::default());
    b.create_topic("temp".into(), 1, 1, vec![]).unwrap();
    b.delete_topic("temp").unwrap();
    assert!(b.delete_topic("temp").is_err());
}

#[test]
fn broker_allocate_producer_id_is_monotonic() {
    let b = Broker::new(BrokerConfig::default());
    let first = b.allocate_producer_id();
    let second = b.allocate_producer_id();
    let third = b.allocate_producer_id();
    assert!(second > first);
    assert!(third > second);
}

#[test]
fn broker_committed_offset_round_trip() {
    let b = Broker::new(BrokerConfig::default());
    // Unknown group/topic/partition → sentinel -1.
    assert_eq!(b.fetch_offset("g", "t", 0), -1);
    b.commit_offset("g", "t", 0, 42);
    assert_eq!(b.fetch_offset("g", "t", 0), 42);
    b.commit_offset("g", "t", 0, 100);
    assert_eq!(b.fetch_offset("g", "t", 0), 100);
    // Distinct (group, topic, partition) keys are isolated.
    assert_eq!(b.fetch_offset("other", "t", 0), -1);
    assert_eq!(b.fetch_offset("g", "t", 1), -1);
}

#[test]
fn broker_metadata_accessors_match_config() {
    let mut cfg = BrokerConfig::default();
    cfg.broker_id = 7;
    let b = Broker::new(cfg);
    assert_eq!(b.broker_id(), 7);
    assert_eq!(b.controller_id(), 7);
    assert_eq!(b.cluster_id(), "cave-streams-cluster");
}

#[test]
fn broker_is_send_sync_concurrent_producer_id_alloc() {
    // Send+Sync check by use: a real Arc<Broker> handed to threads.
    let b = Arc::new(Broker::new(BrokerConfig::default()));
    let mut handles = Vec::new();
    for _ in 0..4 {
        let b = b.clone();
        handles.push(std::thread::spawn(move || {
            let mut local = Vec::new();
            for _ in 0..50 {
                local.push(b.allocate_producer_id());
            }
            local
        }));
    }
    let mut all: Vec<i64> = handles.into_iter().flat_map(|h| h.join().unwrap()).collect();
    all.sort();
    // Every allocated ID must be unique even under contention.
    let len = all.len();
    all.dedup();
    assert_eq!(all.len(), len, "producer IDs were not unique across threads");
}

#[test]
fn broker_reassignment_lifecycle() {
    let b = Broker::new(BrokerConfig::default());
    b.create_topic("re".into(), 3, 1, vec![]).unwrap();
    let mut targets = std::collections::HashMap::new();
    targets.insert(0_i32, vec![1, 2]);
    targets.insert(1_i32, vec![2, 3]);
    b.start_reassignment("re", targets.clone()).unwrap();
    let list = b.list_reassignments();
    assert_eq!(list.get("re").unwrap().len(), 2);
    b.cancel_reassignment("re");
    assert!(b.list_reassignments().is_empty());
    // Reassignment for an unknown topic is rejected.
    assert!(b.start_reassignment("ghost", targets).is_err());
}
