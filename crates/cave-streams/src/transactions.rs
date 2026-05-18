// SPDX-License-Identifier: AGPL-3.0-or-later
//! Exactly-once semantics: idempotent producer + transaction coordinator.

use crate::error::{StreamsError, StreamsResult};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

// ── Producer state (idempotent producer) ──────────────────────────────────────

/// Per-(producer_id, partition) deduplication state.
#[derive(Debug, Clone)]
pub struct ProducerPartitionState {
    pub last_sequence: i32,
    pub epoch: i16,
}

/// State tracked per producer ID.
#[derive(Debug)]
pub struct ProducerState {
    pub producer_id: i64,
    pub epoch: i16,
    /// (topic, partition) → last sequence number seen
    pub sequences: HashMap<(String, i32), i32>,
    pub transaction_id: Option<String>,
    pub transaction_timeout_ms: i32,
}

impl ProducerState {
    pub fn new(producer_id: i64, epoch: i16, transaction_id: Option<String>, txn_timeout_ms: i32) -> Self {
        Self {
            producer_id,
            epoch,
            sequences: HashMap::new(),
            transaction_id,
            transaction_timeout_ms: txn_timeout_ms,
        }
    }

    /// Validate sequence numbers for idempotent delivery.
    pub fn check_sequence(
        &mut self,
        topic: &str,
        partition: i32,
        base_sequence: i32,
    ) -> StreamsResult<()> {
        let key = (topic.to_string(), partition);
        let expected = self.sequences.get(&key).map(|s| s + 1).unwrap_or(0);
        if base_sequence == expected || base_sequence == 0 {
            self.sequences.insert(key, base_sequence);
            Ok(())
        } else if base_sequence < expected {
            // Duplicate — client is retrying
            Ok(())
        } else {
            Err(StreamsError::DuplicateSequenceNumber {
                producer_id: self.producer_id,
                topic: topic.to_string(),
                partition,
            })
        }
    }
}

// ── Transaction state machine ─────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum TxnState {
    Empty,
    Ongoing,
    PrepareCommit,
    PrepareAbort,
    CompleteCommit,
    CompleteAbort,
    Dead,
    PrepareEpochFence,
}

