// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Behavioral parity tests for the Apache Pulsar v4.2.0 transaction subsystem.
//!
//! Ported faithfully from:
//!   - `pulsar-client-api/.../transaction/TxnID.java`
//!   - `pulsar-transaction/common/.../proto/TxnStatus.java` + `util/TransactionUtil.java`
//!   - `pulsar-transaction/coordinator/.../TxnMeta.java` + `impl/*`
//!   - `pulsar-broker/.../transaction/buffer/impl/InMemTransactionBuffer.java`
//!   - `pulsar-broker/.../transaction/buffer/AbortedTxnProcessor.java`
//!   - `pulsar-broker/.../transaction/pendingack/PendingAckHandle.java`
//!   - `pulsar-transaction/coordinator/.../TransactionTimeoutTracker.java`
//!
//! RED until `src/pulsar_transactions.rs` lands.

use cave_streams::pulsar_transactions::{
    AbortedTxnProcessor, EntryPosition, PendingAckHandle, PendingAckPosition, TransactionBuffer,
    TransactionCoordinator, TransactionMetadataStore, TransactionTimeoutTracker, TxnBuffer,
    TxnError, TxnId, TxnMeta, TxnStatus,
};

// ── TxnId ────────────────────────────────────────────────────────────────────

#[test]
fn txn_id_display_is_paren_pair() {
    assert_eq!(TxnId::new(1, 5).to_string(), "(1,5)");
}

#[test]
fn txn_id_equality_uses_both_fields() {
    assert_eq!(TxnId::new(7, 9), TxnId::new(7, 9));
    assert_ne!(TxnId::new(7, 9), TxnId::new(8, 9));
    assert_ne!(TxnId::new(7, 9), TxnId::new(7, 10));
}

// ── TxnStatus state machine ──────────────────────────────────────────────────

#[test]
fn txn_status_open_transitions() {
    assert!(TxnStatus::can_transition_to(TxnStatus::Open, TxnStatus::Committing));
    assert!(TxnStatus::can_transition_to(TxnStatus::Open, TxnStatus::Aborting));
    assert!(TxnStatus::can_transition_to(TxnStatus::Open, TxnStatus::Open));
    assert!(!TxnStatus::can_transition_to(TxnStatus::Open, TxnStatus::Committed));
    assert!(!TxnStatus::can_transition_to(TxnStatus::Open, TxnStatus::Aborted));
}

#[test]
fn txn_status_committing_transitions() {
    assert!(TxnStatus::can_transition_to(TxnStatus::Committing, TxnStatus::Committed));
    assert!(TxnStatus::can_transition_to(TxnStatus::Committing, TxnStatus::Committing));
    assert!(!TxnStatus::can_transition_to(TxnStatus::Committing, TxnStatus::Aborting));
    assert!(!TxnStatus::can_transition_to(TxnStatus::Committing, TxnStatus::Aborted));
}

#[test]
fn txn_status_terminals_are_self_loop_only() {
    assert!(TxnStatus::can_transition_to(TxnStatus::Committed, TxnStatus::Committed));
    assert!(!TxnStatus::can_transition_to(TxnStatus::Committed, TxnStatus::Aborting));
    assert!(!TxnStatus::can_transition_to(TxnStatus::Committed, TxnStatus::Open));
    assert!(TxnStatus::can_transition_to(TxnStatus::Aborted, TxnStatus::Aborted));
    assert!(!TxnStatus::can_transition_to(TxnStatus::Aborted, TxnStatus::Committed));
}

#[test]
fn txn_status_aborting_transitions() {
    assert!(TxnStatus::can_transition_to(TxnStatus::Aborting, TxnStatus::Aborted));
    assert!(TxnStatus::can_transition_to(TxnStatus::Aborting, TxnStatus::Aborting));
    assert!(!TxnStatus::can_transition_to(TxnStatus::Aborting, TxnStatus::Committed));
}

// ── TxnMeta ──────────────────────────────────────────────────────────────────

#[test]
fn txn_meta_starts_open_and_empty() {
    let m = TxnMeta::new(TxnId::new(0, 0), 0, 1000, "owner");
    assert_eq!(m.status(), TxnStatus::Open);
    assert!(m.produced_partitions().is_empty());
    assert!(m.acked_partitions().is_empty());
}

#[test]
fn txn_meta_add_produced_partitions_dedups_and_sorts() {
    let mut m = TxnMeta::new(TxnId::new(0, 0), 0, 1000, "owner");
    m.add_produced_partitions(vec!["t-p1".into(), "t-p0".into()]).unwrap();
    m.add_produced_partitions(vec!["t-p0".into()]).unwrap(); // dup
    assert_eq!(m.produced_partitions(), vec!["t-p0".to_string(), "t-p1".to_string()]);
}

