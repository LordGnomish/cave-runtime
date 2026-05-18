// SPDX-License-Identifier: AGPL-3.0-or-later
//! Raft-replicated storage backend.
//!
//! Mirrors `openbao/physical/raft` (Go) plus the upstream hashicorp
//! `physical/raft.RaftBackend` shape. Where [`super::FileBackend`] and
//! [`super::InMemoryBackend`] are single-node, this backend models the
//! Raft log + state-machine apply loop so a multi-instance Vault HA
//! cluster can keep its KV state in sync.
//!
//! Architecture (deliberately decoupled from a specific Raft impl):
//!
//! * [`RaftLog`] — append-only sequence of [`LogEntry`] (Put / Delete /
//!   Noop), each tagged with a monotonic `term` and 1-based `index`.
//! * `commit_index` — highest index a quorum of peers has acked.
//! * `last_applied` — highest index the local state machine has
//!   applied. `apply_committed()` drains the `(last_applied, commit_index]`
//!   range into the inner [`super::Backend`] state machine.
//! * `current_term` + `voted_for` — persistent vote state per Raft §5.1.
//!
//! The Backend trait impl appends each `put` / `delete` to the log as
//! a new entry. In a single-node setup the caller can drive
//! `mark_committed` + `apply_committed` directly after each call. In
//! a multi-node cluster the caller bridges the log through a real
//! Raft transport (cave-cluster's raft layer): every committed log
//! index across the quorum is fed back through `mark_committed`.
//!
//! Scope cut: no transport, no peer dial / heartbeat. Those live in
//! the cluster-runtime layer (see ADR-149); this module owns the
//! state machine + log + apply pipeline.

use super::{validate_path, Backend, StorageError, InMemoryBackend};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, thiserror::Error)]
pub enum RaftStorageError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("commit index {commit} is below last_applied {applied}")]
    CommitBelowApplied { commit: u64, applied: u64 },
    #[error("log index {0} out of range (log_len={1})")]
    IndexOutOfRange(u64, u64),
    #[error("snapshot replay failed: {0}")]
    SnapshotReplay(String),
}

/// One log entry. `index` is 1-based — Raft convention.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    pub index: u64,
    pub term: u64,
    pub op: LogOp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogOp {
    /// Set `path = value`.
    Put { path: String, value: Vec<u8> },
    /// Remove `path`.
    Delete { path: String },
    /// Empty entry — leaders write one on election so followers
    /// quickly catch up the new term.
    Noop,
}

impl LogOp {
    pub const fn name(&self) -> &'static str {
        match self {
            LogOp::Put { .. } => "put",
            LogOp::Delete { .. } => "delete",
            LogOp::Noop => "noop",
        }
    }
}

/// Append-only log used by [`RaftBackend`].
#[derive(Debug, Default)]
pub struct RaftLog {
    entries: Vec<LogEntry>,
}

impl RaftLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> u64 {
        self.entries.len() as u64
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn last_index(&self) -> u64 {
        self.entries.last().map(|e| e.index).unwrap_or(0)
    }

    pub fn last_term(&self) -> u64 {
        self.entries.last().map(|e| e.term).unwrap_or(0)
    }

    pub fn append(&mut self, term: u64, op: LogOp) -> u64 {
        let index = self.last_index() + 1;
        self.entries.push(LogEntry { index, term, op });
        index
    }

    pub fn get(&self, index: u64) -> Option<&LogEntry> {
        if index == 0 || index > self.last_index() {
            return None;
        }
        self.entries.get((index - 1) as usize)
    }

    /// Truncate at `before_index`: drop every entry with `index >=
    /// before_index`. Matches Raft §5.3 conflict resolution.
    pub fn truncate(&mut self, before_index: u64) {
        if before_index == 0 {
            self.entries.clear();
            return;
        }
        self.entries.retain(|e| e.index < before_index);
    }

    pub fn entries_after(&self, after_index: u64) -> Vec<LogEntry> {
        self.entries
            .iter()
            .filter(|e| e.index > after_index)
            .cloned()
            .collect()
    }
}

