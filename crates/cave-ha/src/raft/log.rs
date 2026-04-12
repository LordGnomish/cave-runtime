use serde::{Deserialize, Serialize};
use crate::error::{HaError, HaResult};
use crate::raft::types::{EntryType, LogIndex, MembershipConfig, Term};

/// A single Raft log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub index: LogIndex,
    pub term: Term,
    pub entry_type: EntryType,
    /// Application data (serialized command for Normal; serialized MembershipConfig for Change).
    pub data: Vec<u8>,
}

impl LogEntry {
    pub fn new_normal(index: LogIndex, term: Term, data: Vec<u8>) -> Self {
        Self { index, term, entry_type: EntryType::Normal, data }
    }

    pub fn new_barrier(index: LogIndex, term: Term) -> Self {
        Self { index, term, entry_type: EntryType::Barrier, data: vec![] }
    }

    pub fn new_membership(index: LogIndex, term: Term, cfg: &MembershipConfig) -> HaResult<Self> {
        let data = serde_json::to_vec(cfg)?;
        Ok(Self { index, term, entry_type: EntryType::MembershipChange, data })
    }

    pub fn decode_membership(&self) -> HaResult<MembershipConfig> {
        serde_json::from_slice(&self.data).map_err(HaError::from)
    }
}

/// In-memory log store used by the Raft node.
/// Entries are kept from `first_index` to `last_index` inclusive.
/// Entries before the snapshot are discarded.
#[derive(Debug, Default)]
pub struct MemLog {
    /// Entries: entries[i] has index = offset + i.
    entries: Vec<LogEntry>,
    /// Index of the snapshot (entries before this are gone).
    snapshot_index: LogIndex,
    snapshot_term: Term,
}

impl MemLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Index of the last entry, or snapshot_index if log is empty.
    pub fn last_index(&self) -> LogIndex {
        if self.entries.is_empty() {
            self.snapshot_index
        } else {
            self.entries.last().unwrap().index
        }
    }

    /// Term of the last entry, or snapshot_term if empty.
    pub fn last_term(&self) -> Term {
        if self.entries.is_empty() {
            self.snapshot_term
        } else {
            self.entries.last().unwrap().term
        }
    }

    /// First stored index (one past the snapshot).
    pub fn first_index(&self) -> LogIndex {
        self.snapshot_index + 1
    }

    /// Retrieve a single entry.
    pub fn entry(&self, index: LogIndex) -> HaResult<&LogEntry> {
        if index <= self.snapshot_index {
            return Err(HaError::LogCompacted {
                requested: index,
                snapshot: self.snapshot_index,
            });
        }
        let offset = (index - self.first_index()) as usize;
        self.entries.get(offset).ok_or_else(|| {
            HaError::Raft(format!("entry {index} not found (last={})", self.last_index()))
        })
    }

    /// Term of a given index (handles snapshot boundary).
    pub fn term(&self, index: LogIndex) -> HaResult<Term> {
        if index == self.snapshot_index {
            return Ok(self.snapshot_term);
        }
        Ok(self.entry(index)?.term)
    }

    /// Return entries in range [lo, hi) — for AppendEntries RPCs.
    pub fn slice(&self, lo: LogIndex, hi: LogIndex) -> HaResult<Vec<LogEntry>> {
        if lo <= self.snapshot_index {
            return Err(HaError::LogCompacted {
                requested: lo,
                snapshot: self.snapshot_index,
            });
        }
        let start = (lo - self.first_index()) as usize;
        let end = ((hi - self.first_index()) as usize).min(self.entries.len());
        if start > end {
            return Ok(vec![]);
        }
        Ok(self.entries[start..end].to_vec())
    }

    /// Append entries, truncating any conflicting suffix first.
    pub fn append(&mut self, entries: Vec<LogEntry>) {
        for e in entries {
            let offset = (e.index - self.first_index()) as usize;
            if offset < self.entries.len() {
                // Conflict: truncate from here.
                self.entries.truncate(offset);
            }
            self.entries.push(e);
        }
    }

    /// Truncate to `last_index` (inclusive) — used for rollback.
    pub fn truncate_to(&mut self, last_index: LogIndex) {
        if last_index < self.first_index() {
            self.entries.clear();
            return;
        }
        let keep = (last_index - self.first_index() + 1) as usize;
        self.entries.truncate(keep);
    }

    /// Discard entries up to and including `index` (after snapshot).
    pub fn compact(&mut self, index: LogIndex, term: Term) {
        if index <= self.snapshot_index {
            return;
        }
        let trim = if index >= self.first_index() {
            (index - self.first_index() + 1) as usize
        } else {
            0
        };
        if trim <= self.entries.len() {
            self.entries.drain(..trim);
        } else {
            self.entries.clear();
        }
        self.snapshot_index = index;
        self.snapshot_term = term;
    }

    /// Snapshot boundary info.
    pub fn snapshot_index(&self) -> LogIndex { self.snapshot_index }
    pub fn snapshot_term(&self) -> Term { self.snapshot_term }

    /// Number of entries above the snapshot.
    pub fn len(&self) -> usize { self.entries.len() }
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }

    /// Find the most recent membership config entry (scanning backwards).
    pub fn last_membership(&self) -> Option<MembershipConfig> {
        for e in self.entries.iter().rev() {
            if e.entry_type == EntryType::MembershipChange {
                return e.decode_membership().ok();
            }
        }
        None
    }
}
