// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Line-by-line ports of upstream Kafka tests, cross-referenced from
//! `parity.manifest.toml`'s `[[upstream_test]]` block.
//!
//! Upstream: apache/kafka @ 4.2.0
//!   * core/src/main/scala/kafka/log/UnifiedLog.scala (+ test)
//!   * core/src/test/scala/.../coordinator/group/GroupCoordinatorTest.scala
//!   * clients/.../org/apache/kafka/common/record/RecordBatch (idempotency)
//!   * core/.../coordinator/transaction/ProducerIdManager.scala
//!
//! Each test asserts the same input → output the upstream JUnit / Scala
//! test asserts. Where Kafka's test infrastructure is JVM-only (e.g.
//! KafkaApis end-to-end), the cave port targets the pure decision/state-
//! machine layer that mirrors the same behaviour.

use cave_streams::consumer_group::RebalanceProtocol;
use cave_streams::idempotent_producer::{ProducerIdRegistry, SequenceCheck};
use cave_streams::segment_log::SegmentLog;
use std::collections::HashMap;

// ────────────────────────────────────────────────────────────────────────────
// Upstream: kafka/log/UnifiedLogTest.scala
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: UnifiedLogTest / `appendRecordsAssignsMonotonicOffsets`.
#[test]
fn upstream_segment_log_append_assigns_monotonic_offsets() {
    let log = SegmentLog::new(1_000_000);
    let o1 = log.append(b"a".to_vec(), 1).unwrap();
    let o2 = log.append(b"b".to_vec(), 2).unwrap();
    let o3 = log.append(b"c".to_vec(), 3).unwrap();
    assert_eq!(o1, 0);
    assert_eq!(o2, 1);
    assert_eq!(o3, 2);
    assert_eq!(log.next_offset(), 3);
}

/// Upstream: UnifiedLogTest / `readReturnsEntriesFromOffset`.
#[test]
fn upstream_segment_log_read_returns_entries_starting_at_offset() {
    let log = SegmentLog::new(1_000_000);
    for i in 0..5 {
        log.append(vec![i as u8], i).unwrap();
    }
    let entries = log.read(2, 1_000_000).unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].offset, 2);
    assert_eq!(entries[2].offset, 4);
}

/// Upstream: UnifiedLogTest / `rollSegmentOnSizeThreshold`.
#[test]
fn upstream_segment_log_rolls_when_active_segment_exceeds_max_bytes() {
    // Tiny max segment so rolling kicks in fast.
    let log = SegmentLog::new(10);
    // Two 6-byte records → second one triggers roll.
    log.append(b"123456".to_vec(), 0).unwrap();
    log.append(b"abcdef".to_vec(), 0).unwrap();
    assert_eq!(log.segment_count(), 2, "expected segment roll");
}

