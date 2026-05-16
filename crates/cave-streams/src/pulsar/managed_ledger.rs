// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
// Source: apache/pulsar@1940aebc6ade10050399cd65f870353eedf80008
//   managed-ledger/src/main/java/org/apache/bookkeeper/mledger/ManagedLedger.java
//   managed-ledger/src/main/java/org/apache/bookkeeper/mledger/impl/ManagedLedgerImpl.java
//   managed-ledger/src/main/java/org/apache/bookkeeper/mledger/Position.java

//! `ManagedLedger` — a chain of fixed-roll BookKeeper ledgers presented
//! as a single append-only log addressable by `(ledger_id, entry_id)`.
//!
//! This is the storage primitive Pulsar's PersistentTopic sits on.
//! Every `Position` is a `(ledger_id, entry_id)` pair; the manager
//! rolls a new ledger when the active one reaches the size threshold
//! and tracks a sliding list of cursors for in-progress consumers.
//!
//! cave-streams models the *semantics* — chain advancement, position
//! comparison, cursor mark-delete, trimming up to the slowest cursor.
//! The actual on-disk durability is the responsibility of the
//! underlying [`super::ledger::LedgerHandle`]s (in-process today).

use crate::error::{StreamsError, StreamsResult};
use crate::pulsar::ledger::{BookieRing, LedgerHandle, LedgerQuorum};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Mutex;

/// A position in a managed ledger — `(ledger_id, entry_id)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Position {
    pub ledger_id: u64,
    pub entry_id: u64,
}

impl Position {
    pub fn new(ledger_id: u64, entry_id: u64) -> Self {
        Self {
            ledger_id,
            entry_id,
        }
    }

    pub const fn earliest() -> Self {
        Self {
            ledger_id: 0,
            entry_id: 0,
        }
    }
}

impl Ord for Position {
    fn cmp(&self, other: &Self) -> Ordering {
        self.ledger_id
            .cmp(&other.ledger_id)
            .then(self.entry_id.cmp(&other.entry_id))
    }
}

impl PartialOrd for Position {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Configuration for [`ManagedLedger`].
#[derive(Debug, Clone)]
pub struct ManagedLedgerConfig {
    pub quorum: LedgerQuorum,
    /// Roll a new ledger when this many entries are appended to the
    /// active one (Pulsar uses `managedLedgerMaxEntriesPerLedger`).
    pub max_entries_per_ledger: u64,
}

impl Default for ManagedLedgerConfig {
    fn default() -> Self {
        Self {
            quorum: LedgerQuorum::default_3_2_2(),
            max_entries_per_ledger: 50_000,
        }
    }
}

/// A consumer cursor — tracks the highest acked position.  Read state
/// only; the ManagedLedger owns the trim policy.
#[derive(Debug, Clone)]
pub struct Cursor {
    pub name: String,
    pub mark_delete_position: Option<Position>,
}

impl Cursor {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            mark_delete_position: None,
        }
    }
}

/// Chain of ledgers presented as one log.
pub struct ManagedLedger<'r> {
    pub name: String,
    pub config: ManagedLedgerConfig,
    ring: &'r BookieRing,
    inner: Mutex<MlInner<'r>>,
}

struct MlInner<'r> {
    /// Closed ledgers in append order (entry count snapshot).
    closed: Vec<(u64, u64)>,
    /// Active ledger handle (writes go here).
    active: Option<LedgerHandle<'r>>,
    next_ledger_id: u64,
    cursors: HashMap<String, Cursor>,
}

impl<'r> ManagedLedger<'r> {
    pub fn open(
        name: impl Into<String>,
        ring: &'r BookieRing,
        config: ManagedLedgerConfig,
    ) -> StreamsResult<Self> {
        let inner = MlInner {
            closed: Vec::new(),
            active: None,
            next_ledger_id: 1,
            cursors: HashMap::new(),
        };
        let ml = Self {
            name: name.into(),
            config,
            ring,
            inner: Mutex::new(inner),
        };
        ml.ensure_active()?;
        Ok(ml)
    }

    fn ensure_active(&self) -> StreamsResult<()> {
        let mut inner = self.inner.lock().unwrap();
        if inner.active.is_none() {
            let id = inner.next_ledger_id;
            inner.next_ledger_id += 1;
            let lh = LedgerHandle::create(self.ring, id, self.config.quorum)?;
            inner.active = Some(lh);
        }
        Ok(())
    }

