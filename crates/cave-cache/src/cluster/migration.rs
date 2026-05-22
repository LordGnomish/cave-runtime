// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Slot migration ledger.
//!
//! Mirrors `clusterSetSlot` from `src/cluster.c` plus the key-handoff
//! progress tracking that happens during a `CLUSTER SETSLOT ...
//! IMPORTING` / `MIGRATING` cycle. The on-wire `MIGRATE key` command
//! moves individual keys between source and destination nodes; this
//! ledger tracks the *progress* so an operator can ask "how many keys
//! left?" and the destination knows when to call SETSLOT STABLE.

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationState {
    /// Slot is sending keys outward (we are the source).
    Migrating,
    /// Slot is receiving keys (we are the destination).
    Importing,
    /// Migration is complete; slot is stable.
    Stable,
}

#[derive(Debug, Clone)]
pub struct SlotMigration {
    pub slot: u16,
    pub state: MigrationState,
    /// For Migrating: destination node id. For Importing: source.
    pub peer_node_id: String,
    pub peer_addr: String,
    pub started_at_unix: i64,
    pub keys_total: u64,
    pub keys_migrated: u64,
}

impl SlotMigration {
    pub fn progress_ratio(&self) -> f64 {
        if self.keys_total == 0 {
            return 0.0;
        }
        self.keys_migrated as f64 / self.keys_total as f64
    }

    pub fn is_complete(&self) -> bool {
        self.state == MigrationState::Stable
            || (self.keys_total > 0 && self.keys_migrated >= self.keys_total)
    }
}

#[derive(Debug, Default)]
pub struct MigrationLedger {
    /// `slot → migration`.
    by_slot: HashMap<u16, SlotMigration>,
}

impl MigrationLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a Migrating-side entry: we are the source for `slot`.
    pub fn start_migrating(
        &mut self,
        slot: u16,
        dest_node_id: &str,
        dest_addr: &str,
        keys_total: u64,
        now_unix: i64,
    ) -> SlotMigration {
        let m = SlotMigration {
            slot,
            state: MigrationState::Migrating,
            peer_node_id: dest_node_id.into(),
            peer_addr: dest_addr.into(),
            started_at_unix: now_unix,
            keys_total,
            keys_migrated: 0,
        };
        self.by_slot.insert(slot, m.clone());
        m
    }

    /// Start an Importing-side entry: we are the destination for `slot`.
    pub fn start_importing(
        &mut self,
        slot: u16,
        source_node_id: &str,
        source_addr: &str,
        keys_total: u64,
        now_unix: i64,
    ) -> SlotMigration {
        let m = SlotMigration {
            slot,
            state: MigrationState::Importing,
            peer_node_id: source_node_id.into(),
            peer_addr: source_addr.into(),
            started_at_unix: now_unix,
            keys_total,
            keys_migrated: 0,
        };
        self.by_slot.insert(slot, m.clone());
        m
    }

    /// Note progress on a slot. Returns the updated migration entry.
    pub fn mark_keys_migrated(&mut self, slot: u16, delta: u64) -> Option<SlotMigration> {
        let m = self.by_slot.get_mut(&slot)?;
        m.keys_migrated = m.keys_migrated.saturating_add(delta);
        Some(m.clone())
    }

    /// Finalize a slot — CLUSTER SETSLOT ... STABLE. Returns the
    /// migration entry being closed.
    pub fn finalize(&mut self, slot: u16) -> Option<SlotMigration> {
        let entry = self.by_slot.get_mut(&slot)?;
        entry.state = MigrationState::Stable;
        // Clamp keys_migrated to keys_total if we hit STABLE early.
        if entry.keys_total > 0 && entry.keys_migrated < entry.keys_total {
            entry.keys_migrated = entry.keys_total;
        }
        Some(entry.clone())
    }

    pub fn get(&self, slot: u16) -> Option<&SlotMigration> {
        self.by_slot.get(&slot)
    }

    pub fn active(&self) -> Vec<&SlotMigration> {
        self.by_slot
            .values()
            .filter(|m| m.state != MigrationState::Stable)
            .collect()
    }

    pub fn clear_stable(&mut self) {
        self.by_slot
            .retain(|_, m| m.state != MigrationState::Stable);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_migrating_creates_entry() {
        let mut l = MigrationLedger::new();
        let m = l.start_migrating(42, "dest", "10.0.0.2:6379", 100, 1000);
        assert_eq!(m.state, MigrationState::Migrating);
        assert_eq!(l.get(42).unwrap().peer_node_id, "dest");
    }

    #[test]
    fn start_importing_creates_entry() {
        let mut l = MigrationLedger::new();
        let m = l.start_importing(99, "src", "10.0.0.1:6379", 50, 1000);
        assert_eq!(m.state, MigrationState::Importing);
        assert_eq!(l.get(99).unwrap().peer_node_id, "src");
    }

    #[test]
    fn mark_progress_accumulates() {
        let mut l = MigrationLedger::new();
        l.start_migrating(1, "d", "1", 100, 0);
        l.mark_keys_migrated(1, 25);
        l.mark_keys_migrated(1, 10);
        assert_eq!(l.get(1).unwrap().keys_migrated, 35);
        assert!((l.get(1).unwrap().progress_ratio() - 0.35).abs() < 0.001);
    }

    #[test]
    fn finalize_sets_state_stable() {
        let mut l = MigrationLedger::new();
        l.start_migrating(7, "d", "1", 10, 0);
        l.mark_keys_migrated(7, 5);
        let m = l.finalize(7).unwrap();
        assert_eq!(m.state, MigrationState::Stable);
        // Counter clamped to total.
        assert_eq!(m.keys_migrated, 10);
        assert!(m.is_complete());
    }

    #[test]
    fn active_excludes_stable_entries() {
        let mut l = MigrationLedger::new();
        l.start_migrating(1, "d", "1", 10, 0);
        l.start_migrating(2, "d", "1", 10, 0);
        l.finalize(1);
        let active = l.active();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].slot, 2);
    }

    #[test]
    fn clear_stable_drops_finalized() {
        let mut l = MigrationLedger::new();
        l.start_migrating(1, "d", "1", 10, 0);
        l.start_migrating(2, "d", "1", 10, 0);
        l.finalize(1);
        l.clear_stable();
        assert!(l.get(1).is_none());
        assert!(l.get(2).is_some());
    }

    #[test]
    fn progress_ratio_zero_for_empty_total() {
        let mut l = MigrationLedger::new();
        l.start_migrating(0, "d", "1", 0, 0);
        assert_eq!(l.get(0).unwrap().progress_ratio(), 0.0);
    }

    #[test]
    fn is_complete_when_keys_match() {
        let mut l = MigrationLedger::new();
        l.start_migrating(0, "d", "1", 5, 0);
        l.mark_keys_migrated(0, 5);
        assert!(l.get(0).unwrap().is_complete());
    }
}