/// Volatile + persistent Raft state plus a state-machine inner Backend.
pub struct RaftBackend {
    inner: Arc<dyn Backend>,
    state: RwLock<RaftState>,
    log: RwLock<RaftLog>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RaftState {
    current_term: u64,
    voted_for: Option<u64>,
    /// Highest index a quorum of peers has acked.
    commit_index: u64,
    /// Highest index the local state machine has applied.
    last_applied: u64,
    /// Optional snapshot index — entries up to and including this
    /// have been baked into the inner backend and removed from the
    /// log (compacted).
    last_snapshot_index: u64,
    last_snapshot_term: u64,
}

impl RaftBackend {
    /// Build a backend wrapping a fresh [`InMemoryBackend`] state machine.
    pub fn new() -> Self {
        Self::with_inner(Arc::new(InMemoryBackend::new()))
    }

    pub fn with_inner(inner: Arc<dyn Backend>) -> Self {
        Self {
            inner,
            state: RwLock::new(RaftState::default()),
            log: RwLock::new(RaftLog::new()),
        }
    }

    pub fn current_term(&self) -> u64 {
        self.state.read().unwrap().current_term
    }

    pub fn voted_for(&self) -> Option<u64> {
        self.state.read().unwrap().voted_for
    }

    pub fn commit_index(&self) -> u64 {
        self.state.read().unwrap().commit_index
    }

    pub fn last_applied(&self) -> u64 {
        self.state.read().unwrap().last_applied
    }

    pub fn last_snapshot_index(&self) -> u64 {
        self.state.read().unwrap().last_snapshot_index
    }

    pub fn log_len(&self) -> u64 {
        self.log.read().unwrap().len()
    }

    pub fn last_log_index(&self) -> u64 {
        self.log.read().unwrap().last_index()
    }

    /// Bump the term on observing a higher term in any RPC. Clears
    /// the voted_for slot per Raft §5.1.
    pub fn bump_term(&self, observed_term: u64) {
        let mut s = self.state.write().unwrap();
        if observed_term > s.current_term {
            s.current_term = observed_term;
            s.voted_for = None;
        }
    }

    /// Vote for `candidate_id` in the current term. Returns true iff
    /// the vote was granted (i.e. we haven't already voted this term).
    pub fn cast_vote(&self, candidate_id: u64, candidate_term: u64) -> bool {
        let mut s = self.state.write().unwrap();
        if candidate_term < s.current_term {
            return false;
        }
        if candidate_term > s.current_term {
            s.current_term = candidate_term;
            s.voted_for = None;
        }
        if s.voted_for.is_none() || s.voted_for == Some(candidate_id) {
            s.voted_for = Some(candidate_id);
            true
        } else {
            false
        }
    }

    /// Append an operation as a new log entry. Returns the new index.
    pub fn propose(&self, op: LogOp) -> u64 {
        let term = self.current_term();
        self.log.write().unwrap().append(term, op)
    }

    /// Move `commit_index` forward. Errors if the caller tries to
    /// regress it past `last_applied`.
    pub fn mark_committed(&self, new_commit: u64) -> Result<(), RaftStorageError> {
        let mut s = self.state.write().unwrap();
        if new_commit < s.last_applied {
            return Err(RaftStorageError::CommitBelowApplied {
                commit: new_commit,
                applied: s.last_applied,
            });
        }
        if new_commit > s.commit_index {
            s.commit_index = new_commit;
        }
        Ok(())
    }

    /// Drain `(last_applied, commit_index]` into the inner state
    /// machine. Returns the number of entries applied.
    pub fn apply_committed(&self) -> Result<u32, RaftStorageError> {
        let (start, end) = {
            let s = self.state.read().unwrap();
            (s.last_applied + 1, s.commit_index)
        };
        if start > end {
            return Ok(0);
        }
        let mut applied = 0u32;
        let log = self.log.read().unwrap();
        for index in start..=end {
            if let Some(entry) = log.get(index) {
                self.apply_op(&entry.op)?;
                applied += 1;
            }
        }
        drop(log);
        self.state.write().unwrap().last_applied = end;
        Ok(applied)
    }

    fn apply_op(&self, op: &LogOp) -> Result<(), RaftStorageError> {
        match op {
            LogOp::Put { path, value } => {
                self.inner.put(path, value.clone())?;
            }
            LogOp::Delete { path } => {
                self.inner.delete(path)?;
            }
            LogOp::Noop => {}
        }
        Ok(())
    }