    /// `ManagedLedger.addEntry` — writes through to the active ledger,
    /// rolls when the active reaches `max_entries_per_ledger`.
    pub fn add_entry(&self, payload: &[u8]) -> StreamsResult<Position> {
        // 1. Append to the active ledger.
        let pos = {
            let inner = self.inner.lock().unwrap();
            let active = inner
                .active
                .as_ref()
                .expect("ensure_active ran in open()");
            let eid = active.add_entry(payload)?;
            Position::new(active.ledger_id, eid)
        };
        // 2. Roll if we hit the threshold.
        self.maybe_roll()?;
        Ok(pos)
    }

    fn maybe_roll(&self) -> StreamsResult<()> {
        let needs_roll = {
            let inner = self.inner.lock().unwrap();
            let active = inner.active.as_ref().unwrap();
            active.entries_added() >= self.config.max_entries_per_ledger
        };
        if needs_roll {
            self.roll_ledger()?;
        }
        Ok(())
    }

    /// Force a new active ledger.  Public so tests / admin can call it.
    pub fn roll_ledger(&self) -> StreamsResult<()> {
        let mut inner = self.inner.lock().unwrap();
        if let Some(active) = inner.active.take() {
            active.close();
            let count = active.entries_added();
            inner.closed.push((active.ledger_id, count));
        }
        let id = inner.next_ledger_id;
        inner.next_ledger_id += 1;
        let lh = LedgerHandle::create(self.ring, id, self.config.quorum)?;
        inner.active = Some(lh);
        Ok(())
    }

    /// Append-order list of `(ledger_id, entries_in_ledger)` for closed
    /// ledgers (the trim window can be cut here).
    pub fn closed_ledgers(&self) -> Vec<(u64, u64)> {
        self.inner.lock().unwrap().closed.clone()
    }

    /// Read `[from..=to]` — currently supports reads inside the active
    /// ledger only (cross-ledger reads will resolve closed ledgers
    /// once we wire a registry).
    pub fn read_entries(
        &self,
        from: Position,
        to: Position,
    ) -> StreamsResult<Vec<(Position, Vec<u8>)>> {
        let inner = self.inner.lock().unwrap();
        let active = inner.active.as_ref().unwrap();
        if from.ledger_id != active.ledger_id || to.ledger_id != active.ledger_id {
            return Err(StreamsError::Internal(
                "cross-ledger reads not yet supported".into(),
            ));
        }
        let raw = active.read_entries(from.entry_id, to.entry_id)?;
        Ok(raw
            .into_iter()
            .map(|(eid, p)| (Position::new(active.ledger_id, eid), p))
            .collect())
    }

    /// Register a new cursor (consumer subscription).
    pub fn open_cursor(&self, name: impl Into<String>) -> StreamsResult<()> {
        let name = name.into();
        let mut inner = self.inner.lock().unwrap();
        if inner.cursors.contains_key(&name) {
            return Err(StreamsError::Internal(format!(
                "cursor {name:?} already exists"
            )));
        }
        inner.cursors.insert(name.clone(), Cursor::new(name));
        Ok(())
    }

    /// Move a cursor's mark-delete position forward (no-op when already
    /// further ahead).
    pub fn mark_delete(&self, cursor: &str, pos: Position) -> StreamsResult<()> {
        let mut inner = self.inner.lock().unwrap();
        let c = inner
            .cursors
            .get_mut(cursor)
            .ok_or_else(|| StreamsError::Internal(format!("unknown cursor {cursor:?}")))?;
        match c.mark_delete_position {
            None => c.mark_delete_position = Some(pos),
            Some(cur) if pos > cur => c.mark_delete_position = Some(pos),
            _ => {} // monotonic — don't go backwards
        }
        Ok(())
    }

    pub fn cursor(&self, name: &str) -> Option<Cursor> {
        self.inner.lock().unwrap().cursors.get(name).cloned()
    }

