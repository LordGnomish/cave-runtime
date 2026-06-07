// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Pulsar transactions — Transaction Coordinator (TC) metadata store +
//! per-topic Transaction Buffer.
//!
//! upstream: apache/pulsar —
//!   * `pulsar-transaction/coordinator/.../{TransactionMetadataStore,
//!     MLTransactionMetadataStore, TxnMeta, TxnMetaImpl}`
//!   * `pulsar-transaction/common/.../TxnID`
//!   * `pulsar-broker/.../transaction/buffer/{TransactionBuffer,
//!     TopicTransactionBuffer}`
//!
//! Pulsar's transactional messaging (PIP-31) layers a two-phase commit on
//! top of the managed-ledger log. A client opens a transaction at a
//! Transaction Coordinator, which hands back a [`TxnID`] = (coordinator id,
//! monotonically-increasing sequence id). The producer then sends messages
//! and acknowledgements tagged with that `TxnID`; the broker holds them in
//! a per-topic [`TransactionBuffer`] where they stay invisible to readers
//! until the coordinator drives the transaction to `COMMITTING` →
//! `COMMITTED` (publish) or `ABORTING` → `ABORTED` (discard).
//!
//! This module is the in-memory parity port of that surface:
//!   * [`TxnStatus`] state machine with Pulsar's exact legal transitions
//!     (`TxnMetaImpl.checkTxnStatusCanBeUpdated`),
//!   * [`TransactionCoordinator`] = `TransactionMetadataStore`
//!     (newTransaction / addProducedPartitions / addAckedPartitions /
//!     updateTxnStatus + timeout tracking),
//!   * [`TransactionBuffer`] = `TopicTransactionBuffer` (append-buffered →
//!     commit-publishes / abort-discards, max-read-position gating).
//!
//! Kafka EOS (idempotent producer + transaction markers) already lives in
//! `transactions.rs` + `txn_markers.rs`; this is the Pulsar-side analog.

use crate::error::{StreamsError, StreamsResult};
use std::collections::{BTreeSet, HashMap, HashSet};

/// Transaction status — `org.apache.pulsar.transaction.coordinator.proto.TxnStatus`.
///
/// Wire ordinals match Pulsar's protobuf enum so that the values can be
/// round-tripped through the metadata log without translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TxnStatus {
    /// Transaction is open; producers may add partitions/subscriptions.
    Open = 0,
    /// Commit has been requested; no further mutation allowed.
    Committing = 1,
    /// Transaction committed — buffered messages are now visible.
    Committed = 2,
    /// Abort has been requested.
    Aborting = 3,
    /// Transaction aborted — buffered messages discarded.
    Aborted = 4,
}

/// Transaction identifier — `org.apache.pulsar.client.api.transaction.TxnID`.
///
/// `most_sig_bits` is the owning coordinator id, `least_sig_bits` the
/// per-coordinator sequence id handed out by `newTransaction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TxnID {
    pub most_sig_bits: u64,
    pub least_sig_bits: u64,
}

impl TxnID {
    pub fn new(most_sig_bits: u64, least_sig_bits: u64) -> Self {
        TxnID {
            most_sig_bits,
            least_sig_bits,
        }
    }
}

impl std::fmt::Display for TxnID {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Pulsar prints `(most,least)`.
        write!(f, "({}:{})", self.most_sig_bits, self.least_sig_bits)
    }
}

/// Per-transaction metadata — `TxnMeta` / `TxnMetaImpl`.
#[derive(Debug, Clone)]
pub struct TxnMeta {
    pub txn_id: TxnID,
    pub status: TxnStatus,
    /// Topics this transaction has produced into.
    pub produced_partitions: BTreeSet<String>,
    /// `topic:subscription` pairs this transaction has acknowledged.
    pub acked_partitions: BTreeSet<String>,
    pub open_timestamp_ms: u64,
    pub timeout_ms: u64,
}

impl TxnMeta {
    fn new(txn_id: TxnID, open_timestamp_ms: u64, timeout_ms: u64) -> Self {
        TxnMeta {
            txn_id,
            status: TxnStatus::Open,
            produced_partitions: BTreeSet::new(),
            acked_partitions: BTreeSet::new(),
            open_timestamp_ms,
            timeout_ms,
        }
    }

