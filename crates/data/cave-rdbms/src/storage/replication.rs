// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Streaming + logical replication.
//!
//! Pure-Rust port of the PostgreSQL replication subsystem:
//!   * `src/backend/replication/slot.c` — [`ReplicationSlots`], the
//!     `restart_lsn` WAL-retention floor and forward-only slot advance;
//!   * `src/backend/replication/walsender.c` — [`StandbyFeedback`] and the
//!     write/flush/apply byte lag a standby reports back;
//!   * `src/backend/replication/logical/reorderbuffer.c` + pgoutput — the
//!     [`ReorderBuffer`] that buffers per-transaction [`Change`]s and streams
//!     them as [`DecodedChange`]s only at commit, in commit order.

use crate::storage::wal::Lsn;
use std::collections::HashMap;

/// Whether a slot feeds a physical standby or a logical decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    Physical,
    Logical,
}

/// Slot management errors (`ReplicationSlotCreate` / `ReplicationSlotDrop`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotError {
    AlreadyExists,
    NotFound,
}

#[derive(Debug, Clone)]
struct Slot {
    kind: SlotKind,
    /// oldest WAL the slot still needs — pins WAL recycling
    restart_lsn: Lsn,
    /// logical slots: LSN up to which the consumer has confirmed receipt
    confirmed_flush_lsn: Lsn,
}

/// In-memory registry of replication slots (`ReplicationSlotCtl`).
#[derive(Debug, Clone, Default)]
pub struct ReplicationSlots {
    slots: HashMap<String, Slot>,
}

impl ReplicationSlots {
    pub fn new() -> Self {
        ReplicationSlots {
            slots: HashMap::new(),
        }
    }

    /// `ReplicationSlotCreate` + `ReplicationSlotReserveWal` — create a named
    /// slot reserving WAL at `restart_lsn`. Duplicate names are rejected.
    pub fn create(&mut self, name: &str, kind: SlotKind, restart_lsn: Lsn) -> Result<(), SlotError> {
        if self.slots.contains_key(name) {
            return Err(SlotError::AlreadyExists);
        }
        self.slots.insert(
            name.to_string(),
            Slot {
                kind,
                restart_lsn,
                confirmed_flush_lsn: restart_lsn,
            },
        );
        Ok(())
    }

    /// `ReplicationSlotDrop`.
    pub fn drop(&mut self, name: &str) -> Result<(), SlotError> {
        self.slots.remove(name).map(|_| ()).ok_or(SlotError::NotFound)
    }

    /// `pg_replication_slot_advance` — move the slot's `restart_lsn`
    /// (and, for logical slots, `confirmed_flush_lsn`) forward. Targets at or
    /// below the current position are a no-op; LSNs never move backwards.
    pub fn advance(&mut self, name: &str, target: Lsn) -> Result<(), SlotError> {
        let slot = self.slots.get_mut(name).ok_or(SlotError::NotFound)?;
        if target > slot.restart_lsn {
            slot.restart_lsn = target;
        }
        if target > slot.confirmed_flush_lsn {
            slot.confirmed_flush_lsn = target;
        }
        Ok(())
    }

    pub fn restart_lsn(&self, name: &str) -> Option<Lsn> {
        self.slots.get(name).map(|s| s.restart_lsn)
    }

    pub fn confirmed_flush_lsn(&self, name: &str) -> Option<Lsn> {
        self.slots.get(name).map(|s| s.confirmed_flush_lsn)
    }

    pub fn kind(&self, name: &str) -> Option<SlotKind> {
        self.slots.get(name).map(|s| s.kind)
    }

    /// The minimum `restart_lsn` across all slots — WAL strictly older than
    /// this is safe to recycle (`ReplicationSlotsComputeRequiredLSN`). `None`
    /// when no slots exist (nothing pins WAL).
    pub fn oldest_restart_lsn(&self) -> Option<Lsn> {
        self.slots.values().map(|s| s.restart_lsn).min()
    }
}

/// Standby write/flush/apply feedback (`walsender.c` `ProcessStandbyReplyMessage`).
/// `sent_lsn` is how far the primary has shipped; the standby reports how far
/// it has persisted and replayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StandbyFeedback {
    pub sent_lsn: Lsn,
    pub write_lsn: Lsn,
    pub flush_lsn: Lsn,
    pub apply_lsn: Lsn,
}