    /// Slowest cursor — used by the trim job to determine which ledgers
    /// can be deleted.  Returns `None` when no cursor is positioned.
    pub fn slowest_mark_delete(&self) -> Option<Position> {
        let inner = self.inner.lock().unwrap();
        if inner.cursors.is_empty() {
            return None;
        }
        let mut min: Option<Position> = None;
        for c in inner.cursors.values() {
            match c.mark_delete_position {
                None => return Some(Position::earliest()), // unread cursor pins everything
                Some(p) => match min {
                    None => min = Some(p),
                    Some(cur) if p < cur => min = Some(p),
                    _ => {}
                },
            }
        }
        min
    }

    /// Trim closed ledgers whose entries are all below the slowest
    /// cursor's mark-delete position.  Returns the IDs trimmed.
    pub fn trim(&self) -> Vec<u64> {
        let slowest = match self.slowest_mark_delete() {
            None => return vec![],
            Some(p) => p,
        };
        let mut inner = self.inner.lock().unwrap();
        let mut trimmed = vec![];
        // Keep ledger ID when slowest cursor is still inside it.
        inner.closed.retain(|(lid, _)| {
            if *lid < slowest.ledger_id {
                trimmed.push(*lid);
                false
            } else {
                true
            }
        });
        trimmed
    }

    pub fn cursor_count(&self) -> usize {
        self.inner.lock().unwrap().cursors.len()
    }