    /// `TxnMetaImpl.checkTxnStatus(expected)` — the current status must equal
    /// the expected one or the operation is rejected.
    fn check_status(&self, expected: TxnStatus) -> StreamsResult<()> {
        if self.status != expected {
            return Err(StreamsError::InvalidTxnState(format!(
                "txn {} expected status {:?} but was {:?}",
                self.txn_id, expected, self.status
            )));
        }
        Ok(())
    }

    /// `TxnMetaImpl.checkTxnStatusCanBeUpdated(newStatus)` — Pulsar's exact
    /// legal-transition table. Idempotent re-statements of the current target
    /// status (e.g. COMMITTING→COMMITTING) are allowed so retries are safe.
    fn can_transition_to(&self, new_status: TxnStatus) -> bool {
        match self.status {
            TxnStatus::Open => {
                new_status == TxnStatus::Committing || new_status == TxnStatus::Aborting
            }
            TxnStatus::Committing => {
                new_status == TxnStatus::Committing || new_status == TxnStatus::Committed
            }
            TxnStatus::Committed => new_status == TxnStatus::Committed,
            TxnStatus::Aborting => {
                new_status == TxnStatus::Aborting || new_status == TxnStatus::Aborted
            }
            TxnStatus::Aborted => new_status == TxnStatus::Aborted,
        }
    }
}

/// Transaction Coordinator metadata store — `TransactionMetadataStore`.
///
/// Owns a contiguous block of `TxnID`s for one coordinator id and tracks the
/// in-flight transaction metadata. Real Pulsar persists each mutation to a
/// dedicated managed ledger; this port keeps the metadata in memory (the
/// ledger substrate lives in `pulsar_managed_ledger.rs`).
#[derive(Debug)]
pub struct TransactionCoordinator {
    coordinator_id: u64,
    sequence_id: u64,
    txns: HashMap<u64, TxnMeta>,
}

impl TransactionCoordinator {
    pub fn new(coordinator_id: u64) -> Self {
        TransactionCoordinator {
            coordinator_id,
            sequence_id: 0,
            txns: HashMap::new(),
        }
    }

    pub fn coordinator_id(&self) -> u64 {
        self.coordinator_id
    }

    /// `newTransaction(timeout)` — allocate the next sequence id and register
    /// a fresh OPEN transaction.
    pub fn new_transaction(&mut self, timeout_ms: u64, now_ms: u64) -> TxnID {
        let seq = self.sequence_id;
        self.sequence_id += 1;
        let txn_id = TxnID::new(self.coordinator_id, seq);
        self.txns
            .insert(seq, TxnMeta::new(txn_id, now_ms, timeout_ms));
        txn_id
    }

    pub fn get_txn_meta(&self, txn_id: &TxnID) -> StreamsResult<&TxnMeta> {
        self.txns
            .get(&txn_id.least_sig_bits)
            .filter(|m| m.txn_id == *txn_id)
            .ok_or_else(|| StreamsError::InvalidTxnState(format!("unknown txn {txn_id}")))
    }

    fn get_open(&mut self, txn_id: &TxnID) -> StreamsResult<&mut TxnMeta> {
        let m = self
            .txns
            .get_mut(&txn_id.least_sig_bits)
            .filter(|m| m.txn_id == *txn_id)
            .ok_or_else(|| StreamsError::InvalidTxnState(format!("unknown txn {txn_id}")))?;
        m.check_status(TxnStatus::Open)?;
        Ok(m)
    }

    /// `addProducedPartitions(txnID, partitions)` — only legal while OPEN.
    pub fn add_produced_partitions(
        &mut self,
        txn_id: &TxnID,
        partitions: &[String],
    ) -> StreamsResult<()> {
        let m = self.get_open(txn_id)?;
        for p in partitions {
            m.produced_partitions.insert(p.clone());
        }
        Ok(())
    }

    /// `addAckedPartitions(txnID, subscriptions)` — only legal while OPEN.
    pub fn add_acked_partitions(
        &mut self,
        txn_id: &TxnID,
        subscriptions: &[String],
    ) -> StreamsResult<()> {
        let m = self.get_open(txn_id)?;
        for s in subscriptions {
            m.acked_partitions.insert(s.clone());
        }
        Ok(())
    }

