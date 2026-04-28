//! Transaction manager — BEGIN/COMMIT/ROLLBACK + isolation tracking.

use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum TxnError {
    #[error("transaction not found: {0}")]
    NotFound(TxnId),
    #[error("transaction already aborted: {0}")]
    AlreadyAborted(TxnId),
    #[error("transaction already committed: {0}")]
    AlreadyCommitted(TxnId),
    #[error("nested transactions not supported")]
    NestedTxn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TxnId(pub u64);

impl std::fmt::Display for TxnId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "txn-{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationLevel {
    ReadUncommitted,
    ReadCommitted,
    RepeatableRead,
    Serializable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxnState {
    Active,
    Committed,
    Aborted,
}

#[derive(Debug, Clone)]
pub struct Txn {
    pub id: TxnId,
    pub state: TxnState,
    pub isolation: IsolationLevel,
    pub read_only: bool,
}

#[derive(Debug, Default)]
pub struct TxnManager {
    txns: BTreeMap<TxnId, Txn>,
    next_id: u64,
}

impl TxnManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn begin(&mut self, isolation: IsolationLevel, read_only: bool) -> TxnId {
        self.next_id += 1;
        let id = TxnId(self.next_id);
        self.txns.insert(
            id,
            Txn {
                id,
                state: TxnState::Active,
                isolation,
                read_only,
            },
        );
        id
    }

    pub fn commit(&mut self, id: TxnId) -> Result<(), TxnError> {
        let t = self
            .txns
            .get_mut(&id)
            .ok_or(TxnError::NotFound(id))?;
        match t.state {
            TxnState::Aborted => return Err(TxnError::AlreadyAborted(id)),
            TxnState::Committed => return Err(TxnError::AlreadyCommitted(id)),
            TxnState::Active => t.state = TxnState::Committed,
        }
        Ok(())
    }

    pub fn abort(&mut self, id: TxnId) -> Result<(), TxnError> {
        let t = self
            .txns
            .get_mut(&id)
            .ok_or(TxnError::NotFound(id))?;
        match t.state {
            TxnState::Aborted => return Err(TxnError::AlreadyAborted(id)),
            TxnState::Committed => return Err(TxnError::AlreadyCommitted(id)),
            TxnState::Active => t.state = TxnState::Aborted,
        }
        Ok(())
    }

    pub fn state(&self, id: TxnId) -> Option<TxnState> {
        self.txns.get(&id).map(|t| t.state)
    }

    pub fn get(&self, id: TxnId) -> Option<&Txn> {
        self.txns.get(&id)
    }

    pub fn active_count(&self) -> usize {
        self.txns
            .values()
            .filter(|t| t.state == TxnState::Active)
            .count()
    }

    /// Snapshot — list of committed txn ids visible to a new transaction.
    pub fn snapshot(&self) -> Vec<TxnId> {
        self.txns
            .iter()
            .filter(|(_, t)| t.state == TxnState::Committed)
            .map(|(id, _)| *id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn begin_assigns_monotonic_ids() {
        let mut m = TxnManager::new();
        let a = m.begin(IsolationLevel::ReadCommitted, false);
        let b = m.begin(IsolationLevel::ReadCommitted, false);
        assert!(b.0 > a.0);
    }

    #[test]
    fn commit_changes_state() {
        let mut m = TxnManager::new();
        let id = m.begin(IsolationLevel::ReadCommitted, false);
        m.commit(id).unwrap();
        assert_eq!(m.state(id), Some(TxnState::Committed));
    }

    #[test]
    fn abort_changes_state() {
        let mut m = TxnManager::new();
        let id = m.begin(IsolationLevel::ReadCommitted, false);
        m.abort(id).unwrap();
        assert_eq!(m.state(id), Some(TxnState::Aborted));
    }

    #[test]
    fn double_commit_errors() {
        let mut m = TxnManager::new();
        let id = m.begin(IsolationLevel::ReadCommitted, false);
        m.commit(id).unwrap();
        let err = m.commit(id).unwrap_err();
        assert_eq!(err, TxnError::AlreadyCommitted(id));
    }

    #[test]
    fn double_abort_errors() {
        let mut m = TxnManager::new();
        let id = m.begin(IsolationLevel::ReadCommitted, false);
        m.abort(id).unwrap();
        let err = m.abort(id).unwrap_err();
        assert_eq!(err, TxnError::AlreadyAborted(id));
    }

    #[test]
    fn commit_after_abort_errors() {
        let mut m = TxnManager::new();
        let id = m.begin(IsolationLevel::ReadCommitted, false);
        m.abort(id).unwrap();
        let err = m.commit(id).unwrap_err();
        assert_eq!(err, TxnError::AlreadyAborted(id));
    }

    #[test]
    fn unknown_txn_errors() {
        let mut m = TxnManager::new();
        let err = m.commit(TxnId(99)).unwrap_err();
        assert_eq!(err, TxnError::NotFound(TxnId(99)));
    }

    #[test]
    fn active_count_reflects_state() {
        let mut m = TxnManager::new();
        m.begin(IsolationLevel::ReadCommitted, false);
        let id = m.begin(IsolationLevel::ReadCommitted, false);
        m.commit(id).unwrap();
        assert_eq!(m.active_count(), 1);
    }

    #[test]
    fn snapshot_lists_committed_only() {
        let mut m = TxnManager::new();
        let a = m.begin(IsolationLevel::ReadCommitted, false);
        let b = m.begin(IsolationLevel::ReadCommitted, false);
        m.commit(a).unwrap();
        let snap = m.snapshot();
        assert_eq!(snap, vec![a]);
        let _ = b;
    }

    #[test]
    fn isolation_levels_distinct() {
        assert_ne!(IsolationLevel::ReadUncommitted, IsolationLevel::ReadCommitted);
        assert_ne!(IsolationLevel::RepeatableRead, IsolationLevel::Serializable);
    }

    #[test]
    fn read_only_txn_marked() {
        let mut m = TxnManager::new();
        let id = m.begin(IsolationLevel::Serializable, true);
        assert!(m.get(id).unwrap().read_only);
    }

    #[test]
    fn txn_id_display() {
        let id = TxnId(42);
        assert_eq!(format!("{}", id), "txn-42");
    }
}
