//! Transactional coordinator state machine — KIP-98 §3.3.
//!
//! Each transactional id has a coordinator-side state machine:
//!
//! ```text
//! Empty  → Ongoing      (init)
//! Ongoing → PrepareCommit (commit init)
//! Ongoing → PrepareAbort  (abort init)
//! PrepareCommit → CompleteCommit
//! PrepareAbort  → CompleteAbort
//! CompleteCommit → Empty / Ongoing
//! CompleteAbort  → Empty / Ongoing
//! ```
//!
//! A transactional producer must register its `transactional_id` first,
//! receiving `(pid, epoch)` and the current state.

use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxnState {
    Empty,
    Ongoing,
    PrepareCommit,
    PrepareAbort,
    CompleteCommit,
    CompleteAbort,
    Dead,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CoordinatorError {
    #[error("invalid transition from {from:?} to {to:?}")]
    InvalidTransition { from: TxnState, to: TxnState },
    #[error("unknown transactional id: {0}")]
    UnknownTxnId(String),
    #[error("producer fenced: epoch {0} < current {1}")]
    Fenced(i16, i16),
    #[error("partitions added in non-Ongoing state: {0:?}")]
    NotOngoing(TxnState),
}

#[derive(Debug, Clone)]
pub struct TxnMetadata {
    pub transactional_id: String,
    pub pid: i64,
    pub epoch: i16,
    pub state: TxnState,
    /// (topic, partition) participants in the current transaction.
    pub partitions: BTreeSet<(String, i32)>,
    pub timeout_ms: i64,
}

#[derive(Debug, Default)]
pub struct TxnCoordinator {
    txns: BTreeMap<String, TxnMetadata>,
    next_pid: i64,
}

impl TxnCoordinator {
    pub fn new() -> Self {
        Self::default()
    }

    /// `init_producer_id` request: returns `(pid, epoch)`. Bumps epoch on
    /// re-registration to fence stale producers.
    pub fn init_producer_id(
        &mut self,
        transactional_id: &str,
        timeout_ms: i64,
    ) -> Result<(i64, i16), CoordinatorError> {
        let entry = self.txns.entry(transactional_id.to_string());
        match entry {
            std::collections::btree_map::Entry::Vacant(v) => {
                self.next_pid += 1;
                let pid = self.next_pid;
                v.insert(TxnMetadata {
                    transactional_id: transactional_id.into(),
                    pid,
                    epoch: 0,
                    state: TxnState::Empty,
                    partitions: BTreeSet::new(),
                    timeout_ms,
                });
                Ok((pid, 0))
            }
            std::collections::btree_map::Entry::Occupied(mut o) => {
                let m = o.get_mut();
                m.epoch += 1;
                m.state = TxnState::Empty;
                m.partitions.clear();
                m.timeout_ms = timeout_ms;
                Ok((m.pid, m.epoch))
            }
        }
    }

    pub fn add_partitions(
        &mut self,
        transactional_id: &str,
        epoch: i16,
        partitions: &[(String, i32)],
    ) -> Result<(), CoordinatorError> {
        let m = self
            .txns
            .get_mut(transactional_id)
            .ok_or_else(|| CoordinatorError::UnknownTxnId(transactional_id.into()))?;
        if epoch < m.epoch {
            return Err(CoordinatorError::Fenced(epoch, m.epoch));
        }
        if m.state == TxnState::Empty {
            m.state = TxnState::Ongoing;
        }
        if m.state != TxnState::Ongoing {
            return Err(CoordinatorError::NotOngoing(m.state));
        }
        for p in partitions {
            m.partitions.insert(p.clone());
        }
        Ok(())
    }

    pub fn prepare_commit(&mut self, transactional_id: &str, epoch: i16) -> Result<(), CoordinatorError> {
        let m = self
            .txns
            .get_mut(transactional_id)
            .ok_or_else(|| CoordinatorError::UnknownTxnId(transactional_id.into()))?;
        if epoch < m.epoch {
            return Err(CoordinatorError::Fenced(epoch, m.epoch));
        }
        if m.state != TxnState::Ongoing {
            return Err(CoordinatorError::InvalidTransition {
                from: m.state,
                to: TxnState::PrepareCommit,
            });
        }
        m.state = TxnState::PrepareCommit;
        Ok(())
    }

    pub fn complete_commit(&mut self, transactional_id: &str) -> Result<(), CoordinatorError> {
        let m = self
            .txns
            .get_mut(transactional_id)
            .ok_or_else(|| CoordinatorError::UnknownTxnId(transactional_id.into()))?;
        if m.state != TxnState::PrepareCommit {
            return Err(CoordinatorError::InvalidTransition {
                from: m.state,
                to: TxnState::CompleteCommit,
            });
        }
        m.state = TxnState::CompleteCommit;
        m.partitions.clear();
        Ok(())
    }

    pub fn prepare_abort(&mut self, transactional_id: &str, epoch: i16) -> Result<(), CoordinatorError> {
        let m = self
            .txns
            .get_mut(transactional_id)
            .ok_or_else(|| CoordinatorError::UnknownTxnId(transactional_id.into()))?;
        if epoch < m.epoch {
            return Err(CoordinatorError::Fenced(epoch, m.epoch));
        }
        if !matches!(m.state, TxnState::Ongoing | TxnState::PrepareCommit) {
            return Err(CoordinatorError::InvalidTransition {
                from: m.state,
                to: TxnState::PrepareAbort,
            });
        }
        m.state = TxnState::PrepareAbort;
        Ok(())
    }

    pub fn complete_abort(&mut self, transactional_id: &str) -> Result<(), CoordinatorError> {
        let m = self
            .txns
            .get_mut(transactional_id)
            .ok_or_else(|| CoordinatorError::UnknownTxnId(transactional_id.into()))?;
        if m.state != TxnState::PrepareAbort {
            return Err(CoordinatorError::InvalidTransition {
                from: m.state,
                to: TxnState::CompleteAbort,
            });
        }
        m.state = TxnState::CompleteAbort;
        m.partitions.clear();
        Ok(())
    }

    /// Once a Complete state is reached, the next AddPartitions begins a new
    /// transaction. This is what makes the coordinator durable across many
    /// txns under the same transactional id.
    pub fn reset_to_empty(&mut self, transactional_id: &str) -> Result<(), CoordinatorError> {
        let m = self
            .txns
            .get_mut(transactional_id)
            .ok_or_else(|| CoordinatorError::UnknownTxnId(transactional_id.into()))?;
        if !matches!(m.state, TxnState::CompleteCommit | TxnState::CompleteAbort) {
            return Err(CoordinatorError::InvalidTransition {
                from: m.state,
                to: TxnState::Empty,
            });
        }
        m.state = TxnState::Empty;
        Ok(())
    }

    pub fn metadata(&self, transactional_id: &str) -> Option<&TxnMetadata> {
        self.txns.get(transactional_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_producer_id_assigns_pid() {
        let mut c = TxnCoordinator::new();
        let (pid, epoch) = c.init_producer_id("t1", 60_000).unwrap();
        assert_eq!(epoch, 0);
        assert!(pid > 0);
    }

    #[test]
    fn re_init_increments_epoch() {
        let mut c = TxnCoordinator::new();
        let (pid1, _) = c.init_producer_id("t1", 60_000).unwrap();
        let (pid2, epoch) = c.init_producer_id("t1", 60_000).unwrap();
        assert_eq!(pid1, pid2);
        assert_eq!(epoch, 1);
    }

    #[test]
    fn add_partitions_moves_to_ongoing() {
        let mut c = TxnCoordinator::new();
        c.init_producer_id("t1", 60_000).unwrap();
        c.add_partitions("t1", 0, &[("topic".into(), 0)]).unwrap();
        assert_eq!(c.metadata("t1").unwrap().state, TxnState::Ongoing);
    }

    #[test]
    fn add_partitions_fenced_with_old_epoch() {
        let mut c = TxnCoordinator::new();
        c.init_producer_id("t1", 60_000).unwrap();
        c.init_producer_id("t1", 60_000).unwrap();
        let err = c.add_partitions("t1", 0, &[]).unwrap_err();
        assert!(matches!(err, CoordinatorError::Fenced(0, 1)));
    }

    #[test]
    fn full_commit_path() {
        let mut c = TxnCoordinator::new();
        c.init_producer_id("t1", 60_000).unwrap();
        c.add_partitions("t1", 0, &[("t".into(), 0)]).unwrap();
        c.prepare_commit("t1", 0).unwrap();
        c.complete_commit("t1").unwrap();
        assert_eq!(c.metadata("t1").unwrap().state, TxnState::CompleteCommit);
        assert!(c.metadata("t1").unwrap().partitions.is_empty());
    }

    #[test]
    fn full_abort_path() {
        let mut c = TxnCoordinator::new();
        c.init_producer_id("t1", 60_000).unwrap();
        c.add_partitions("t1", 0, &[("t".into(), 0)]).unwrap();
        c.prepare_abort("t1", 0).unwrap();
        c.complete_abort("t1").unwrap();
        assert_eq!(c.metadata("t1").unwrap().state, TxnState::CompleteAbort);
    }

    #[test]
    fn prepare_commit_in_empty_errors() {
        let mut c = TxnCoordinator::new();
        c.init_producer_id("t1", 60_000).unwrap();
        let err = c.prepare_commit("t1", 0).unwrap_err();
        assert!(matches!(err, CoordinatorError::InvalidTransition { .. }));
    }

    #[test]
    fn complete_commit_without_prepare_errors() {
        let mut c = TxnCoordinator::new();
        c.init_producer_id("t1", 60_000).unwrap();
        c.add_partitions("t1", 0, &[("t".into(), 0)]).unwrap();
        let err = c.complete_commit("t1").unwrap_err();
        assert!(matches!(err, CoordinatorError::InvalidTransition { .. }));
    }

    #[test]
    fn abort_during_prepare_commit_allowed() {
        let mut c = TxnCoordinator::new();
        c.init_producer_id("t1", 60_000).unwrap();
        c.add_partitions("t1", 0, &[("t".into(), 0)]).unwrap();
        c.prepare_commit("t1", 0).unwrap();
        c.prepare_abort("t1", 0).unwrap();
        assert_eq!(c.metadata("t1").unwrap().state, TxnState::PrepareAbort);
    }

    #[test]
    fn reset_to_empty_after_commit() {
        let mut c = TxnCoordinator::new();
        c.init_producer_id("t1", 60_000).unwrap();
        c.add_partitions("t1", 0, &[("t".into(), 0)]).unwrap();
        c.prepare_commit("t1", 0).unwrap();
        c.complete_commit("t1").unwrap();
        c.reset_to_empty("t1").unwrap();
        assert_eq!(c.metadata("t1").unwrap().state, TxnState::Empty);
    }

    #[test]
    fn reset_during_ongoing_errors() {
        let mut c = TxnCoordinator::new();
        c.init_producer_id("t1", 60_000).unwrap();
        c.add_partitions("t1", 0, &[("t".into(), 0)]).unwrap();
        let err = c.reset_to_empty("t1").unwrap_err();
        assert!(matches!(err, CoordinatorError::InvalidTransition { .. }));
    }

    #[test]
    fn unknown_txn_id_errors() {
        let mut c = TxnCoordinator::new();
        let err = c.add_partitions("ghost", 0, &[]).unwrap_err();
        assert_eq!(err, CoordinatorError::UnknownTxnId("ghost".into()));
    }

    #[test]
    fn add_partitions_dedupes() {
        let mut c = TxnCoordinator::new();
        c.init_producer_id("t1", 60_000).unwrap();
        c.add_partitions("t1", 0, &[("t".into(), 0), ("t".into(), 0)]).unwrap();
        assert_eq!(c.metadata("t1").unwrap().partitions.len(), 1);
    }
}