    /// Build a snapshot of the current state machine. Caller is
    /// responsible for shipping the snapshot to a slow follower.
    pub fn take_snapshot(&self) -> Result<RaftSnapshot, RaftStorageError> {
        let s = self.state.read().unwrap();
        // Walk every key from the inner backend.
        let keys = walk_all(&*self.inner, "")?;
        let mut data = BTreeMap::new();
        for k in keys {
            if let Some(v) = self.inner.get(&k)? {
                data.insert(k, v);
            }
        }
        Ok(RaftSnapshot {
            last_included_index: s.last_applied,
            last_included_term: self
                .log
                .read()
                .unwrap()
                .get(s.last_applied)
                .map(|e| e.term)
                .unwrap_or(s.current_term),
            data,
        })
    }

    /// Install a snapshot from a leader. Wipes the state machine, then
    /// reloads from `snapshot.data`, then truncates the log so future
    /// entries start at `snapshot.last_included_index + 1`.
    pub fn install_snapshot(&self, snapshot: RaftSnapshot) -> Result<(), RaftStorageError> {
        // Wipe the inner store.
        for k in walk_all(&*self.inner, "")? {
            self.inner.delete(&k)?;
        }
        for (path, value) in &snapshot.data {
            validate_path(path)?;
            self.inner.put(path, value.clone())?;
        }
        let mut log = self.log.write().unwrap();
        // Drop any entry at-or-before the snapshot index.
        log.truncate(snapshot.last_included_index + 1);
        let mut s = self.state.write().unwrap();
        s.last_applied = s.last_applied.max(snapshot.last_included_index);
        s.commit_index = s.commit_index.max(snapshot.last_included_index);
        s.last_snapshot_index = snapshot.last_included_index;
        s.last_snapshot_term = snapshot.last_included_term;
        Ok(())
    }
}

impl Default for RaftBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for RaftBackend {
    fn get(&self, path: &str) -> Result<Option<Vec<u8>>, StorageError> {
        validate_path(path)?;
        self.inner.get(path)
    }

    fn put(&self, path: &str, value: Vec<u8>) -> Result<(), StorageError> {
        validate_path(path)?;
        // Propose into the log first; caller drives commit/apply via
        // their Raft layer. The shortcut path for single-node
        // (no peers) is to commit + apply inline so the Backend
        // trait behaves like a normal one.
        let idx = self.propose(LogOp::Put {
            path: path.to_string(),
            value,
        });
        self.mark_committed(idx)
            .map_err(|e| StorageError::Other(format!("commit: {e}")))?;
        self.apply_committed()
            .map_err(|e| StorageError::Other(format!("apply: {e}")))?;
        Ok(())
    }