    pub fn active_ledger_id(&self) -> u64 {
        self.inner.lock().unwrap().active.as_ref().unwrap().ledger_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small_config(max: u64) -> ManagedLedgerConfig {
        ManagedLedgerConfig {
            quorum: LedgerQuorum::default_3_2_2(),
            max_entries_per_ledger: max,
        }
    }

    #[test]
    fn test_position_orders_by_ledger_then_entry() {
        // cite: pulsar 4.2.0 PositionImpl.compareTo
        // ensemble = ml-001
        assert!(Position::new(1, 5) < Position::new(2, 0));
        assert!(Position::new(1, 0) < Position::new(1, 1));
        assert_eq!(Position::new(2, 2), Position::new(2, 2));
    }

    #[test]
    fn test_managed_ledger_open_creates_first_ledger() {
        // cite: pulsar 4.2.0 ManagedLedgerImpl#initialize
        // ensemble = ml-002
        let ring = BookieRing::with_size(3);
        let ml = ManagedLedger::open("topic-a", &ring, ManagedLedgerConfig::default()).unwrap();
        assert_eq!(ml.active_ledger_id(), 1);
        assert!(ml.closed_ledgers().is_empty());
    }

    #[test]
    fn test_managed_ledger_add_entry_returns_position_in_active_ledger() {
        // cite: pulsar 4.2.0 ManagedLedger.addEntry returns Position
        // ensemble = ml-003
        let ring = BookieRing::with_size(3);
        let ml = ManagedLedger::open("t", &ring, ManagedLedgerConfig::default()).unwrap();
        let p = ml.add_entry(b"hello").unwrap();
        assert_eq!(p, Position::new(1, 0));
        let p2 = ml.add_entry(b"world").unwrap();
        assert_eq!(p2, Position::new(1, 1));
    }

    #[test]
    fn test_managed_ledger_rolls_when_max_entries_hit() {
        // cite: pulsar 4.2.0 managedLedgerMaxEntriesPerLedger
        // ensemble = ml-004
        let ring = BookieRing::with_size(3);
        let ml = ManagedLedger::open("t", &ring, small_config(2)).unwrap();
        ml.add_entry(b"a").unwrap();
        ml.add_entry(b"b").unwrap();
        // Threshold met → new active ledger.
        let p3 = ml.add_entry(b"c").unwrap();
        assert_eq!(p3.ledger_id, 2, "rolled to ledger 2");
        let closed = ml.closed_ledgers();
        assert_eq!(closed, vec![(1, 2)]);
    }

    #[test]
    fn test_managed_ledger_read_entries_inside_active_ledger() {
        // cite: pulsar 4.2.0 ManagedCursor.readEntries within active
        // ensemble = ml-005
        let ring = BookieRing::with_size(3);
        let ml = ManagedLedger::open("t", &ring, ManagedLedgerConfig::default()).unwrap();
        for s in &[b"a", b"b", b"c"] {
            ml.add_entry(s.as_slice()).unwrap();
        }
        let got = ml
            .read_entries(Position::new(1, 0), Position::new(1, 2))
            .unwrap();
        assert_eq!(got.len(), 3);
        assert_eq!(got[1].1, b"b");
    }

    #[test]
    fn test_managed_ledger_open_cursor_duplicate_errors() {
        // cite: pulsar 4.2.0 ManagedLedger.openCursor (existing returns same; cave-streams strict)
        // ensemble = ml-006
        let ring = BookieRing::with_size(3);
        let ml = ManagedLedger::open("t", &ring, ManagedLedgerConfig::default()).unwrap();
        ml.open_cursor("sub-1").unwrap();
        assert!(ml.open_cursor("sub-1").is_err());
        assert_eq!(ml.cursor_count(), 1);
    }

    #[test]
    fn test_managed_ledger_mark_delete_is_monotonic() {
        // cite: pulsar 4.2.0 ManagedCursor.markDelete (no backwards moves)
        // ensemble = ml-007
        let ring = BookieRing::with_size(3);
        let ml = ManagedLedger::open("t", &ring, ManagedLedgerConfig::default()).unwrap();
        ml.open_cursor("sub").unwrap();
        ml.mark_delete("sub", Position::new(1, 5)).unwrap();
        ml.mark_delete("sub", Position::new(1, 2)).unwrap(); // earlier — ignored
        assert_eq!(
            ml.cursor("sub").unwrap().mark_delete_position,
            Some(Position::new(1, 5))
        );
    }

    #[test]
    fn test_managed_ledger_trim_drops_ledgers_fully_below_slowest_cursor() {
        // cite: pulsar 4.2.0 ManagedLedgerImpl#trimConsumedLedgersInBackground
        // ensemble = ml-008
        let ring = BookieRing::with_size(3);
        let ml = ManagedLedger::open("t", &ring, small_config(2)).unwrap();
        ml.add_entry(b"a").unwrap(); // (1,0)
        ml.add_entry(b"b").unwrap(); // (1,1) -> rolls
        ml.add_entry(b"c").unwrap(); // (2,0) -> active
        ml.open_cursor("sub").unwrap();
        // Mark-delete sits in ledger 2 — ledger 1 fully consumed.
        ml.mark_delete("sub", Position::new(2, 0)).unwrap();
        let trimmed = ml.trim();
        assert_eq!(trimmed, vec![1]);
        assert!(ml.closed_ledgers().is_empty());
    }

    #[test]
    fn test_managed_ledger_trim_keeps_ledgers_with_unread_cursor() {
        // cite: pulsar 4.2.0 unread cursor pins log_start_offset to earliest
        // ensemble = ml-009
        let ring = BookieRing::with_size(3);
        let ml = ManagedLedger::open("t", &ring, small_config(1)).unwrap();
        ml.add_entry(b"a").unwrap();
        ml.add_entry(b"b").unwrap();
        ml.open_cursor("sub").unwrap();
        // No mark-delete yet — should pin nothing trimmable.
        assert!(ml.trim().is_empty());
    }

    #[test]
    fn test_managed_ledger_slowest_mark_delete_handles_unread_cursor() {
        // cite: pulsar 4.2.0 slowest read cursor in trim policy
        // ensemble = ml-010
        let ring = BookieRing::with_size(3);
        let ml = ManagedLedger::open("t", &ring, ManagedLedgerConfig::default()).unwrap();
        ml.open_cursor("c1").unwrap();
        ml.open_cursor("c2").unwrap();
        ml.mark_delete("c1", Position::new(1, 100)).unwrap();
        // c2 hasn't read yet — slowest = earliest sentinel
        assert_eq!(ml.slowest_mark_delete(), Some(Position::earliest()));
    }

    #[test]
    fn test_managed_ledger_roll_ledger_force_advances() {
        // cite: pulsar 4.2.0 explicit ledger roll on close
        // ensemble = ml-011
        let ring = BookieRing::with_size(3);
        let ml = ManagedLedger::open("t", &ring, ManagedLedgerConfig::default()).unwrap();
        ml.add_entry(b"x").unwrap();
        ml.roll_ledger().unwrap();
        assert_eq!(ml.active_ledger_id(), 2);
        assert_eq!(ml.closed_ledgers(), vec![(1, 1)]);
    }
}