impl StandbyFeedback {
    pub fn write_lag(&self) -> u64 {
        self.sent_lsn.saturating_sub(self.write_lsn)
    }
    pub fn flush_lag(&self) -> u64 {
        self.sent_lsn.saturating_sub(self.flush_lsn)
    }
    pub fn apply_lag(&self) -> u64 {
        self.sent_lsn.saturating_sub(self.apply_lsn)
    }
    /// The standby has durably flushed everything the primary has sent.
    pub fn is_caught_up(&self) -> bool {
        self.flush_lag() == 0
    }
}

// ── Logical decoding ─────────────────────────────────────────────────────────

/// A buffered row change before decoding (`reorderbuffer.c` `ReorderBufferChange`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Insert { relation: String, tuple: Vec<String> },
    Update { relation: String, tuple: Vec<String> },
    Delete { relation: String, key: Vec<String> },
}

impl Change {
    pub fn insert(relation: &str, tuple: Vec<String>) -> Self {
        Change::Insert {
            relation: relation.to_string(),
            tuple,
        }
    }
    pub fn update(relation: &str, tuple: Vec<String>) -> Self {
        Change::Update {
            relation: relation.to_string(),
            tuple,
        }
    }
    pub fn delete(relation: &str, key: Vec<String>) -> Self {
        Change::Delete {
            relation: relation.to_string(),
            key,
        }
    }
}

/// A decoded output-plugin message (`pgoutput` B/I/U/D/C protocol messages).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodedChange {
    Begin { xid: u32, final_lsn: Lsn },
    Insert { relation: String, tuple: Vec<String> },
    Update { relation: String, tuple: Vec<String> },
    Delete { relation: String, key: Vec<String> },
    Commit { xid: u32, commit_lsn: Lsn },
}

#[derive(Debug, Clone)]
struct Txn {
    first_lsn: Lsn,
    changes: Vec<Change>,
}

/// Reassembles interleaved WAL changes into per-transaction streams, emitted
/// only when (and in the order that) transactions commit.
#[derive(Debug, Clone, Default)]
pub struct ReorderBuffer {
    txns: HashMap<u32, Txn>,
}

impl ReorderBuffer {
    pub fn new() -> Self {
        ReorderBuffer {
            txns: HashMap::new(),
        }
    }

    /// `ReorderBufferProcessXid` — register the first WAL record seen for an xid.
    pub fn begin(&mut self, xid: u32, first_lsn: Lsn) {
        self.txns.entry(xid).or_insert(Txn {
            first_lsn,
            changes: Vec::new(),
        });
    }

    /// `ReorderBufferQueueChange` — buffer a change against its xact.
    pub fn queue_change(&mut self, xid: u32, change: Change) {
        self.txns
            .entry(xid)
            .or_insert(Txn {
                first_lsn: 0,
                changes: Vec::new(),
            })
            .changes
            .push(change);
    }

    /// `ReorderBufferCommit` — stream the xact: a BEGIN marker, its buffered
    /// changes in insertion order, then a COMMIT marker. An unknown/already
    /// drained xid yields nothing.
    pub fn commit(&mut self, xid: u32, commit_lsn: Lsn) -> Vec<DecodedChange> {
        let Some(txn) = self.txns.remove(&xid) else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(txn.changes.len() + 2);
        out.push(DecodedChange::Begin {
            xid,
            final_lsn: txn.first_lsn,
        });
        for c in txn.changes {
            out.push(match c {
                Change::Insert { relation, tuple } => DecodedChange::Insert { relation, tuple },
                Change::Update { relation, tuple } => DecodedChange::Update { relation, tuple },
                Change::Delete { relation, key } => DecodedChange::Delete { relation, key },
            });
        }
        out.push(DecodedChange::Commit { xid, commit_lsn });
        out
    }

    /// `ReorderBufferAbort` — discard the xact; nothing is decoded.
    pub fn abort(&mut self, xid: u32) -> Vec<DecodedChange> {
        self.txns.remove(&xid);
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_slots_means_nothing_pins_wal() {
        let slots = ReplicationSlots::new();
        assert_eq!(slots.oldest_restart_lsn(), None);
    }

    #[test]
    fn advance_unknown_slot_errors() {
        let mut slots = ReplicationSlots::new();
        assert_eq!(slots.advance("nope", 5), Err(SlotError::NotFound));
    }
}
