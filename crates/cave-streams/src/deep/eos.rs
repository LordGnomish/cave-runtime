//! Read-Process-Write exactly-once-semantics validator.
//!
//! Models a Kafka Streams `read → process → write` loop:
//!
//! 1. Consumer reads a record at offset `o`.
//! 2. Processor produces zero or more output records.
//! 3. Coordinator commits both the produced records and the consumer offset
//!    `o + 1` in a **single transaction**.
//!
//! The model verifies that:
//!
//! * No record is processed twice (idempotent).
//! * If a transaction aborts, the consumer offset is **not** advanced.
//! * After a successful commit, the consumer offset is exactly
//!   `last_processed + 1`.

use std::collections::BTreeMap;

use super::txn_coordinator::{CoordinatorError, TxnCoordinator, TxnState};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum EosError {
    #[error("transaction not in Ongoing state")]
    NotOngoing,
    #[error("offset already committed: {0}")]
    AlreadyCommitted(i64),
    #[error("coordinator error: {0}")]
    Coordinator(#[from] CoordinatorError),
}

#[derive(Debug, Default)]
pub struct EosSession {
    /// Per-(group_id, topic-partition) committed offset (last_offset + 1).
    pub committed_offsets: BTreeMap<(String, String, i32), i64>,
    /// Outputs from the in-flight txn (cleared on commit/abort).
    pub pending_outputs: Vec<Output>,
    /// Consumer offsets the in-flight txn would commit.
    pub pending_offsets: BTreeMap<(String, String, i32), i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub topic: String,
    pub partition: i32,
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

impl EosSession {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn process(
        &mut self,
        coordinator: &TxnCoordinator,
        transactional_id: &str,
        group_id: &str,
        consumer_topic: &str,
        consumer_partition: i32,
        consumer_offset: i64,
        outputs: Vec<Output>,
    ) -> Result<(), EosError> {
        let m = coordinator
            .metadata(transactional_id)
            .ok_or_else(|| EosError::Coordinator(CoordinatorError::UnknownTxnId(transactional_id.into())))?;
        if m.state != TxnState::Ongoing {
            return Err(EosError::NotOngoing);
        }
        let key = (group_id.to_string(), consumer_topic.to_string(), consumer_partition);
        if let Some(committed) = self.committed_offsets.get(&key) {
            if consumer_offset < *committed {
                return Err(EosError::AlreadyCommitted(consumer_offset));
            }
        }
        self.pending_outputs.extend(outputs);
        self.pending_offsets.insert(key, consumer_offset + 1);
        Ok(())
    }

    /// Commit the in-flight outputs and consumer offsets atomically.
    pub fn commit(&mut self, coordinator: &mut TxnCoordinator, transactional_id: &str, epoch: i16) -> Result<(), EosError> {
        coordinator.prepare_commit(transactional_id, epoch)?;
        coordinator.complete_commit(transactional_id)?;
        self.committed_offsets.extend(std::mem::take(&mut self.pending_offsets));
        self.pending_outputs.clear();
        Ok(())
    }

    pub fn abort(&mut self, coordinator: &mut TxnCoordinator, transactional_id: &str, epoch: i16) -> Result<(), EosError> {
        coordinator.prepare_abort(transactional_id, epoch)?;
        coordinator.complete_abort(transactional_id)?;
        self.pending_outputs.clear();
        self.pending_offsets.clear();
        Ok(())
    }

    pub fn last_committed(
        &self,
        group_id: &str,
        topic: &str,
        partition: i32,
    ) -> Option<i64> {
        self.committed_offsets
            .get(&(group_id.into(), topic.into(), partition))
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deep::txn_coordinator::TxnCoordinator;

    fn output(topic: &str, partition: i32, k: &[u8], v: &[u8]) -> Output {
        Output {
            topic: topic.into(),
            partition,
            key: k.to_vec(),
            value: v.to_vec(),
        }
    }

    fn setup() -> (TxnCoordinator, EosSession, &'static str) {
        let mut c = TxnCoordinator::new();
        c.init_producer_id("eos-app-1", 60_000).unwrap();
        c.add_partitions("eos-app-1", 0, &[("output".into(), 0)]).unwrap();
        (c, EosSession::new(), "eos-app-1")
    }

    #[test]
    fn process_then_commit_advances_offset() {
        let (mut c, mut s, tid) = setup();
        s.process(&c, tid, "grp", "input", 0, 100, vec![output("output", 0, b"k", b"v")]).unwrap();
        s.commit(&mut c, tid, 0).unwrap();
        assert_eq!(s.last_committed("grp", "input", 0), Some(101));
    }

    #[test]
    fn abort_does_not_advance_offset() {
        let (mut c, mut s, tid) = setup();
        s.process(&c, tid, "grp", "input", 0, 100, vec![]).unwrap();
        s.abort(&mut c, tid, 0).unwrap();
        assert_eq!(s.last_committed("grp", "input", 0), None);
    }

    #[test]
    fn process_outside_ongoing_errors() {
        let mut c = TxnCoordinator::new();
        c.init_producer_id("t1", 60_000).unwrap();
        // No add_partitions → state is still Empty.
        let mut s = EosSession::new();
        let err = s.process(&c, "t1", "grp", "input", 0, 0, vec![]).unwrap_err();
        assert_eq!(err, EosError::NotOngoing);
    }

    #[test]
    fn double_process_same_offset_is_rejected_after_commit() {
        let (mut c, mut s, tid) = setup();
        s.process(&c, tid, "grp", "input", 0, 100, vec![]).unwrap();
        s.commit(&mut c, tid, 0).unwrap();
        // Start a new txn cycle.
        c.reset_to_empty(tid).unwrap();
        c.add_partitions(tid, 0, &[("output".into(), 0)]).unwrap();
        let err = s
            .process(&c, tid, "grp", "input", 0, 99, vec![])
            .unwrap_err();
        assert!(matches!(err, EosError::AlreadyCommitted(99)));
    }

    #[test]
    fn pending_outputs_held_until_commit() {
        let (mut c, mut s, tid) = setup();
        s.process(&c, tid, "grp", "input", 0, 100, vec![output("output", 0, b"a", b"b")]).unwrap();
        assert_eq!(s.pending_outputs.len(), 1);
        s.commit(&mut c, tid, 0).unwrap();
        assert!(s.pending_outputs.is_empty());
    }

    #[test]
    fn pending_offsets_dropped_on_abort() {
        let (mut c, mut s, tid) = setup();
        s.process(&c, tid, "grp", "input", 0, 100, vec![]).unwrap();
        assert!(!s.pending_offsets.is_empty());
        s.abort(&mut c, tid, 0).unwrap();
        assert!(s.pending_offsets.is_empty());
    }

    #[test]
    fn process_multiple_partitions_in_same_txn() {
        let mut c = TxnCoordinator::new();
        c.init_producer_id("t", 60_000).unwrap();
        c.add_partitions("t", 0, &[("o".into(), 0), ("o".into(), 1)]).unwrap();
        let mut s = EosSession::new();
        s.process(&c, "t", "g", "i", 0, 100, vec![]).unwrap();
        s.process(&c, "t", "g", "i", 1, 200, vec![]).unwrap();
        s.commit(&mut c, "t", 0).unwrap();
        assert_eq!(s.last_committed("g", "i", 0), Some(101));
        assert_eq!(s.last_committed("g", "i", 1), Some(201));
    }

    #[test]
    fn commit_clears_pending() {
        let (mut c, mut s, tid) = setup();
        s.process(&c, tid, "grp", "input", 0, 100, vec![output("output", 0, b"a", b"b")]).unwrap();
        s.commit(&mut c, tid, 0).unwrap();
        assert!(s.pending_outputs.is_empty());
        assert!(s.pending_offsets.is_empty());
    }
}
