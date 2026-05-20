// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Pulsar managed-ledger — BookKeeper-style durable segmented log.
//!
//! upstream: apache/pulsar — managed-ledger/.../{ManagedLedger,
//! ManagedCursor, EntryCacheManager, LedgerHandle}
//!
//! A Managed Ledger is a sequence of *ledgers* (segments) each
//! containing immutable entries identified by `(ledger_id, entry_id)`.
//! Cursors are durable bookmarks that name the last-acknowledged
//! `(ledger_id, entry_id)` for a subscription. Pulsar's broker uses
//! BookKeeper for the actual byte store; cave-streams owns
//! `segment_log.rs` as its substrate, but the *managed-ledger surface*
//! (rolling ledgers + open cursors + retention enforcement) is
//! distinct and still useful to port.
//!
//! This module is the in-memory parity port of that surface — entry
//! append/read, cursor mark-delete, ledger rollover by entry count or
//! size threshold, and retention policy enforcement.

use std::collections::HashMap;

#[derive(Default, Debug, Clone, PartialEq, Eq, Hash)]
pub struct EntryId(pub u64, pub u64); // (ledger_id, entry_id)

impl EntryId {
    pub fn new(ledger: u64, entry: u64) -> Self {
        EntryId(ledger, entry)
    }
    pub fn ledger(&self) -> u64 {
        self.0
    }
    pub fn entry(&self) -> u64 {
        self.1
    }
}

#[derive(Debug, Clone)]
pub struct LedgerSegment {
    pub id: u64,
    pub entries: Vec<Vec<u8>>,
    pub size_bytes: u64,
    pub sealed: bool,
    pub created_at_ms: i64,
}

impl LedgerSegment {
    pub fn new(id: u64, created_at_ms: i64) -> Self {
        Self {
            id,
            entries: Vec::new(),
            size_bytes: 0,
            sealed: false,
            created_at_ms,
        }
    }
    pub fn seal(&mut self) {
        self.sealed = true;
    }
    pub fn append(&mut self, data: Vec<u8>) -> u64 {
        let entry_id = self.entries.len() as u64;
        self.size_bytes += data.len() as u64;
        self.entries.push(data);
        entry_id
    }
}

#[derive(Debug, Clone)]
pub struct RetentionPolicy {
    pub retention_size_bytes: u64,
    pub retention_time_ms: i64,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            retention_size_bytes: 100 * 1024 * 1024,
            retention_time_ms: 24 * 60 * 60 * 1000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ManagedCursor {
    pub name: String,
    /// Highest `(ledger, entry)` the subscription has acknowledged.
    pub mark_delete_position: EntryId,
    /// Individual acks ahead of the mark — for out-of-order ack windows.
    pub pending_acks: Vec<EntryId>,
}

impl ManagedCursor {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            mark_delete_position: EntryId::new(0, 0),
            pending_acks: Vec::new(),
        }
    }

    /// Move the durable mark forward to `target` if it's ahead of the
    /// current mark. Returns the new mark-delete position.
    pub fn mark_delete(&mut self, target: EntryId) -> EntryId {
        if entry_lt(&self.mark_delete_position, &target) {
            self.mark_delete_position = target.clone();
            self.pending_acks.retain(|e| entry_lt(&target, e));
        }
        self.mark_delete_position.clone()
    }

    /// Record an individual ack ahead of the mark. Collapses contiguous
    /// runs into the mark-delete when possible.
    pub fn ack_individual(&mut self, target: EntryId) {
        if !self.pending_acks.iter().any(|e| *e == target) {
            self.pending_acks.push(target);
        }
        self.pending_acks
            .sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        // Collapse run starting from the mark+1.
        loop {
            let next_id = next_after(&self.mark_delete_position);
            if let Some(pos) = self.pending_acks.iter().position(|e| *e == next_id) {
                self.mark_delete_position = self.pending_acks.remove(pos);
            } else {
                break;
            }
        }
    }
}

fn entry_lt(a: &EntryId, b: &EntryId) -> bool {
    a.0 < b.0 || (a.0 == b.0 && a.1 < b.1)
}

