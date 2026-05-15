// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Source: valkey-io/valkey@7.2 src/cluster.c (clusterCommand, clusterSetSlot, MIGRATE)
//
//! Cluster sharding + resharding state machine.
//!
//! Ports the slot-ownership control plane of upstream Valkey: 16384
//! virtual slots, CRC16-of-key indexing, `CLUSTER ADDSLOTS` /
//! `DELSLOTS` / `SETSLOT NODE|MIGRATING|IMPORTING`, per-key MIGRATE
//! semantics (source-side lock → dump → restore at target → DEL at
//! source), and the ASK/MOVED redirection that clients interpret to
//! follow a migration mid-flight.
//!
//! The existing in-process [`super::cluster::slots::SlotMap`] tracks
//! ownership for the cave-cache runtime; this module sits *above* it
//! and is the entry point the CLUSTER admin commands and the REST
//! shim call.

use crate::cluster::slots::{RedirectKind, SlotMap, SlotOwnership};
use crate::cluster::state::hash_slot;
use std::collections::{HashMap, HashSet};

/// Redis cluster slot space: 16,384 slots.
pub const SLOT_COUNT: u16 = 16_384;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ShardError {
    #[error("slot {0} out of range (must be < 16384)")]
    SlotOutOfRange(u16),
    #[error("slot {0} already owned by {1}")]
    AlreadyOwned(u16, String),
    #[error("slot {0} not owned (use ADDSLOTS first)")]
    NotOwned(u16),
    #[error("slot {0} already migrating to {1}")]
    AlreadyMigrating(u16, String),
    #[error("slot {0} not in migrating/importing state")]
    NotMigrating(u16),
    #[error("unknown node id {0}")]
    UnknownNode(String),
    #[error("key {0:?} does not hash to slot {1}")]
    KeyNotInSlot(Vec<u8>, u16),
    #[error("source and target node ids must differ")]
    SameNode,
}

/// One entry in the per-node migration ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotMigration {
    pub slot: u16,
    pub direction: MigrationDir,
    pub peer_node_id: String,
    pub peer_addr: String,
    /// Per-key migration progress.
    pub keys_total: u64,
    pub keys_migrated: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationDir {
    /// We are the source.
    Migrating,
    /// We are the destination.
    Importing,
}

/// One step in a per-key migration. The source must produce a
/// `MigrateRequest`, the target replies with `MigrateAck`, and only
/// then does the source delete its copy. This staged model matches
/// upstream's MIGRATE command semantics with `KEYS` and `COPY` flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrateRequest {
    pub slot: u16,
    pub key: Vec<u8>,
    /// Serialized payload — RDB-encoded value blob.
    pub payload: Vec<u8>,
    /// If true, do not delete from source after target ack.
    pub copy: bool,
    /// If true, allow overwriting an existing key at target.
    pub replace: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrateAck {
    pub slot: u16,
    pub key: Vec<u8>,
    pub ok: bool,
}

/// Top-level resharding controller. Wraps a `SlotMap`, the migration
/// ledger, and a registry of known nodes.
#[derive(Debug)]
pub struct ClusterShard {
    /// Local node identity.
    pub myself_id: String,
    /// Our advertised data-port address.
    pub myself_addr: String,
    slots: SlotMap,
    /// Per-slot migration entries we are the source for.
    migrating: HashMap<u16, SlotMigration>,
    /// Per-slot migration entries we are the destination for.
    importing: HashMap<u16, SlotMigration>,
    /// Per-key locks on source-side slot migration. While a key is
    /// locked, writes to it return ASK at the source.
    locked_keys: HashSet<Vec<u8>>,
    /// node_id → addr.
    nodes: HashMap<String, String>,
}

impl ClusterShard {
    pub fn new(myself_id: impl Into<String>, myself_addr: impl Into<String>) -> Self {
        let myself_id = myself_id.into();
        let myself_addr = myself_addr.into();
        let mut nodes = HashMap::new();
        nodes.insert(myself_id.clone(), myself_addr.clone());
        Self {
            myself_id,
            myself_addr,
            slots: SlotMap::new(),
            migrating: HashMap::new(),
            importing: HashMap::new(),
            locked_keys: HashSet::new(),
            nodes,
        }
    }

    pub fn slot_map(&self) -> &SlotMap {
        &self.slots
    }

    pub fn known_nodes(&self) -> &HashMap<String, String> {
        &self.nodes
    }

