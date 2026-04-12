//! Persistent log store backed by the WAL.

use std::path::Path;

use tracing::info;

use crate::error::HaResult;
use crate::raft::log::{LogEntry, MemLog};
use crate::raft::types::{HardState, LogIndex, Term};
use crate::storage::wal::Wal;

/// Durable log store — writes go to WAL, reads from in-memory cache.
pub struct PersistentLogStore {
    mem: MemLog,
    wal: Wal,
    hard_state: HardState,
}

impl PersistentLogStore {
    /// Open (or recover) a log store at `dir/raft.wal`.
    pub async fn open(dir: impl AsRef<Path>) -> HaResult<Self> {
        let wal_path = dir.as_ref().join("raft.wal");
        let mut wal = Wal::open(&wal_path).await?;
        let replay = wal.replay().await?;

        let mut mem = MemLog::new();
        let hard_state = replay.hard_state.unwrap_or_default();

        // Rebuild in-memory log from WAL.
        mem.append(replay.entries);
        info!(
            entries = mem.len(),
            term = hard_state.term,
            "log store recovered from WAL"
        );

        Ok(Self { mem, wal, hard_state })
    }

    // ── Write ops ─────────────────────────────────────────────────────────

    pub async fn save_hard_state(&mut self, hs: HardState) -> HaResult<()> {
        self.wal.append_hard_state(&hs).await?;
        self.hard_state = hs;
        Ok(())
    }

    pub async fn append(&mut self, entries: Vec<LogEntry>) -> HaResult<()> {
        for e in &entries {
            self.wal.append_entry(e).await?;
        }
        self.mem.append(entries);
        Ok(())
    }

    pub async fn truncate_to(&mut self, last_index: LogIndex) -> HaResult<()> {
        self.wal.append_truncate(last_index).await?;
        self.mem.truncate_to(last_index);
        Ok(())
    }

    /// Compact entries up to `index` — reset WAL and write fresh baseline.
    pub async fn compact(&mut self, index: LogIndex, term: Term) -> HaResult<()> {
        self.mem.compact(index, term);
        // Reset WAL and re-write hard state so recovery starts clean.
        self.wal.reset().await?;
        self.wal.append_hard_state(&self.hard_state).await?;
        Ok(())
    }

    // ── Read ops ──────────────────────────────────────────────────────────

    pub fn hard_state(&self) -> &HardState { &self.hard_state }
    pub fn last_index(&self) -> LogIndex { self.mem.last_index() }
    pub fn last_term(&self) -> Term { self.mem.last_term() }
    pub fn snapshot_index(&self) -> LogIndex { self.mem.snapshot_index() }
    pub fn snapshot_term(&self) -> Term { self.mem.snapshot_term() }

    pub fn entry(&self, index: LogIndex) -> HaResult<&LogEntry> {
        self.mem.entry(index)
    }

    pub fn term(&self, index: LogIndex) -> HaResult<Term> {
        self.mem.term(index)
    }

    pub fn slice(&self, lo: LogIndex, hi: LogIndex) -> HaResult<Vec<LogEntry>> {
        self.mem.slice(lo, hi)
    }

    pub fn mem(&self) -> &MemLog { &self.mem }
    pub fn wal_size(&self) -> u64 { self.wal.size() }
}