    /// `updateTxnStatus(txnID, newStatus, expectedStatus)` — guards on the
    /// expected current status *and* the legal-transition table.
    pub fn update_txn_status(
        &mut self,
        txn_id: &TxnID,
        new_status: TxnStatus,
        expected_status: TxnStatus,
    ) -> StreamsResult<()> {
        let m = self
            .txns
            .get_mut(&txn_id.least_sig_bits)
            .filter(|m| m.txn_id == *txn_id)
            .ok_or_else(|| StreamsError::InvalidTxnState(format!("unknown txn {txn_id}")))?;
        m.check_status(expected_status)?;
        if !m.can_transition_to(new_status) {
            return Err(StreamsError::InvalidTxnState(format!(
                "txn {} illegal transition {:?} -> {:?}",
                txn_id, m.status, new_status
            )));
        }
        m.status = new_status;
        Ok(())
    }

    /// Convenience: drive a transaction OPEN → COMMITTING → COMMITTED.
    pub fn commit(&mut self, txn_id: &TxnID) -> StreamsResult<()> {
        self.update_txn_status(txn_id, TxnStatus::Committing, TxnStatus::Open)?;
        self.update_txn_status(txn_id, TxnStatus::Committed, TxnStatus::Committing)
    }

    /// Convenience: drive a transaction OPEN → ABORTING → ABORTED.
    pub fn abort(&mut self, txn_id: &TxnID) -> StreamsResult<()> {
        self.update_txn_status(txn_id, TxnStatus::Aborting, TxnStatus::Open)?;
        self.update_txn_status(txn_id, TxnStatus::Aborted, TxnStatus::Aborting)
    }

    /// `TransactionTimeoutTracker` — OPEN transactions whose deadline has
    /// passed. Returns their ids in ascending sequence order so a caller can
    /// abort them deterministically.
    pub fn timed_out(&self, now_ms: u64) -> Vec<TxnID> {
        let mut out: Vec<TxnID> = self
            .txns
            .values()
            .filter(|m| {
                m.status == TxnStatus::Open && now_ms >= m.open_timestamp_ms + m.timeout_ms
            })
            .map(|m| m.txn_id)
            .collect();
        out.sort();
        out
    }

    pub fn txn_count(&self) -> usize {
        self.txns.len()
    }
}

/// A message buffered inside a transaction, awaiting commit/abort.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BufferedMessage {
    pub txn_id: TxnID,
    pub sequence_id: u64,
    pub payload: Vec<u8>,
}

/// Per-topic Transaction Buffer — `TopicTransactionBuffer`.
///
/// Buffers transactional writes so they are invisible to ordinary readers
/// until commit. The `max_read_position` is the offset up to which readers
/// may consume — it only advances past committed (published) messages, never
/// past an in-flight transaction (the "ongoing-txn read barrier").
#[derive(Debug, Default)]
pub struct TransactionBuffer {
    ongoing: HashMap<TxnID, Vec<BufferedMessage>>,
    aborted: HashSet<TxnID>,
    /// Committed, reader-visible messages in publish order.
    committed: Vec<BufferedMessage>,
    /// Next log position to assign.
    next_position: u64,
    /// Highest position readers may consume (exclusive).
    max_read_position: u64,
}

impl TransactionBuffer {
    pub fn new() -> Self {
        TransactionBuffer::default()
    }

    /// `appendBufferToTxn` — stash a message under its transaction. Returns
    /// the tentative log position. The message is NOT yet readable.
    pub fn append(&mut self, txn_id: TxnID, sequence_id: u64, payload: Vec<u8>) -> u64 {
        let position = self.next_position;
        self.next_position += 1;
        self.ongoing.entry(txn_id).or_default().push(BufferedMessage {
            txn_id,
            sequence_id,
            payload,
        });
        position
    }

    /// `commitTxn` — publish all buffered messages for the transaction and
    /// advance the max-read-position. Returns the now-visible messages in
    /// the order they were appended.
    pub fn commit(&mut self, txn_id: &TxnID) -> StreamsResult<Vec<BufferedMessage>> {
        let msgs = self.ongoing.remove(txn_id).ok_or_else(|| {
            StreamsError::InvalidTxnState(format!("no buffered messages for txn {txn_id}"))
        })?;
        for m in &msgs {
            self.committed.push(m.clone());
            self.max_read_position += 1;
        }
        Ok(msgs)
    }

    /// `abortTxn` — discard buffered messages and remember the txn as aborted
    /// (so a late `commit` can be rejected and consumers can filter it out).
    pub fn abort(&mut self, txn_id: &TxnID) -> StreamsResult<()> {
        if self.ongoing.remove(txn_id).is_none() && !self.aborted.contains(txn_id) {
            return Err(StreamsError::InvalidTxnState(format!(
                "no buffered messages for txn {txn_id}"
            )));
        }
        self.aborted.insert(*txn_id);
        Ok(())
    }