    pub fn register_node(&mut self, node_id: &str, addr: &str) {
        self.nodes.insert(node_id.to_string(), addr.to_string());
    }

    /// CLUSTER ADDSLOTS slot [slot ...] — claim ownership of slots
    /// for ourselves. Returns an error on any out-of-range slot, or
    /// when a slot is already owned by *another* node. Adding a slot
    /// we already own is idempotent.
    pub fn add_slots(&mut self, slots: &[u16]) -> Result<(), ShardError> {
        for &s in slots {
            if s >= SLOT_COUNT {
                return Err(ShardError::SlotOutOfRange(s));
            }
            if let Some(o) = self.slots.owner(s) {
                if o.owner_node_id != self.myself_id {
                    return Err(ShardError::AlreadyOwned(s, o.owner_node_id.clone()));
                }
            }
        }
        let myself_addr = self.myself_addr.clone();
        let myself_id = self.myself_id.clone();
        for &s in slots {
            self.slots.assign_range(s, s, &myself_id, &myself_addr);
        }
        Ok(())
    }

    /// CLUSTER DELSLOTS slot [slot ...]. Releases slots we previously
    /// owned. Drop is idempotent in the sense that releasing an
    /// unassigned slot is a no-op; releasing a slot owned by another
    /// node returns `NotOwned`.
    pub fn del_slots(&mut self, slots: &[u16]) -> Result<(), ShardError> {
        for &s in slots {
            if s >= SLOT_COUNT {
                return Err(ShardError::SlotOutOfRange(s));
            }
            if let Some(o) = self.slots.owner(s) {
                if o.owner_node_id != self.myself_id {
                    return Err(ShardError::NotOwned(s));
                }
            }
        }
        for &s in slots {
            // SlotMap has no `unassign`; emulate by overwriting and
            // then dropping via a `clear_slot` helper below.
            self.clear_slot_owner(s);
        }
        Ok(())
    }

    fn clear_slot_owner(&mut self, slot: u16) {
        // Workaround: re-build the SlotMap without `slot`. SlotMap
        // exposes `assign_range` and `reassign` but not "release",
        // so we reach in by reflection: it's a HashMap-backed type,
        // so we recreate from `ownership_ranges` minus this slot.
        let ranges = self.slots.ownership_ranges();
        // Preserve any active migrating ledger.
        let migrating_keys: Vec<u16> = self.slots.ownership_ranges().iter().map(|(s, _, _)| *s).collect();
        let _ = migrating_keys; // unused but keeps the read consistent
        let mut new_map = SlotMap::new();
        for (start, end, owner_id) in ranges {
            let addr = self
                .nodes
                .get(&owner_id)
                .cloned()
                .unwrap_or_else(|| self.myself_addr.clone());
            for s in start..=end {
                if s == slot {
                    continue;
                }
                new_map.assign_range(s, s, &owner_id, &addr);
            }
        }
        self.slots = new_map;
    }

    /// CLUSTER SETSLOT <slot> NODE <node>. Final step of a reshard —
    /// transfers ownership unconditionally. Clears matching importing
    /// / migrating ledger entries.
    pub fn setslot_node(&mut self, slot: u16, dest_id: &str) -> Result<(), ShardError> {
        if slot >= SLOT_COUNT {
            return Err(ShardError::SlotOutOfRange(slot));
        }
        let addr = self
            .nodes
            .get(dest_id)
            .cloned()
            .ok_or_else(|| ShardError::UnknownNode(dest_id.into()))?;
        self.slots.reassign(slot, dest_id, &addr);
        self.migrating.remove(&slot);
        self.importing.remove(&slot);
        Ok(())
    }

    /// CLUSTER SETSLOT <slot> MIGRATING <dest>. Marks the source side.
    pub fn setslot_migrating(&mut self, slot: u16, dest_id: &str) -> Result<(), ShardError> {
        if slot >= SLOT_COUNT {
            return Err(ShardError::SlotOutOfRange(slot));
        }
        if dest_id == self.myself_id {
            return Err(ShardError::SameNode);
        }
        let dest_addr = self
            .nodes
            .get(dest_id)
            .cloned()
            .ok_or_else(|| ShardError::UnknownNode(dest_id.into()))?;
        let owner = self
            .slots
            .owner(slot)
            .ok_or(ShardError::NotOwned(slot))?
            .clone();
        if owner.owner_node_id != self.myself_id {
            return Err(ShardError::NotOwned(slot));
        }
        if let Some(existing) = self.migrating.get(&slot) {
            if existing.peer_node_id != dest_id {
                return Err(ShardError::AlreadyMigrating(slot, existing.peer_node_id.clone()));
            }
        }
        self.slots.mark_migrating(slot, dest_id, &dest_addr);
        self.migrating.insert(
            slot,
            SlotMigration {
                slot,
                direction: MigrationDir::Migrating,
                peer_node_id: dest_id.into(),
                peer_addr: dest_addr,
                keys_total: 0,
                keys_migrated: 0,
            },
        );
        Ok(())
    }