#[derive(Debug)]
pub struct Transaction {
    pub transactional_id: String,
    pub producer_id: i64,
    pub producer_epoch: i16,
    pub timeout_ms: i32,
    pub state: TxnState,
    /// Partitions enrolled in this transaction
    pub partitions: HashSet<(String, i32)>,
    /// Consumer groups enrolled via AddOffsetsToTxn
    pub consumer_group_offsets: HashMap<String, HashMap<(String, i32), i64>>,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Transaction {
    pub fn new(
        transactional_id: String,
        producer_id: i64,
        producer_epoch: i16,
        timeout_ms: i32,
    ) -> Self {
        Self {
            transactional_id,
            producer_id,
            producer_epoch,
            timeout_ms,
            state: TxnState::Empty,
            partitions: HashSet::new(),
            consumer_group_offsets: HashMap::new(),
            started_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    pub fn add_partitions(&mut self, partitions: Vec<(String, i32)>) -> StreamsResult<()> {
        if self.state != TxnState::Empty && self.state != TxnState::Ongoing {
            return Err(StreamsError::InvalidTxnState(format!(
                "cannot add partitions in state {:?}",
                self.state
            )));
        }
        self.state = TxnState::Ongoing;
        for p in partitions {
            self.partitions.insert(p);
        }
        self.updated_at = Utc::now();
        Ok(())
    }

    pub fn end(&mut self, commit: bool) -> StreamsResult<()> {
        if self.state != TxnState::Ongoing {
            return Err(StreamsError::InvalidTxnState(format!(
                "cannot end transaction in state {:?}",
                self.state
            )));
        }
        self.state = if commit {
            TxnState::PrepareCommit
        } else {
            TxnState::PrepareAbort
        };
        self.updated_at = Utc::now();
        Ok(())
    }

    pub fn complete(&mut self) {
        self.state = match self.state {
            TxnState::PrepareCommit => TxnState::CompleteCommit,
            TxnState::PrepareAbort => TxnState::CompleteAbort,
            _ => TxnState::Dead,
        };
        self.updated_at = Utc::now();
    }

    pub fn add_offsets_to_txn(&mut self, group_id: String) {
        self.consumer_group_offsets.entry(group_id).or_default();
    }

    pub fn commit_offset_to_txn(
        &mut self,
        group_id: &str,
        topic: String,
        partition: i32,
        offset: i64,
    ) {
        self.consumer_group_offsets
            .entry(group_id.to_string())
            .or_default()
            .insert((topic, partition), offset);
    }
}

// ── Transaction Coordinator ───────────────────────────────────────────────────

pub struct TransactionCoordinator {
    /// transactional_id → Transaction
    transactions: DashMap<String, Transaction>,
    /// producer_id → ProducerState
    producers: DashMap<i64, ProducerState>,
}

impl TransactionCoordinator {
    pub fn new() -> Self {
        Self {
            transactions: DashMap::new(),
            producers: DashMap::new(),
        }
    }

    // ── InitProducerId ────────────────────────────────────────────────────────

    pub fn init_producer(
        &self,
        transactional_id: Option<String>,
        transaction_timeout_ms: i32,
        next_id_fn: impl Fn() -> i64,
    ) -> StreamsResult<(i64, i16)> {
        if let Some(ref txn_id) = transactional_id {
            // Transactional producer: bump epoch on re-init.
            //
            // NOTE: a previous version held a `DashMap::entry` write-guard
            // and then called `DashMap::get` on the same key — that
            // deadlocks on the same shard.  We now look up the prior txn
            // first (releasing the read guard before the insert below).
            let (producer_id, epoch) = match self.transactions.get(txn_id) {
                Some(txn) => (txn.producer_id, txn.producer_epoch + 1),
                None => (next_id_fn(), 0),
            };
            self.transactions.insert(
                txn_id.clone(),
                Transaction::new(txn_id.clone(), producer_id, epoch, transaction_timeout_ms),
            );
            let state = ProducerState::new(
                producer_id,
                epoch,
                Some(txn_id.clone()),
                transaction_timeout_ms,
            );
            self.producers.insert(producer_id, state);
            Ok((producer_id, epoch))
        } else {
            // Idempotent-only producer
            let producer_id = next_id_fn();
            let state = ProducerState::new(producer_id, 0, None, transaction_timeout_ms);
            self.producers.insert(producer_id, state);
            Ok((producer_id, 0))
        }
    }

    // ── AddPartitionsToTxn ────────────────────────────────────────────────────

    pub fn add_partitions_to_txn(
        &self,
        transactional_id: &str,
        producer_id: i64,
        producer_epoch: i16,
        partitions: Vec<(String, i32)>,
    ) -> StreamsResult<()> {
        let mut txn = self
            .transactions
            .get_mut(transactional_id)
            .ok_or_else(|| StreamsError::InvalidTxnState("transaction not initialised".into()))?;
        self.verify_producer(&txn, producer_id, producer_epoch)?;
        txn.add_partitions(partitions)
    }

    // ── EndTxn ────────────────────────────────────────────────────────────────

    pub fn end_txn(
        &self,
        transactional_id: &str,
        producer_id: i64,
        producer_epoch: i16,
        commit: bool,
    ) -> StreamsResult<()> {
        let mut txn = self
            .transactions
            .get_mut(transactional_id)
            .ok_or_else(|| StreamsError::InvalidTxnState("transaction not found".into()))?;
        self.verify_producer(&txn, producer_id, producer_epoch)?;
        txn.end(commit)?;
        txn.complete();
        Ok(())
    }

    // ── TxnOffsetCommit ───────────────────────────────────────────────────────

    pub fn txn_offset_commit(
        &self,
        transactional_id: &str,
        group_id: &str,
        producer_id: i64,
        producer_epoch: i16,
        offsets: Vec<(String, i32, i64)>,
    ) -> StreamsResult<()> {
        let mut txn = self
            .transactions
            .get_mut(transactional_id)
            .ok_or_else(|| StreamsError::InvalidTxnState("transaction not found".into()))?;
        self.verify_producer(&txn, producer_id, producer_epoch)?;
        for (topic, partition, offset) in offsets {
            txn.commit_offset_to_txn(group_id, topic, partition, offset);
        }
        Ok(())
    }

    // ── Idempotency check ─────────────────────────────────────────────────────

    pub fn check_sequence(
        &self,
        producer_id: i64,
        topic: &str,
        partition: i32,
        base_sequence: i32,
    ) -> StreamsResult<()> {
        let mut state = self
            .producers
            .get_mut(&producer_id)
            .ok_or_else(|| StreamsError::ProducerIdNotFound(producer_id))?;
        state.check_sequence(topic, partition, base_sequence)
    }

    // ── Describe transactions ─────────────────────────────────────────────────

    pub fn describe_transaction(&self, transactional_id: &str) -> StreamsResult<TxnSummary> {
        let txn = self
            .transactions
            .get(transactional_id)
            .ok_or_else(|| StreamsError::InvalidTxnState(format!("{transactional_id} not found")))?;
        Ok(TxnSummary {
            transactional_id: txn.transactional_id.clone(),
            producer_id: txn.producer_id,
            producer_epoch: txn.producer_epoch,
            state: format!("{:?}", txn.state),
            timeout_ms: txn.timeout_ms,
            partitions: txn.partitions.iter().cloned().collect(),
        })
    }

    pub fn list_transactions(&self) -> Vec<TxnSummary> {
        self.transactions
            .iter()
            .map(|e| TxnSummary {
                transactional_id: e.transactional_id.clone(),
                producer_id: e.producer_id,
                producer_epoch: e.producer_epoch,
                state: format!("{:?}", e.state),
                timeout_ms: e.timeout_ms,
                partitions: e.partitions.iter().cloned().collect(),
            })
            .collect()
    }

    fn verify_producer(
        &self,
        txn: &Transaction,
        producer_id: i64,
        producer_epoch: i16,
    ) -> StreamsResult<()> {
        if txn.producer_id != producer_id || txn.producer_epoch != producer_epoch {
            return Err(StreamsError::InvalidTxnState(format!(
                "producer fenced: expected ({}, {}), got ({}, {})",
                txn.producer_id, txn.producer_epoch, producer_id, producer_epoch
            )));
        }
        Ok(())
    }
}

// ── DTOs ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct TxnSummary {
    pub transactional_id: String,
    pub producer_id: i64,
    pub producer_epoch: i16,
    pub state: String,
    pub timeout_ms: i32,
    pub partitions: Vec<(String, i32)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI64, Ordering};

    fn coordinator() -> TransactionCoordinator {
        TransactionCoordinator::new()
    }

    #[test]
    fn init_idempotent_producer() {
        let c = coordinator();
        let counter = AtomicI64::new(1);
        let (pid, epoch) = c.init_producer(None, 60000, || counter.fetch_add(1, Ordering::SeqCst)).unwrap();
        assert!(pid >= 1);
        assert_eq!(epoch, 0);
    }

    #[test]
    fn init_transactional_producer() {
        let c = coordinator();
        let counter = AtomicI64::new(100);
        let (pid, epoch) = c
            .init_producer(Some("my-txn".into()), 60000, || counter.fetch_add(1, Ordering::SeqCst))
            .unwrap();
        assert!(pid >= 100);
        assert_eq!(epoch, 0);

        // Re-init bumps epoch
        let (pid2, epoch2) = c
            .init_producer(Some("my-txn".into()), 60000, || counter.fetch_add(1, Ordering::SeqCst))
            .unwrap();
        assert_eq!(pid, pid2);
        assert_eq!(epoch2, 1);
    }

    #[test]
    fn transaction_lifecycle() {
        let c = coordinator();
        let counter = AtomicI64::new(1);
        let (pid, epoch) = c
            .init_producer(Some("txn-1".into()), 60000, || counter.fetch_add(1, Ordering::SeqCst))
            .unwrap();

        c.add_partitions_to_txn("txn-1", pid, epoch, vec![("orders".into(), 0)]).unwrap();
        c.end_txn("txn-1", pid, epoch, true).unwrap();

        let desc = c.describe_transaction("txn-1").unwrap();
        assert_eq!(desc.state, "CompleteCommit");
    }

    #[test]
    fn sequence_check_idempotent() {
        let c = coordinator();
        let counter = AtomicI64::new(50);
        let (pid, _) = c.init_producer(None, 30000, || counter.fetch_add(1, Ordering::SeqCst)).unwrap();
        c.check_sequence(pid, "topic", 0, 0).unwrap();
        c.check_sequence(pid, "topic", 0, 1).unwrap();
        // Retry of seq 0 should also be ok (idempotent)
        c.check_sequence(pid, "topic", 0, 0).unwrap();
    }
}