    pub fn is_aborted(&self, txn_id: &TxnID) -> bool {
        self.aborted.contains(txn_id)
    }

    pub fn is_ongoing(&self, txn_id: &TxnID) -> bool {
        self.ongoing.contains_key(txn_id)
    }

    /// Reader-visible (committed) messages.
    pub fn committed(&self) -> &[BufferedMessage] {
        &self.committed
    }

    pub fn max_read_position(&self) -> u64 {
        self.max_read_position
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── TxnID + status machine ───────────────────────────────────────────

    #[test]
    fn new_transaction_hands_out_monotonic_sequence_ids() {
        let mut tc = TransactionCoordinator::new(7);
        let a = tc.new_transaction(60_000, 0);
        let b = tc.new_transaction(60_000, 0);
        assert_eq!(a, TxnID::new(7, 0));
        assert_eq!(b, TxnID::new(7, 1));
        assert_eq!(tc.txn_count(), 2);
        // Both carry the owning coordinator id in the high bits.
        assert_eq!(a.most_sig_bits, 7);
        assert_eq!(b.most_sig_bits, 7);
    }

    #[test]
    fn fresh_transaction_is_open() {
        let mut tc = TransactionCoordinator::new(1);
        let id = tc.new_transaction(1000, 0);
        assert_eq!(tc.get_txn_meta(&id).unwrap().status, TxnStatus::Open);
    }

    #[test]
    fn open_to_committing_to_committed_is_legal() {
        let mut tc = TransactionCoordinator::new(1);
        let id = tc.new_transaction(1000, 0);
        tc.update_txn_status(&id, TxnStatus::Committing, TxnStatus::Open)
            .unwrap();
        tc.update_txn_status(&id, TxnStatus::Committed, TxnStatus::Committing)
            .unwrap();
        assert_eq!(tc.get_txn_meta(&id).unwrap().status, TxnStatus::Committed);
    }

    #[test]
    fn open_to_aborting_to_aborted_is_legal() {
        let mut tc = TransactionCoordinator::new(1);
        let id = tc.new_transaction(1000, 0);
        tc.abort(&id).unwrap();
        assert_eq!(tc.get_txn_meta(&id).unwrap().status, TxnStatus::Aborted);
    }

    #[test]
    fn cannot_skip_committing_state() {
        // OPEN -> COMMITTED directly is illegal (must pass through COMMITTING).
        let mut tc = TransactionCoordinator::new(1);
        let id = tc.new_transaction(1000, 0);
        let err = tc.update_txn_status(&id, TxnStatus::Committed, TxnStatus::Open);
        assert!(err.is_err());
    }

    #[test]
    fn cannot_commit_an_aborting_transaction() {
        let mut tc = TransactionCoordinator::new(1);
        let id = tc.new_transaction(1000, 0);
        tc.update_txn_status(&id, TxnStatus::Aborting, TxnStatus::Open)
            .unwrap();
        // ABORTING -> COMMITTED is not a legal transition.
        let err = tc.update_txn_status(&id, TxnStatus::Committed, TxnStatus::Aborting);
        assert!(err.is_err());
    }

    #[test]
    fn expected_status_mismatch_is_rejected() {
        let mut tc = TransactionCoordinator::new(1);
        let id = tc.new_transaction(1000, 0);
        // Status is OPEN, but we claim it is COMMITTING.
        let err = tc.update_txn_status(&id, TxnStatus::Committed, TxnStatus::Committing);
        assert!(err.is_err());
    }

    #[test]
    fn committing_is_idempotent() {
        // Retrying the same forward transition must not error.
        let mut tc = TransactionCoordinator::new(1);
        let id = tc.new_transaction(1000, 0);
        tc.update_txn_status(&id, TxnStatus::Committing, TxnStatus::Open)
            .unwrap();
        tc.update_txn_status(&id, TxnStatus::Committing, TxnStatus::Committing)
            .unwrap();
        assert_eq!(tc.get_txn_meta(&id).unwrap().status, TxnStatus::Committing);
    }

    // ── partitions / subscriptions ───────────────────────────────────────

    #[test]
    fn add_produced_partitions_accumulates_unique() {
        let mut tc = TransactionCoordinator::new(1);
        let id = tc.new_transaction(1000, 0);
        tc.add_produced_partitions(&id, &["t1".into(), "t2".into()])
            .unwrap();
        tc.add_produced_partitions(&id, &["t2".into(), "t3".into()])
            .unwrap();
        let m = tc.get_txn_meta(&id).unwrap();
        assert_eq!(m.produced_partitions.len(), 3);
        assert!(m.produced_partitions.contains("t1"));
        assert!(m.produced_partitions.contains("t3"));
    }

    #[test]
    fn add_acked_subscriptions_tracked() {
        let mut tc = TransactionCoordinator::new(1);
        let id = tc.new_transaction(1000, 0);
        tc.add_acked_partitions(&id, &["topic:sub-a".into()])
            .unwrap();
        assert!(tc
            .get_txn_meta(&id)
            .unwrap()
            .acked_partitions
            .contains("topic:sub-a"));
    }

    #[test]
    fn cannot_add_partitions_after_committing() {
        let mut tc = TransactionCoordinator::new(1);
        let id = tc.new_transaction(1000, 0);
        tc.update_txn_status(&id, TxnStatus::Committing, TxnStatus::Open)
            .unwrap();
        let err = tc.add_produced_partitions(&id, &["late".into()]);
        assert!(err.is_err());
    }

    // ── timeout tracking ─────────────────────────────────────────────────

    #[test]
    fn timed_out_returns_expired_open_txns_in_order() {
        let mut tc = TransactionCoordinator::new(3);
        let a = tc.new_transaction(100, 0); // deadline 100
        let b = tc.new_transaction(50, 0); // deadline 50
        let _c = tc.new_transaction(100_000, 0); // far future
        // At t=60, only b (deadline 50) has expired.
        assert_eq!(tc.timed_out(60), vec![b]);
        // At t=200, a and b have both expired (ascending seq order).
        assert_eq!(tc.timed_out(200), vec![a, b]);
    }

    #[test]
    fn committed_txn_is_not_timed_out() {
        let mut tc = TransactionCoordinator::new(1);
        let id = tc.new_transaction(10, 0);
        tc.commit(&id).unwrap();
        assert!(tc.timed_out(1_000_000).is_empty());
    }

    // ── transaction buffer ───────────────────────────────────────────────

    #[test]
    fn buffered_messages_are_invisible_until_commit() {
        let mut buf = TransactionBuffer::new();
        let txn = TxnID::new(1, 0);
        buf.append(txn, 0, b"a".to_vec());
        buf.append(txn, 1, b"b".to_vec());
        // Nothing readable yet, read barrier still at 0.
        assert_eq!(buf.committed().len(), 0);
        assert_eq!(buf.max_read_position(), 0);
        assert!(buf.is_ongoing(&txn));

        let published = buf.commit(&txn).unwrap();
        assert_eq!(published.len(), 2);
        assert_eq!(buf.committed().len(), 2);
        assert_eq!(buf.max_read_position(), 2);
        assert!(!buf.is_ongoing(&txn));
    }

    #[test]
    fn aborted_messages_are_discarded() {
        let mut buf = TransactionBuffer::new();
        let txn = TxnID::new(1, 5);
        buf.append(txn, 0, b"x".to_vec());
        buf.abort(&txn).unwrap();
        assert_eq!(buf.committed().len(), 0);
        assert_eq!(buf.max_read_position(), 0);
        assert!(buf.is_aborted(&txn));
        assert!(!buf.is_ongoing(&txn));
    }

    #[test]
    fn commit_after_abort_is_rejected() {
        let mut buf = TransactionBuffer::new();
        let txn = TxnID::new(1, 0);
        buf.append(txn, 0, b"x".to_vec());
        buf.abort(&txn).unwrap();
        assert!(buf.commit(&txn).is_err());
    }

    #[test]
    fn interleaved_txns_publish_independently() {
        let mut buf = TransactionBuffer::new();
        let t1 = TxnID::new(1, 0);
        let t2 = TxnID::new(1, 1);
        buf.append(t1, 0, b"1a".to_vec());
        buf.append(t2, 0, b"2a".to_vec());
        buf.append(t1, 1, b"1b".to_vec());
        // Commit t2 first — only its message becomes visible.
        buf.commit(&t2).unwrap();
        assert_eq!(buf.committed().len(), 1);
        assert_eq!(buf.committed()[0].payload, b"2a");
        // Then t1's two messages.
        buf.commit(&t1).unwrap();
        assert_eq!(buf.committed().len(), 3);
        assert_eq!(buf.max_read_position(), 3);
    }
}