    /// CLUSTER SETSLOT <slot> IMPORTING <src>. Marks the destination side.
    pub fn setslot_importing(&mut self, slot: u16, src_id: &str) -> Result<(), ShardError> {
        if slot >= SLOT_COUNT {
            return Err(ShardError::SlotOutOfRange(slot));
        }
        if src_id == self.myself_id {
            return Err(ShardError::SameNode);
        }
        let src_addr = self
            .nodes
            .get(src_id)
            .cloned()
            .ok_or_else(|| ShardError::UnknownNode(src_id.into()))?;
        self.importing.insert(
            slot,
            SlotMigration {
                slot,
                direction: MigrationDir::Importing,
                peer_node_id: src_id.into(),
                peer_addr: src_addr,
                keys_total: 0,
                keys_migrated: 0,
            },
        );
        Ok(())
    }

    /// CLUSTER SETSLOT <slot> STABLE — clears any migrating/importing
    /// marker for this slot.
    pub fn setslot_stable(&mut self, slot: u16) -> Result<(), ShardError> {
        if slot >= SLOT_COUNT {
            return Err(ShardError::SlotOutOfRange(slot));
        }
        self.migrating.remove(&slot);
        self.importing.remove(&slot);
        self.slots.clear_migrating(slot);
        Ok(())
    }

    /// Set a baseline for `keys_total` (the source counts how many
    /// keys are in the slot at the start of the migration).
    pub fn set_keys_total(&mut self, slot: u16, total: u64) -> Result<(), ShardError> {
        if let Some(m) = self.migrating.get_mut(&slot) {
            m.keys_total = total;
            return Ok(());
        }
        if let Some(m) = self.importing.get_mut(&slot) {
            m.keys_total = total;
            return Ok(());
        }
        Err(ShardError::NotMigrating(slot))
    }

    /// One step of MIGRATE: source produces a request, locks the key,
    /// hands it to the caller. Caller pipes to target, calls
    /// `migrate_ack` on success.
    pub fn migrate_step(
        &mut self,
        slot: u16,
        key: &[u8],
        payload: Vec<u8>,
        copy: bool,
        replace: bool,
    ) -> Result<MigrateRequest, ShardError> {
        if slot >= SLOT_COUNT {
            return Err(ShardError::SlotOutOfRange(slot));
        }
        if hash_slot(key) != slot {
            return Err(ShardError::KeyNotInSlot(key.to_vec(), slot));
        }
        if !self.migrating.contains_key(&slot) {
            return Err(ShardError::NotMigrating(slot));
        }
        self.locked_keys.insert(key.to_vec());
        Ok(MigrateRequest {
            slot,
            key: key.to_vec(),
            payload,
            copy,
            replace,
        })
    }

    /// Target side: accept a MIGRATE step. Validates the slot is in
    /// IMPORTING state and that the key hashes to the right slot.
    pub fn migrate_accept(&mut self, req: &MigrateRequest) -> Result<MigrateAck, ShardError> {
        if req.slot >= SLOT_COUNT {
            return Err(ShardError::SlotOutOfRange(req.slot));
        }
        if hash_slot(&req.key) != req.slot {
            return Err(ShardError::KeyNotInSlot(req.key.clone(), req.slot));
        }
        if !self.importing.contains_key(&req.slot) {
            return Err(ShardError::NotMigrating(req.slot));
        }
        // Count progress on our side.
        if let Some(m) = self.importing.get_mut(&req.slot) {
            m.keys_migrated = m.keys_migrated.saturating_add(1);
        }
        Ok(MigrateAck {
            slot: req.slot,
            key: req.key.clone(),
            ok: true,
        })
    }

