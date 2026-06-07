// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Unified-streaming acceptance suite.
//!
//! Ties the Charter acceptance checklist into one cohesive integration
//! test over the *public* `cave_streams` surface, spanning both upstream
//! wire protocols:
//!   * Pulsar transactional messaging (PIP-31) — commit publishes, abort
//!     stays invisible (the two-phase-commit invariant);
//!   * Kafka share groups (KIP-932) — acquire / acknowledge / SPSO advance;
//!   * Pulsar multi-region geo-replication — loop-guard + per-peer fan-out;
//!   * Schema Registry compatibility — BACKWARD add-optional-field.
//!
//! The Kafka and Pulsar produce/consume wire round-trips are covered by the
//! in-module unit tests `kafka_wire::tests::test_kafka_produce_roundtrip`
//! and `pulsar_wire::tests::test_pulsar_send_returns_receipt`.

use cave_streams::pulsar_geo_replication::{
    ClusterId, PersistentReplicator, ReplicatedMessage, ReplicationOutcome, ReplicationSender,
};
use cave_streams::pulsar_transactions::{
    TransactionBuffer, TransactionCoordinator, TxnStatus,
};
use cave_streams::schema_evolution::check_compatibility;
use cave_streams::schema_registry::CompatibilityLevel;
use cave_streams::share_group::{AcknowledgeType, SharePartition};

// ── Pulsar transactions: commit is visible, abort is not ─────────────────────

#[test]
fn pulsar_transaction_commit_publishes_abort_discards() {
    let mut tc = TransactionCoordinator::new(1);
    let mut buf = TransactionBuffer::new();

    // Txn A: two messages, committed.
    let a = tc.new_transaction(60_000, 0);
    buf.append(a, 0, b"a0".to_vec());
    buf.append(a, 1, b"a1".to_vec());
    tc.commit(&a).unwrap();
    buf.commit(&a).unwrap();

    // Txn B: one message, aborted — must never surface.
    let b = tc.new_transaction(60_000, 0);
    buf.append(b, 0, b"b0".to_vec());
    tc.abort(&b).unwrap();
    buf.abort(&b).unwrap();

    assert_eq!(tc.get_txn_meta(&a).unwrap().status, TxnStatus::Committed);
    assert_eq!(tc.get_txn_meta(&b).unwrap().status, TxnStatus::Aborted);

    let visible: Vec<_> = buf.committed().iter().map(|m| m.payload.clone()).collect();
    assert_eq!(visible, vec![b"a0".to_vec(), b"a1".to_vec()]);
    assert_eq!(buf.max_read_position(), 2);
    assert!(buf.is_aborted(&b));
}

// ── Kafka share group: acquire → acknowledge → SPSO advance ──────────────────

#[test]
fn share_group_acquire_acknowledge_advances_spso() {
    let mut p = SharePartition::new(0, 5, 30_000);
    // Log has offsets 0..3.
    let acquired = p.acquire("m1", 10, 3, 1000);
    assert_eq!(acquired, vec![0, 1, 2]);

    // Accept 0 and 1 (contiguous prefix) → SPSO advances to 2.
    p.acknowledge("m1", 0, AcknowledgeType::Accept, 1100).unwrap();
    p.acknowledge("m1", 1, AcknowledgeType::Accept, 1100).unwrap();
    assert_eq!(p.start_offset(), 2);

    // Reject 2 → archived, prefix slides to 3.
    p.acknowledge("m1", 2, AcknowledgeType::Reject, 1100).unwrap();
    assert_eq!(p.start_offset(), 3);
    assert_eq!(p.acquired_count(), 0);
}

// ── Pulsar multi-region replication: fan-out + loop guard ─────────────────────

#[derive(Default)]
struct RecordingSender {
    delivered: Vec<(String, (u64, u64))>,
}
impl ReplicationSender for RecordingSender {
    fn send(&mut self, target: &ClusterId, msg: &ReplicatedMessage) -> ReplicationOutcome {
        self.delivered
            .push((target.as_str().to_string(), msg.message_id));
        ReplicationOutcome::Acked
    }
}

#[test]
fn multi_region_replication_fans_out_and_guards_loopback() {
    let local = ClusterId::new("us-east");
    let peers = vec![ClusterId::new("us-west"), ClusterId::new("eu-central")];
    let mut repl = PersistentReplicator::new("persistent://t/ns/topic", local.clone(), peers);

    // A locally-produced message is queued.
    let local_msg = ReplicatedMessage {
        topic: "persistent://t/ns/topic".into(),
        source_cluster: local.clone(),
        message_id: (1, 0),
        producer_name: "p1".into(),
        replicated_from: None,
        payload: b"hello".to_vec(),
    };
    assert!(repl.enqueue(local_msg));

    // A message that originated in a peer must NOT be re-replicated (loop guard).
    let foreign = ReplicatedMessage {
        topic: "persistent://t/ns/topic".into(),
        source_cluster: ClusterId::new("us-west"),
        message_id: (9, 9),
        producer_name: "p2".into(),
        replicated_from: Some(ClusterId::new("us-west")),
        payload: b"loop".to_vec(),
    };
    assert!(!repl.enqueue(foreign));

    let mut sender = RecordingSender::default();
    let shipped = repl.drain(&mut sender);
    // One message, two peers → two deliveries.
    assert_eq!(shipped, 2);
    assert_eq!(sender.delivered.len(), 2);
    assert_eq!(*repl.sent.get(&ClusterId::new("us-west")).unwrap(), 1);
    assert_eq!(*repl.sent.get(&ClusterId::new("eu-central")).unwrap(), 1);
}

// ── Schema Registry: BACKWARD compatibility ──────────────────────────────────

#[test]
fn schema_registry_backward_add_optional_field() {
    let v1 = r#"{ "properties": { "id": { "type": "long" } } }"#;
    // Adding a field WITH a default is BACKWARD-compatible.
    let v2_ok = r#"{ "properties": { "id": { "type": "long" }, "name": { "type": "string", "default": "" } } }"#;
    let ok = check_compatibility(CompatibilityLevel::Backward, v1, v2_ok).unwrap();
    assert!(ok.compatible, "add-with-default must be backward-compatible: {:?}", ok.reasons);

    // Changing a field's type is NOT compatible.
    let v2_bad = r#"{ "properties": { "id": { "type": "string" } } }"#;
    let bad = check_compatibility(CompatibilityLevel::Backward, v1, v2_bad).unwrap();
    assert!(!bad.compatible);
}