    fn delete(&self, path: &str) -> Result<(), StorageError> {
        validate_path(path)?;
        let idx = self.propose(LogOp::Delete { path: path.to_string() });
        self.mark_committed(idx)
            .map_err(|e| StorageError::Other(format!("commit: {e}")))?;
        self.apply_committed()
            .map_err(|e| StorageError::Other(format!("apply: {e}")))?;
        Ok(())
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>, StorageError> {
        self.inner.list(prefix)
    }
}

/// Snapshot payload — emitted by [`RaftBackend::take_snapshot`] and
/// consumed by [`RaftBackend::install_snapshot`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RaftSnapshot {
    pub last_included_index: u64,
    pub last_included_term: u64,
    pub data: BTreeMap<String, Vec<u8>>,
}

fn walk_all(backend: &dyn Backend, prefix: &str) -> Result<Vec<String>, StorageError> {
    let mut out = Vec::new();
    walk_rec(backend, prefix, &mut out)?;
    Ok(out)
}

fn walk_rec(backend: &dyn Backend, prefix: &str, out: &mut Vec<String>) -> Result<(), StorageError> {
    let children = backend.list(prefix)?;
    for child in children {
        let full = if prefix.is_empty() {
            child.trim_end_matches('/').to_string()
        } else {
            format!("{}/{}", prefix.trim_end_matches('/'), child.trim_end_matches('/'))
        };
        if child.ends_with('/') {
            walk_rec(backend, &full, out)?;
        } else {
            out.push(full);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_backend_has_zero_state() {
        let b = RaftBackend::new();
        assert_eq!(b.current_term(), 0);
        assert_eq!(b.commit_index(), 0);
        assert_eq!(b.last_applied(), 0);
        assert_eq!(b.log_len(), 0);
    }

    #[test]
    fn put_advances_log_commit_and_apply() {
        let b = RaftBackend::new();
        b.bump_term(1);
        b.put("kv/x", b"v1".to_vec()).unwrap();
        assert_eq!(b.log_len(), 1);
        assert_eq!(b.commit_index(), 1);
        assert_eq!(b.last_applied(), 1);
        assert_eq!(b.get("kv/x").unwrap(), Some(b"v1".to_vec()));
    }

    #[test]
    fn delete_is_logged_and_applied() {
        let b = RaftBackend::new();
        b.bump_term(1);
        b.put("kv/x", b"v1".to_vec()).unwrap();
        b.delete("kv/x").unwrap();
        assert_eq!(b.log_len(), 2);
        assert!(b.get("kv/x").unwrap().is_none());
    }

    #[test]
    fn invalid_path_blocked_before_log_append() {
        let b = RaftBackend::new();
        let err = b.put("/abs", b"".to_vec()).unwrap_err();
        assert!(matches!(err, StorageError::InvalidPath(_)));
        // No log entry was appended.
        assert_eq!(b.log_len(), 0);
    }

    #[test]
    fn list_delegates_to_inner() {
        let b = RaftBackend::new();
        b.bump_term(1);
        b.put("kv/a", b"1".to_vec()).unwrap();
        b.put("kv/b", b"2".to_vec()).unwrap();
        let l = b.list("kv").unwrap();
        assert_eq!(l, vec!["a", "b"]);
    }

    #[test]
    fn bump_term_clears_voted_for() {
        let b = RaftBackend::new();
        assert!(b.cast_vote(7, 1));
        assert_eq!(b.voted_for(), Some(7));
        b.bump_term(2);
        assert!(b.voted_for().is_none());
    }

    #[test]
    fn cast_vote_rejects_older_term() {
        let b = RaftBackend::new();
        b.bump_term(5);
        assert!(!b.cast_vote(1, 3));
    }

    #[test]
    fn cast_vote_one_vote_per_term() {
        let b = RaftBackend::new();
        b.bump_term(1);
        assert!(b.cast_vote(1, 1));
        assert!(!b.cast_vote(2, 1)); // already voted for candidate 1
        assert!(b.cast_vote(1, 1)); // idempotent for same candidate
    }

    #[test]
    fn manual_propose_then_commit_then_apply() {
        let b = RaftBackend::new();
        b.bump_term(3);
        // Propose two entries without committing.
        let i1 = b.propose(LogOp::Put { path: "k/a".into(), value: b"1".to_vec() });
        let i2 = b.propose(LogOp::Put { path: "k/b".into(), value: b"2".to_vec() });
        assert_eq!((i1, i2), (1, 2));
        assert_eq!(b.last_applied(), 0);
        // Commit one. Apply that one.
        b.mark_committed(1).unwrap();
        assert_eq!(b.apply_committed().unwrap(), 1);
        assert_eq!(b.get("k/a").unwrap(), Some(b"1".to_vec()));
        assert!(b.get("k/b").unwrap().is_none());
        // Commit + apply the second.
        b.mark_committed(2).unwrap();
        assert_eq!(b.apply_committed().unwrap(), 1);
        assert_eq!(b.get("k/b").unwrap(), Some(b"2".to_vec()));
    }

    #[test]
    fn mark_committed_cannot_regress_below_applied() {
        let b = RaftBackend::new();
        b.bump_term(1);
        b.put("kv/x", b"v".to_vec()).unwrap(); // commits + applies index 1
        assert!(matches!(
            b.mark_committed(0).unwrap_err(),
            RaftStorageError::CommitBelowApplied { commit: 0, applied: 1 }
        ));
    }

    #[test]
    fn apply_is_noop_when_already_caught_up() {
        let b = RaftBackend::new();
        b.bump_term(1);
        b.put("kv/x", b"v".to_vec()).unwrap();
        assert_eq!(b.apply_committed().unwrap(), 0);
    }

    #[test]
    fn log_truncate_drops_conflicting_entries() {
        let b = RaftBackend::new();
        b.bump_term(1);
        b.propose(LogOp::Put { path: "k/1".into(), value: b"v".to_vec() });
        b.propose(LogOp::Put { path: "k/2".into(), value: b"v".to_vec() });
        b.propose(LogOp::Put { path: "k/3".into(), value: b"v".to_vec() });
        b.log.write().unwrap().truncate(2);
        assert_eq!(b.log_len(), 1);
        assert_eq!(b.last_log_index(), 1);
    }

    #[test]
    fn snapshot_round_trips_into_fresh_backend() {
        let src = RaftBackend::new();
        src.bump_term(1);
        src.put("kv/a", b"1".to_vec()).unwrap();
        src.put("kv/b", b"2".to_vec()).unwrap();
        let snap = src.take_snapshot().unwrap();
        assert_eq!(snap.data.len(), 2);

        let dst = RaftBackend::new();
        dst.install_snapshot(snap.clone()).unwrap();
        assert_eq!(dst.get("kv/a").unwrap(), Some(b"1".to_vec()));
        assert_eq!(dst.get("kv/b").unwrap(), Some(b"2".to_vec()));
        assert_eq!(dst.last_snapshot_index(), snap.last_included_index);
    }

    #[test]
    fn snapshot_install_wipes_stale_state() {
        let b = RaftBackend::new();
        b.bump_term(1);
        b.put("kv/keep_me", b"old".to_vec()).unwrap();

        let mut data = BTreeMap::new();
        data.insert("kv/fresh".into(), b"new".to_vec());
        let snap = RaftSnapshot {
            last_included_index: 5,
            last_included_term: 2,
            data,
        };
        b.install_snapshot(snap).unwrap();
        assert!(b.get("kv/keep_me").unwrap().is_none());
        assert_eq!(b.get("kv/fresh").unwrap(), Some(b"new".to_vec()));
    }

    #[test]
    fn entries_after_returns_uncommitted_tail() {
        let b = RaftBackend::new();
        b.bump_term(1);
        b.propose(LogOp::Put { path: "k/1".into(), value: b"v".to_vec() });
        b.propose(LogOp::Put { path: "k/2".into(), value: b"v".to_vec() });
        b.propose(LogOp::Put { path: "k/3".into(), value: b"v".to_vec() });
        let tail = b.log.read().unwrap().entries_after(1);
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].index, 2);
        assert_eq!(tail[1].index, 3);
    }

    #[test]
    fn noop_entry_advances_index_without_state_change() {
        let b = RaftBackend::new();
        b.bump_term(2);
        let idx = b.propose(LogOp::Noop);
        b.mark_committed(idx).unwrap();
        assert_eq!(b.apply_committed().unwrap(), 1);
        // State machine unchanged — no keys created.
        assert!(b.list("").unwrap().is_empty());
    }

    #[test]
    fn log_get_zero_and_oob_returns_none() {
        let b = RaftBackend::new();
        b.bump_term(1);
        b.propose(LogOp::Noop);
        let log = b.log.read().unwrap();
        assert!(log.get(0).is_none());
        assert!(log.get(99).is_none());
        assert!(log.get(1).is_some());
    }

    #[test]
    fn replay_from_log_reconstructs_state_machine() {
        // Build a log on one backend, then replay it on a fresh
        // backend's apply path. Mirrors "follower joins cluster +
        // catches up via AppendEntries".
        let src = RaftBackend::new();
        src.bump_term(1);
        src.put("kv/a", b"1".to_vec()).unwrap();
        src.put("kv/b", b"2".to_vec()).unwrap();
        src.delete("kv/a").unwrap();
        src.put("kv/c", b"3".to_vec()).unwrap();

        let dst = RaftBackend::new();
        dst.bump_term(1);
        // Replay every entry into the destination's log.
        let src_log = src.log.read().unwrap().entries.clone();
        for e in src_log {
            dst.log.write().unwrap().append(e.term, e.op);
        }
        dst.mark_committed(dst.last_log_index()).unwrap();
        let applied = dst.apply_committed().unwrap();
        assert_eq!(applied as u64, dst.last_log_index());
        assert!(dst.get("kv/a").unwrap().is_none());
        assert_eq!(dst.get("kv/b").unwrap(), Some(b"2".to_vec()));
        assert_eq!(dst.get("kv/c").unwrap(), Some(b"3".to_vec()));
    }

    #[test]
    fn exists_uses_inner_state_machine() {
        let b = RaftBackend::new();
        b.bump_term(1);
        b.put("kv/x", b"v".to_vec()).unwrap();
        assert!(b.exists("kv/x").unwrap());
        assert!(!b.exists("kv/y").unwrap());
    }
}