fn next_after(e: &EntryId) -> EntryId {
    EntryId::new(e.0, e.1 + 1)
}

#[derive(Debug, Clone)]
pub struct ManagedLedger {
    pub name: String,
    pub segments: Vec<LedgerSegment>,
    pub cursors: HashMap<String, ManagedCursor>,
    pub roll_after_entries: u64,
    pub roll_after_bytes: u64,
    pub retention: RetentionPolicy,
    next_ledger_id: u64,
    clock_ms: i64,
}

impl ManagedLedger {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            segments: vec![LedgerSegment::new(1, 0)],
            cursors: HashMap::new(),
            roll_after_entries: 1_000,
            roll_after_bytes: 8 * 1024 * 1024,
            retention: RetentionPolicy::default(),
            next_ledger_id: 2,
            clock_ms: 0,
        }
    }

    pub fn set_clock(&mut self, ms: i64) {
        self.clock_ms = ms;
    }

    pub fn open_cursor(&mut self, name: &str) -> &ManagedCursor {
        self.cursors
            .entry(name.to_string())
            .or_insert_with(|| ManagedCursor::new(name))
    }

    pub fn append(&mut self, data: Vec<u8>) -> EntryId {
        let need_roll = {
            let seg = self.segments.last().unwrap();
            seg.sealed
                || seg.entries.len() as u64 >= self.roll_after_entries
                || seg.size_bytes >= self.roll_after_bytes
        };
        if need_roll {
            self.segments.last_mut().unwrap().seal();
            self.segments
                .push(LedgerSegment::new(self.next_ledger_id, self.clock_ms));
            self.next_ledger_id += 1;
        }
        let seg = self.segments.last_mut().unwrap();
        let ledger_id = seg.id;
        let entry_id = seg.append(data);
        EntryId::new(ledger_id, entry_id)
    }

    pub fn read(&self, id: &EntryId) -> Option<&[u8]> {
        let seg = self.segments.iter().find(|s| s.id == id.ledger())?;
        seg.entries.get(id.entry() as usize).map(Vec::as_slice)
    }

    pub fn earliest_cursor_position(&self) -> EntryId {
        self.cursors
            .values()
            .map(|c| c.mark_delete_position.clone())
            .min_by(|a, b| {
                if entry_lt(a, b) {
                    std::cmp::Ordering::Less
                } else if a == b {
                    std::cmp::Ordering::Equal
                } else {
                    std::cmp::Ordering::Greater
                }
            })
            .unwrap_or_else(|| EntryId::new(0, 0))
    }

    /// Apply retention. Removes sealed segments whose ledger_id is less
    /// than every cursor's mark-delete and whose age exceeds
    /// `retention_time_ms`. Returns the number of segments deleted.
    pub fn enforce_retention(&mut self) -> usize {
        let earliest = self.earliest_cursor_position();
        let now = self.clock_ms;
        let before = self.segments.len();
        self.segments.retain(|seg| {
            if !seg.sealed {
                return true;
            }
            let age = now - seg.created_at_ms;
            let beyond_cursor = seg.id < earliest.ledger();
            let beyond_age = age >= self.retention.retention_time_ms;
            !(beyond_cursor && beyond_age)
        });
        before - self.segments.len()
    }

    pub fn segment_count(&self) -> usize {
        self.segments.len()
    }

    pub fn total_entries(&self) -> usize {
        self.segments.iter().map(|s| s.entries.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_returns_growing_entry_id() {
        let mut ml = ManagedLedger::new("t");
        let a = ml.append(vec![1, 2]);
        let b = ml.append(vec![3, 4]);
        assert_eq!(a, EntryId::new(1, 0));
        assert_eq!(b, EntryId::new(1, 1));
    }

    #[test]
    fn rolls_segment_after_entry_threshold() {
        let mut ml = ManagedLedger::new("t");
        ml.roll_after_entries = 2;
        ml.append(vec![1]);
        ml.append(vec![2]);
        let id = ml.append(vec![3]);
        assert_eq!(id, EntryId::new(2, 0));
        assert_eq!(ml.segments.len(), 2);
        assert!(ml.segments[0].sealed);
    }

    #[test]
    fn rolls_segment_after_byte_threshold() {
        let mut ml = ManagedLedger::new("t");
        ml.roll_after_bytes = 5;
        ml.append(vec![0; 4]); // 4 bytes
        ml.append(vec![0; 4]); // total 8 bytes → roll on next
        let id = ml.append(vec![0; 4]);
        assert_eq!(id, EntryId::new(2, 0));
    }

    #[test]
    fn read_returns_appended_payload() {
        let mut ml = ManagedLedger::new("t");
        let id = ml.append(vec![9, 9, 9]);
        assert_eq!(ml.read(&id), Some(&[9, 9, 9][..]));
    }

    #[test]
    fn cursor_mark_delete_advances() {
        let mut ml = ManagedLedger::new("t");
        ml.append(vec![1]);
        ml.append(vec![2]);
        let cursor = ml.open_cursor("sub-1").clone();
        assert_eq!(cursor.mark_delete_position, EntryId::new(0, 0));
        ml.cursors
            .get_mut("sub-1")
            .unwrap()
            .mark_delete(EntryId::new(1, 0));
        assert_eq!(ml.cursors["sub-1"].mark_delete_position, EntryId::new(1, 0));
    }

    #[test]
    fn cursor_mark_delete_ignores_backwards_move() {
        let mut c = ManagedCursor::new("s");
        c.mark_delete_position = EntryId::new(1, 5);
        c.mark_delete(EntryId::new(1, 2));
        assert_eq!(c.mark_delete_position, EntryId::new(1, 5));
    }

    #[test]
    fn cursor_ack_individual_collapses_contiguous_run() {
        let mut c = ManagedCursor::new("s");
        c.mark_delete_position = EntryId::new(1, 0);
        c.ack_individual(EntryId::new(1, 1));
        c.ack_individual(EntryId::new(1, 2));
        c.ack_individual(EntryId::new(1, 3));
        assert_eq!(c.mark_delete_position, EntryId::new(1, 3));
        assert!(c.pending_acks.is_empty());
    }

    #[test]
    fn cursor_ack_individual_leaves_gap_in_pending() {
        let mut c = ManagedCursor::new("s");
        c.mark_delete_position = EntryId::new(1, 0);
        c.ack_individual(EntryId::new(1, 3));
        assert_eq!(c.mark_delete_position, EntryId::new(1, 0));
        assert_eq!(c.pending_acks, vec![EntryId::new(1, 3)]);
    }

    #[test]
    fn enforce_retention_removes_aged_segments() {
        let mut ml = ManagedLedger::new("t");
        ml.roll_after_entries = 1;
        ml.retention.retention_time_ms = 1_000;
        ml.set_clock(0);
        ml.append(vec![1]);
        ml.set_clock(2_000);
        ml.append(vec![2]); // rolls; seg1 sealed at t=0
        let mut c = ManagedCursor::new("s");
        c.mark_delete_position = EntryId::new(2, 0);
        ml.cursors.insert("s".into(), c);
        let removed = ml.enforce_retention();
        assert_eq!(removed, 1);
        assert_eq!(ml.segments[0].id, 2);
    }

    #[test]
    fn enforce_retention_keeps_segments_when_cursor_lags() {
        let mut ml = ManagedLedger::new("t");
        ml.roll_after_entries = 1;
        ml.retention.retention_time_ms = 1_000;
        ml.set_clock(0);
        ml.append(vec![1]);
        ml.set_clock(2_000);
        ml.append(vec![2]);
        // No cursor advance — earliest_cursor_position returns (0,0)
        let removed = ml.enforce_retention();
        assert_eq!(removed, 0);
    }

    #[test]
    fn total_entries_counts_across_segments() {
        let mut ml = ManagedLedger::new("t");
        ml.roll_after_entries = 2;
        for _ in 0..5 {
            ml.append(vec![1]);
        }
        assert_eq!(ml.total_entries(), 5);
        assert!(ml.segments.len() >= 2);
    }
}