#[test]
fn txn_meta_add_partitions_gated_to_open() {
    let mut m = TxnMeta::new(TxnId::new(0, 0), 0, 1000, "owner");
    m.update_txn_status(TxnStatus::Committing, TxnStatus::Open).unwrap();
    let err = m.add_produced_partitions(vec!["t-p0".into()]).unwrap_err();
    assert!(matches!(err, TxnError::InvalidTxnStatus { .. }));
}

#[test]
fn txn_meta_compare_and_set_status() {
    let mut m = TxnMeta::new(TxnId::new(0, 0), 0, 1000, "owner");
    m.update_txn_status(TxnStatus::Committing, TxnStatus::Open).unwrap();
    m.update_txn_status(TxnStatus::Committed, TxnStatus::Committing).unwrap();
    assert_eq!(m.status(), TxnStatus::Committed);
}

#[test]
fn txn_meta_wrong_expected_status_errors() {
    let mut m = TxnMeta::new(TxnId::new(0, 0), 0, 1000, "owner");
    // current is Open; asserting expected==Committing must fail compare-and-set.
    let err = m.update_txn_status(TxnStatus::Committing, TxnStatus::Committing).unwrap_err();
    assert!(matches!(err, TxnError::InvalidTxnStatus { .. }));
    // Open cannot skip straight to a terminal.
    assert!(m.update_txn_status(TxnStatus::Committed, TxnStatus::Open).is_err());
}

// ── TransactionMetadataStore ─────────────────────────────────────────────────

#[test]
fn store_allocates_monotonic_txn_ids() {
    let mut s = TransactionMetadataStore::new(3);
    assert_eq!(s.new_transaction(1000, "o", 0), TxnId::new(3, 0));
    assert_eq!(s.new_transaction(1000, "o", 0), TxnId::new(3, 1));
    assert_eq!(s.new_transaction(1000, "o", 0), TxnId::new(3, 2));
}

#[test]
fn store_created_counter_tracks_new_transactions() {
    let mut s = TransactionMetadataStore::new(0);
    s.new_transaction(1000, "o", 0);
    s.new_transaction(1000, "o", 0);
    let st = s.stats();
    assert_eq!(st.created, 2);
    assert_eq!(st.committed, 0);
    assert_eq!(st.aborted, 0);
    assert_eq!(st.timed_out, 0);
}

// ── TransactionCoordinator ───────────────────────────────────────────────────

#[test]
fn coordinator_commit_and_abort_update_stats() {
    let mut c = TransactionCoordinator::new(0);
    let a = c.begin(1000, "o", 0);
    let b = c.begin(1000, "o", 0);
    c.commit(&a).unwrap();
    c.abort(&b).unwrap();
    let st = c.store().stats();
    assert_eq!(st.committed, 1);
    assert_eq!(st.aborted, 1);
    assert_eq!(st.created, 2);
}

#[test]
fn coordinator_cannot_abort_committed_txn() {
    let mut c = TransactionCoordinator::new(0);
    let a = c.begin(1000, "o", 0);
    c.commit(&a).unwrap();
    assert!(c.abort(&a).is_err());
}

#[test]
fn coordinator_begin_uses_coordinator_id_and_monotonic_least() {
    let mut c = TransactionCoordinator::new(42);
    let a = c.begin(1000, "o", 0);
    let b = c.begin(1000, "o", 0);
    assert_eq!(a.most_sig_bits(), 42);
    assert_eq!(b.most_sig_bits(), 42);
    assert!(b.least_sig_bits() > a.least_sig_bits());
}

#[test]
fn process_timeouts_aborts_open_and_counts_timed_out() {
    let mut c = TransactionCoordinator::new(0);
    let a = c.begin(100, "o", 0); // deadline 100
    let expired = c.process_timeouts(200);
    assert!(expired.contains(&a));
    assert_eq!(c.store().get(&a).unwrap().status(), TxnStatus::Aborted);
    assert_eq!(c.store().stats().timed_out, 1);
}

#[test]
fn process_timeouts_skips_committing_txn() {
    let mut c = TransactionCoordinator::new(0);
    let a = c.begin(100, "o", 0);
    c.store_mut().update_status(&a, TxnStatus::Committing, TxnStatus::Open).unwrap();
    c.process_timeouts(200);
    assert_eq!(c.store().get(&a).unwrap().status(), TxnStatus::Committing);
    assert_eq!(c.store().stats().timed_out, 0);
}

// ── TransactionTimeoutTracker (min-heap) ─────────────────────────────────────