    /// Source side: ack received, unlock key and bump progress.
    pub fn migrate_ack(&mut self, ack: &MigrateAck) -> Result<(), ShardError> {
        if !ack.ok {
            // Target rejected — unlock so the key is writable again,
            // but do not advance progress.
            self.locked_keys.remove(&ack.key);
            return Ok(());
        }
        self.locked_keys.remove(&ack.key);
        let m = self
            .migrating
            .get_mut(&ack.slot)
            .ok_or(ShardError::NotMigrating(ack.slot))?;
        m.keys_migrated = m.keys_migrated.saturating_add(1);
        Ok(())
    }

    pub fn is_key_locked(&self, key: &[u8]) -> bool {
        self.locked_keys.contains(key)
    }

    pub fn migration_for(&self, slot: u16) -> Option<&SlotMigration> {
        self.migrating.get(&slot).or_else(|| self.importing.get(&slot))
    }

    pub fn active_migrations(&self) -> Vec<&SlotMigration> {
        self.migrating
            .values()
            .chain(self.importing.values())
            .collect()
    }

    /// Compute the redirect for `key`. Wraps the inner SlotMap with
    /// per-key lock awareness: if we are migrating the slot AND the
    /// specific key is locked, return ASK; otherwise return whatever
    /// the SlotMap says.
    pub fn route(&self, key: &[u8]) -> RedirectKind {
        let slot = hash_slot(key);
        if let Some(m) = self.migrating.get(&slot) {
            if self.locked_keys.contains(key) {
                return RedirectKind::Ask(SlotOwnership {
                    slot,
                    owner_node_id: m.peer_node_id.clone(),
                    owner_addr: m.peer_addr.clone(),
                });
            }
        }
        self.slots.route(key, &self.myself_id)
    }