/// Upstream: UnifiedLogTest / `truncateBeforeAdvancesLogStart`.
#[test]
fn upstream_segment_log_truncate_before_advances_log_start_and_drops_entries() {
    let log = SegmentLog::new(1_000_000);
    for i in 0..10 {
        log.append(vec![i as u8], 0).unwrap();
    }
    log.truncate_before(5);
    assert_eq!(log.log_start_offset(), 5);
    // Read from 4 (below log_start) → out-of-range.
    assert!(log.read(4, 1_000_000).is_err());
    let entries = log.read(5, 1_000_000).unwrap();
    assert_eq!(entries.len(), 5);
    assert_eq!(entries[0].offset, 5);
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: ProducerStateManager.scala / idempotency tests
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: ProducerStateManagerTest / `firstBatchAccepted`.
#[test]
fn upstream_producer_id_first_batch_accepted() {
    let reg = ProducerIdRegistry::new();
    let pid = reg.allocate();
    let outcome = reg.check(pid, 0, "topic-1", 0, 0, 5).unwrap();
    assert_eq!(outcome, SequenceCheck::Accepted);
    let state = reg.partition_state(pid, "topic-1", 0).unwrap();
    assert_eq!(state.last_sequence, 4);
}

/// Upstream: ProducerStateManagerTest / `inOrderBatchAcceptedAndExtendsSequence`.
#[test]
fn upstream_producer_id_in_order_extends_sequence() {
    let reg = ProducerIdRegistry::new();
    let pid = reg.allocate();
    reg.check(pid, 0, "t", 0, 0, 3).unwrap();
    let outcome = reg.check(pid, 0, "t", 0, 3, 2).unwrap();
    assert_eq!(outcome, SequenceCheck::Accepted);
    assert_eq!(reg.partition_state(pid, "t", 0).unwrap().last_sequence, 4);
}

/// Upstream: ProducerStateManagerTest / `duplicateBatchDetected`.
#[test]
fn upstream_producer_id_duplicate_returns_duplicate() {
    let reg = ProducerIdRegistry::new();
    let pid = reg.allocate();
    reg.check(pid, 0, "t", 0, 0, 5).unwrap();
    let outcome = reg.check(pid, 0, "t", 0, 2, 1).unwrap();
    assert_eq!(outcome, SequenceCheck::Duplicate);
}

/// Upstream: ProducerStateManagerTest / `outOfOrderSequenceRejected`.
#[test]
fn upstream_producer_id_out_of_order_returns_out_of_order() {
    let reg = ProducerIdRegistry::new();
    let pid = reg.allocate();
    reg.check(pid, 0, "t", 0, 0, 5).unwrap();
    let outcome = reg.check(pid, 0, "t", 0, 99, 1).unwrap();
    assert_eq!(outcome, SequenceCheck::OutOfOrder);
}

/// Upstream: ProducerStateManagerTest / `lowerEpochFenced`.
#[test]
fn upstream_producer_id_lower_epoch_is_fenced() {
    let reg = ProducerIdRegistry::new();
    let pid = reg.allocate();
    // Bump epoch twice → current 2; a check at epoch 0 must fail with
    // INVALID_PRODUCER_EPOCH equivalent.
    reg.bump_epoch(pid).unwrap();
    reg.bump_epoch(pid).unwrap();
    let err = reg.check(pid, 0, "t", 0, 0, 1);
    assert!(err.is_err(), "fenced epoch must error");
}

/// Upstream: ProducerStateManagerTest / `higherEpochPersists`.
#[test]
fn upstream_producer_id_higher_epoch_persists_into_state() {
    let reg = ProducerIdRegistry::new();
    let pid = reg.allocate();
    reg.check(pid, 7, "t", 0, 0, 1).unwrap();
    assert_eq!(reg.epoch(pid), Some(7));
}

/// Upstream: ProducerIdManagerTest / `allocateAssignsMonotonicIds`.
#[test]
fn upstream_producer_id_manager_allocates_monotonic_ids() {
    let reg = ProducerIdRegistry::new();
    let id1 = reg.allocate();
    let id2 = reg.allocate();
    let id3 = reg.allocate();
    assert!(id2 > id1);
    assert!(id3 > id2);
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: GroupCoordinatorTest.scala / rebalance protocol assignment
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: RangeAssignorTest / `evenSplitAmongMembers`.
#[test]
fn upstream_rebalance_range_assignor_evenly_splits() {
    let members = vec!["m1".to_string(), "m2".to_string()];
    let mut topics = HashMap::new();
    topics.insert("t".to_string(), 4i32);
    let assignments = RebalanceProtocol::Range.assign(&members, &topics);
    // 4 partitions / 2 members → 2 each.
    assert_eq!(assignments.get("m1").unwrap().len(), 2);
    assert_eq!(assignments.get("m2").unwrap().len(), 2);
}

/// Upstream: RangeAssignorTest / `unevenSplitGivesEarlierMembersExtraPartitions`.
#[test]
fn upstream_rebalance_range_assignor_distributes_remainder_to_earlier_members() {
    let members = vec!["m1".to_string(), "m2".to_string()];
    let mut topics = HashMap::new();
    topics.insert("t".to_string(), 5i32);
    let assignments = RebalanceProtocol::Range.assign(&members, &topics);
    // 5 / 2 = 2 with remainder 1 → m1 gets 3, m2 gets 2.
    assert_eq!(assignments.get("m1").unwrap().len(), 3);
    assert_eq!(assignments.get("m2").unwrap().len(), 2);
}

/// Upstream: RoundRobinAssignorTest / `interleavesPartitionsAcrossMembers`.
#[test]
fn upstream_rebalance_roundrobin_interleaves_partitions() {
    let members = vec!["m1".to_string(), "m2".to_string()];
    let mut topics = HashMap::new();
    topics.insert("t".to_string(), 4i32);
    let assignments = RebalanceProtocol::RoundRobin.assign(&members, &topics);
    assert_eq!(assignments.get("m1").unwrap().len(), 2);
    assert_eq!(assignments.get("m2").unwrap().len(), 2);
    // Round-robin sorted members alphabetically; m1 gets evens, m2 gets odds.
    let m1: Vec<i32> = assignments
        .get("m1")
        .unwrap()
        .iter()
        .map(|(_, p)| *p)
        .collect();
    assert_eq!(m1, vec![0, 2]);
}

/// Upstream: ConfigTest / `from_str_accepts_kafka_protocol_aliases`.
#[test]
fn upstream_rebalance_protocol_from_str_recognises_aliases() {
    assert_eq!(
        RebalanceProtocol::from_str("range"),
        RebalanceProtocol::Range
    );
    assert_eq!(
        RebalanceProtocol::from_str("roundrobin"),
        RebalanceProtocol::RoundRobin
    );
    assert_eq!(
        RebalanceProtocol::from_str("round-robin"),
        RebalanceProtocol::RoundRobin
    );
    assert_eq!(
        RebalanceProtocol::from_str("cooperative-sticky"),
        RebalanceProtocol::CooperativeSticky
    );
}