#[test]
fn timeout_tracker_pops_earliest_deadline_first() {
    let mut t = TransactionTimeoutTracker::default();
    t.add_transaction(TxnId::new(0, 300), 300);
    t.add_transaction(TxnId::new(0, 100), 100);
    t.add_transaction(TxnId::new(0, 200), 200);
    let got = t.poll_expired(250);
    assert_eq!(got, vec![TxnId::new(0, 100), TxnId::new(0, 200)]);
    // 300 remains
    assert_eq!(t.poll_expired(1000), vec![TxnId::new(0, 300)]);
}

#[test]
fn timeout_tracker_uses_strict_less_than() {
    let mut t = TransactionTimeoutTracker::default();
    t.add_transaction(TxnId::new(0, 1), 100);
    assert!(t.poll_expired(100).is_empty(), "deadline == now is NOT expired");
}

// ── TransactionBuffer / TxnBuffer ────────────────────────────────────────────

#[test]
fn buffer_reads_entries_in_sequence_order() {
    let mut tb = TransactionBuffer::default();
    let id = TxnId::new(0, 0);
    let pos_a = EntryPosition { ledger_id: 1, entry_id: 10 };
    let pos_b = EntryPosition { ledger_id: 1, entry_id: 11 };
    tb.append_entry(&id, 0, pos_a.clone()).unwrap();
    tb.append_entry(&id, 1, pos_b.clone()).unwrap();
    assert_eq!(tb.read_entries(&id, 10, 0), vec![pos_a.clone(), pos_b.clone()]);
    assert_eq!(tb.read_entries(&id, 10, 1), vec![pos_b]);
}

#[test]
fn txn_buffer_last_sequence_id_is_greatest_key() {
    let mut b = TxnBuffer::new(TxnId::new(0, 0));
    b.append_entry(0, EntryPosition { ledger_id: 0, entry_id: 0 }).unwrap();
    b.append_entry(5, EntryPosition { ledger_id: 0, entry_id: 5 }).unwrap();
    b.append_entry(2, EntryPosition { ledger_id: 0, entry_id: 2 }).unwrap();
    assert_eq!(b.last_sequence_id(), Some(5));
}

#[test]
fn txn_buffer_append_gated_to_open() {
    let mut b = TxnBuffer::new(TxnId::new(0, 0));
    b.committing_txn();
    let err = b.append_entry(0, EntryPosition { ledger_id: 0, entry_id: 0 }).unwrap_err();
    assert!(matches!(err, TxnError::InvalidTxnStatus { .. }));
}

#[test]
fn txn_buffer_abort_requires_open() {
    let mut b = TxnBuffer::new(TxnId::new(0, 0));
    b.abort_txn().unwrap();
    assert_eq!(b.status(), TxnStatus::Aborted);
    assert!(b.abort_txn().is_err(), "aborting an already-aborted buffer errors");
}

// ── AbortedTxnProcessor ──────────────────────────────────────────────────────

#[test]
fn aborted_processor_tracks_and_filters() {
    let mut p = AbortedTxnProcessor::default();
    let a = TxnId::new(1, 2);
    let b = TxnId::new(1, 3);
    p.put_aborted_txn(a);
    assert!(p.check_aborted_transaction(a));
    assert!(!p.check_aborted_transaction(b));
    let kept = p.filter_aborted(vec![
        (b, EntryPosition { ledger_id: 0, entry_id: 1 }),
        (a, EntryPosition { ledger_id: 0, entry_id: 2 }),
    ]);
    assert_eq!(kept, vec![EntryPosition { ledger_id: 0, entry_id: 1 }]);
}

// ── PendingAckHandle ─────────────────────────────────────────────────────────

#[test]
fn pending_ack_dedups_by_position() {
    let mut h = PendingAckHandle::default();
    let pos = PendingAckPosition { ledger_id: 1, entry_id: 2 };
    h.individual_ack(TxnId::new(0, 0), pos.clone()).unwrap();
    let err = h.individual_ack(TxnId::new(0, 1), pos.clone()).unwrap_err();
    assert!(matches!(err, TxnError::TransactionConflict { .. }));
    assert_eq!(h.pending_count(), 1);
}

#[test]
fn pending_ack_commit_applies_and_abort_releases() {
    let mut h = PendingAckHandle::default();
    let txn = TxnId::new(0, 0);
    h.individual_ack(txn, PendingAckPosition { ledger_id: 1, entry_id: 2 }).unwrap();
    h.individual_ack(txn, PendingAckPosition { ledger_id: 1, entry_id: 3 }).unwrap();
    assert_eq!(h.pending_count(), 2);
    h.commit_txn(&txn);
    assert_eq!(h.pending_count(), 0);

    let mut h2 = PendingAckHandle::default();
    let txn2 = TxnId::new(0, 9);
    h2.individual_ack(txn2, PendingAckPosition { ledger_id: 2, entry_id: 5 }).unwrap();
    h2.abort_txn(&txn2);
    assert_eq!(h2.pending_count(), 0);
}