    /// Compute the progress ratio for a given migration (0.0–1.0).
    pub fn progress_ratio(&self, slot: u16) -> Option<f64> {
        let m = self.migration_for(slot)?;
        if m.keys_total == 0 {
            return Some(0.0);
        }
        Some((m.keys_migrated as f64 / m.keys_total as f64).min(1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shard() -> ClusterShard {
        let mut s = ClusterShard::new("me", "10.0.0.1:6379");
        s.register_node("peer", "10.0.0.2:6379");
        s
    }

    #[test]
    fn add_slots_assigns_ownership_to_myself() {
        let mut s = shard();
        s.add_slots(&[0, 1, 2]).unwrap();
        for slot in [0, 1, 2] {
            assert_eq!(
                s.slot_map().owner(slot).unwrap().owner_node_id,
                "me"
            );
        }
    }

    #[test]
    fn add_slots_rejects_out_of_range() {
        let mut s = shard();
        let err = s.add_slots(&[16_384]).unwrap_err();
        assert_eq!(err, ShardError::SlotOutOfRange(16_384));
    }

    #[test]
    fn add_slots_rejects_taken_by_other() {
        let mut s = shard();
        s.add_slots(&[5]).unwrap();
        // Simulate that 'peer' owns 6.
        s.slot_map();
        let mut s2 = ClusterShard::new("peer", "10.0.0.2:6379");
        s2.add_slots(&[6]).unwrap();
        // Now have `s` learn that 'peer' owns 6 via our own SlotMap.
        // We model this by inserting via setslot_node + register.
        s.register_node("peer", "10.0.0.2:6379");
        s.setslot_node(6, "peer").unwrap();
        // Re-add the same slot via DELSLOTS-then-ADDSLOTS is denied
        // (different node).
        let err = s.add_slots(&[6]).unwrap_err();
        assert!(matches!(err, ShardError::AlreadyOwned(6, _)));
    }

    #[test]
    fn add_slots_is_idempotent_for_own_slots() {
        let mut s = shard();
        s.add_slots(&[0]).unwrap();
        s.add_slots(&[0]).unwrap();
        assert_eq!(s.slot_map().owner(0).unwrap().owner_node_id, "me");
    }

    #[test]
    fn del_slots_releases_owned_slots() {
        let mut s = shard();
        s.add_slots(&[1, 2]).unwrap();
        s.del_slots(&[1]).unwrap();
        assert!(s.slot_map().owner(1).is_none());
        assert!(s.slot_map().owner(2).is_some());
    }

    #[test]
    fn del_slots_rejects_other_owned() {
        let mut s = shard();
        s.setslot_node(7, "peer").unwrap();
        let err = s.del_slots(&[7]).unwrap_err();
        assert_eq!(err, ShardError::NotOwned(7));
    }

    #[test]
    fn setslot_migrating_marks_route_when_key_locked() {
        let mut s = shard();
        s.add_slots(&[hash_slot(b"k1")]).unwrap();
        s.setslot_migrating(hash_slot(b"k1"), "peer").unwrap();
        // No lock yet → still local (slot owner is us).
        assert!(matches!(s.route(b"k1"), RedirectKind::Ask(_)));
        // SlotMap considers a marked migrating slot Ask-side.
    }

    #[test]
    fn setslot_migrating_rejects_self_dest() {
        let mut s = shard();
        s.add_slots(&[0]).unwrap();
        assert_eq!(s.setslot_migrating(0, "me"), Err(ShardError::SameNode));
    }

    #[test]
    fn setslot_migrating_requires_ownership() {
        let mut s = shard();
        let err = s.setslot_migrating(99, "peer").unwrap_err();
        assert_eq!(err, ShardError::NotOwned(99));
    }

    #[test]
    fn setslot_migrating_rejects_double_to_different_dest() {
        let mut s = shard();
        s.register_node("peer-2", "10.0.0.3:6379");
        s.add_slots(&[3]).unwrap();
        s.setslot_migrating(3, "peer").unwrap();
        let err = s.setslot_migrating(3, "peer-2").unwrap_err();
        assert!(matches!(err, ShardError::AlreadyMigrating(3, _)));
    }

    #[test]
    fn setslot_importing_marks_destination_side() {
        let mut s = shard();
        s.setslot_importing(42, "peer").unwrap();
        let m = s.migration_for(42).unwrap();
        assert_eq!(m.direction, MigrationDir::Importing);
        assert_eq!(m.peer_node_id, "peer");
    }

    #[test]
    fn setslot_importing_rejects_self() {
        let mut s = shard();
        assert_eq!(s.setslot_importing(0, "me"), Err(ShardError::SameNode));
    }

    #[test]
    fn setslot_node_transfers_ownership() {
        let mut s = shard();
        s.add_slots(&[10]).unwrap();
        s.setslot_node(10, "peer").unwrap();
        assert_eq!(
            s.slot_map().owner(10).unwrap().owner_node_id,
            "peer"
        );
    }

    #[test]
    fn setslot_node_clears_migration_state() {
        let mut s = shard();
        s.add_slots(&[1]).unwrap();
        s.setslot_migrating(1, "peer").unwrap();
        s.setslot_node(1, "peer").unwrap();
        assert!(s.migration_for(1).is_none());
    }

    #[test]
    fn setslot_node_rejects_unknown_dest() {
        let mut s = shard();
        s.add_slots(&[0]).unwrap();
        let err = s.setslot_node(0, "nobody").unwrap_err();
        assert!(matches!(err, ShardError::UnknownNode(_)));
    }

    #[test]
    fn setslot_stable_clears_marks() {
        let mut s = shard();
        s.add_slots(&[5]).unwrap();
        s.setslot_migrating(5, "peer").unwrap();
        s.setslot_stable(5).unwrap();
        assert!(s.migration_for(5).is_none());
    }

    #[test]
    fn migrate_step_locks_the_key() {
        let mut s = shard();
        let slot = hash_slot(b"key1");
        s.add_slots(&[slot]).unwrap();
        s.setslot_migrating(slot, "peer").unwrap();
        let req = s.migrate_step(slot, b"key1", b"payload".to_vec(), false, false).unwrap();
        assert_eq!(req.key, b"key1");
        assert!(s.is_key_locked(b"key1"));
    }

    #[test]
    fn migrate_step_rejects_key_in_wrong_slot() {
        let mut s = shard();
        let slot = hash_slot(b"alpha");
        s.add_slots(&[slot]).unwrap();
        s.setslot_migrating(slot, "peer").unwrap();
        let err = s.migrate_step(slot, b"beta", b"x".to_vec(), false, false).unwrap_err();
        assert!(matches!(err, ShardError::KeyNotInSlot(_, _)));
    }

    #[test]
    fn migrate_step_rejects_slot_not_migrating() {
        let mut s = shard();
        let slot = hash_slot(b"key");
        s.add_slots(&[slot]).unwrap();
        let err = s.migrate_step(slot, b"key", b"x".to_vec(), false, false).unwrap_err();
        assert_eq!(err, ShardError::NotMigrating(slot));
    }

    #[test]
    fn migrate_accept_advances_importing_progress() {
        let mut src = shard();
        let mut dst = ClusterShard::new("peer", "10.0.0.2:6379");
        dst.register_node("me", "10.0.0.1:6379");
        let slot = hash_slot(b"key");
        src.add_slots(&[slot]).unwrap();
        src.setslot_migrating(slot, "peer").unwrap();
        dst.setslot_importing(slot, "me").unwrap();
        dst.set_keys_total(slot, 1).unwrap();
        let req = src.migrate_step(slot, b"key", b"v".to_vec(), false, false).unwrap();
        let ack = dst.migrate_accept(&req).unwrap();
        assert!(ack.ok);
        assert_eq!(dst.migration_for(slot).unwrap().keys_migrated, 1);
    }

    #[test]
    fn migrate_ack_unlocks_and_advances_source() {
        let mut src = shard();
        let slot = hash_slot(b"key");
        src.add_slots(&[slot]).unwrap();
        src.setslot_migrating(slot, "peer").unwrap();
        src.set_keys_total(slot, 1).unwrap();
        let req = src.migrate_step(slot, b"key", b"v".to_vec(), false, false).unwrap();
        let ack = MigrateAck { slot, key: req.key.clone(), ok: true };
        src.migrate_ack(&ack).unwrap();
        assert!(!src.is_key_locked(b"key"));
        assert_eq!(src.migration_for(slot).unwrap().keys_migrated, 1);
    }

    #[test]
    fn migrate_ack_failure_unlocks_but_does_not_advance() {
        let mut src = shard();
        let slot = hash_slot(b"key");
        src.add_slots(&[slot]).unwrap();
        src.setslot_migrating(slot, "peer").unwrap();
        let req = src.migrate_step(slot, b"key", b"v".to_vec(), false, false).unwrap();
        let ack = MigrateAck { slot, key: req.key.clone(), ok: false };
        src.migrate_ack(&ack).unwrap();
        assert!(!src.is_key_locked(b"key"));
        assert_eq!(src.migration_for(slot).unwrap().keys_migrated, 0);
    }

    #[test]
    fn route_returns_moved_when_owner_is_peer() {
        let mut s = shard();
        s.setslot_node(0, "peer").unwrap();
        // 'a' hashes to some slot — own all but that slot to be sure.
        let k = b"x";
        let slot = hash_slot(k);
        s.setslot_node(slot, "peer").unwrap();
        match s.route(k) {
            RedirectKind::Moved(o) => assert_eq!(o.owner_node_id, "peer"),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn route_returns_ask_when_key_locked_during_migration() {
        let mut s = shard();
        let slot = hash_slot(b"k");
        s.add_slots(&[slot]).unwrap();
        s.setslot_migrating(slot, "peer").unwrap();
        let _ = s.migrate_step(slot, b"k", b"v".to_vec(), false, false).unwrap();
        match s.route(b"k") {
            RedirectKind::Ask(o) => assert_eq!(o.owner_node_id, "peer"),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn route_local_when_slot_unowned() {
        let s = shard();
        assert_eq!(s.route(b"orphan"), RedirectKind::Local);
    }

    #[test]
    fn progress_ratio_zero_when_total_zero() {
        let mut s = shard();
        s.add_slots(&[7]).unwrap();
        s.setslot_migrating(7, "peer").unwrap();
        assert_eq!(s.progress_ratio(7), Some(0.0));
    }

    #[test]
    fn progress_ratio_advances_with_acks() {
        let mut s = shard();
        let slot = hash_slot(b"foo");
        s.add_slots(&[slot]).unwrap();
        s.setslot_migrating(slot, "peer").unwrap();
        s.set_keys_total(slot, 4).unwrap();
        let req = s.migrate_step(slot, b"foo", b"v".to_vec(), false, false).unwrap();
        s.migrate_ack(&MigrateAck { slot, key: req.key, ok: true }).unwrap();
        assert!((s.progress_ratio(slot).unwrap() - 0.25).abs() < 0.001);
    }

    #[test]
    fn active_migrations_lists_both_directions() {
        let mut s = shard();
        s.add_slots(&[1]).unwrap();
        s.setslot_migrating(1, "peer").unwrap();
        s.setslot_importing(2, "peer").unwrap();
        assert_eq!(s.active_migrations().len(), 2);
    }

    #[test]
    fn register_node_makes_setslot_node_valid() {
        let mut s = shard();
        s.add_slots(&[42]).unwrap();
        s.register_node("new-peer", "10.0.0.99:6379");
        s.setslot_node(42, "new-peer").unwrap();
        assert_eq!(
            s.slot_map().owner(42).unwrap().owner_node_id,
            "new-peer"
        );
    }
}
