// SPDX-License-Identifier: AGPL-3.0-or-later
//! 16,384-slot routing table.
//!
//! Mirrors the slot-ownership half of `src/cluster.c` plus the
//! per-slot counters from `src/cluster_slot_stats.c`. A `SlotMap`
//! tracks which node owns each slot, and emits MOVED / ASK redirect
//! info for a key lookup that lands on a slot we don't own.

use super::state::hash_slot;
use std::collections::HashMap;

pub const SLOT_COUNT: u16 = 16_384;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotOwnership {
    pub slot: u16,
    pub owner_node_id: String,
    /// Address the client should redirect to (host:port form).
    pub owner_addr: String,
}

#[derive(Debug, Default, Clone)]
pub struct SlotStats {
    pub key_count: u64,
    pub cpu_usec: u64,
    pub network_bytes_in: u64,
    pub network_bytes_out: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedirectKind {
    /// Key belongs to a different slot than we own — redirect.
    Moved(SlotOwnership),
    /// Slot is in the middle of migrating to another node. Client
    /// should ASK that node, but stay loyal to this one until SETSLOT
    /// completes.
    Ask(SlotOwnership),
    /// We own the slot — proceed locally.
    Local,
}

#[derive(Debug, Default)]
pub struct SlotMap {
    /// `slot → owner`. A missing entry means the slot is unassigned
    /// (cluster is bootstrapping).
    owners: HashMap<u16, SlotOwnership>,
    /// Slots currently importing or migrating; entries are erased on
    /// SETSLOT STABLE.
    migrating: HashMap<u16, SlotOwnership>,
    stats: HashMap<u16, SlotStats>,
}

impl SlotMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Assign a contiguous slot range to a single node.
    pub fn assign_range(&mut self, start: u16, end: u16, node_id: &str, node_addr: &str) {
        assert!(start <= end && end < SLOT_COUNT);
        for s in start..=end {
            self.owners.insert(
                s,
                SlotOwnership {
                    slot: s,
                    owner_node_id: node_id.to_string(),
                    owner_addr: node_addr.to_string(),
                },
            );
        }
    }

    /// Hand a slot off to another node (the final step of CLUSTER
    /// SETSLOT). Clears the migration mark and updates ownership.
    pub fn reassign(&mut self, slot: u16, node_id: &str, node_addr: &str) {
        self.owners.insert(
            slot,
            SlotOwnership {
                slot,
                owner_node_id: node_id.to_string(),
                owner_addr: node_addr.to_string(),
            },
        );
        self.migrating.remove(&slot);
    }

    /// Mark a slot as migrating to a destination node. The owner
    /// remains the source until [`SlotMap::reassign`] finalizes.
    pub fn mark_migrating(&mut self, slot: u16, dest_node_id: &str, dest_addr: &str) {
        self.migrating.insert(
            slot,
            SlotOwnership {
                slot,
                owner_node_id: dest_node_id.to_string(),
                owner_addr: dest_addr.to_string(),
            },
        );
    }

    pub fn clear_migrating(&mut self, slot: u16) {
        self.migrating.remove(&slot);
    }

    pub fn is_migrating(&self, slot: u16) -> bool {
        self.migrating.contains_key(&slot)
    }

    pub fn owner(&self, slot: u16) -> Option<&SlotOwnership> {
        self.owners.get(&slot)
    }

    pub fn migrating_target(&self, slot: u16) -> Option<&SlotOwnership> {
        self.migrating.get(&slot)
    }

    /// Per-node slot count. Used by CLUSTER INFO's cluster_size field.
    pub fn nodes_owning_slots(&self) -> HashMap<String, u32> {
        let mut counts: HashMap<String, u32> = HashMap::new();
        for o in self.owners.values() {
            *counts.entry(o.owner_node_id.clone()).or_default() += 1;
        }
        counts
    }

    /// Set of `(slot_start, slot_end, owner)` runs — the on-wire shape
    /// CLUSTER NODES emits.
    pub fn ownership_ranges(&self) -> Vec<(u16, u16, String)> {
        let mut slots: Vec<u16> = self.owners.keys().copied().collect();
        slots.sort();
        let mut out = Vec::new();
        let mut iter = slots.into_iter();
        let Some(first) = iter.next() else {
            return out;
        };
        let mut run_start = first;
        let mut run_end = first;
        let mut run_owner = self.owners[&first].owner_node_id.clone();
        for s in iter {
            let owner = self.owners[&s].owner_node_id.clone();
            if s == run_end + 1 && owner == run_owner {
                run_end = s;
            } else {
                out.push((run_start, run_end, run_owner.clone()));
                run_start = s;
                run_end = s;
                run_owner = owner;
            }
        }
        out.push((run_start, run_end, run_owner));
        out
    }

    /// Resolve a key against this slot map for the local node id.
    /// Returns `Local` if we own the slot, `Moved` if it belongs
    /// elsewhere, `Ask` if it's mid-migration to a new owner.
    pub fn route(&self, key: &[u8], local_node_id: &str) -> RedirectKind {
        let slot = hash_slot(key);
        let owner = match self.owners.get(&slot) {
            Some(o) => o.clone(),
            None => return RedirectKind::Local, // unassigned — fall through to local handling
        };
        if owner.owner_node_id != local_node_id {
            return RedirectKind::Moved(owner);
        }
        if let Some(dest) = self.migrating.get(&slot) {
            return RedirectKind::Ask(dest.clone());
        }
        RedirectKind::Local
    }

    /// Record a slot access for the per-slot stats table.
    pub fn record_access(&mut self, slot: u16, bytes_in: u64, bytes_out: u64, cpu_usec: u64) {
        let entry = self.stats.entry(slot).or_default();
        entry.key_count = entry.key_count.saturating_add(1);
        entry.network_bytes_in = entry.network_bytes_in.saturating_add(bytes_in);
        entry.network_bytes_out = entry.network_bytes_out.saturating_add(bytes_out);
        entry.cpu_usec = entry.cpu_usec.saturating_add(cpu_usec);
    }

    pub fn stats_for(&self, slot: u16) -> SlotStats {
        self.stats.get(&slot).cloned().unwrap_or_default()
    }

    pub fn top_slots_by_keys(&self, top_n: usize) -> Vec<(u16, u64)> {
        let mut all: Vec<(u16, u64)> =
            self.stats.iter().map(|(s, st)| (*s, st.key_count)).collect();
        all.sort_by(|a, b| b.1.cmp(&a.1));
        all.truncate(top_n);
        all
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assigned_range_round_trips() {
        let mut m = SlotMap::new();
        m.assign_range(0, 5460, "node-a", "10.0.0.1:6379");
        assert_eq!(m.owner(0).unwrap().owner_node_id, "node-a");
        assert_eq!(m.owner(5460).unwrap().owner_node_id, "node-a");
        assert!(m.owner(5461).is_none());
    }

    #[test]
    fn nodes_owning_slots_counts_each_node() {
        let mut m = SlotMap::new();
        m.assign_range(0, 5460, "a", "1");
        m.assign_range(5461, 10922, "b", "2");
        m.assign_range(10923, 16383, "c", "3");
        let counts = m.nodes_owning_slots();
        assert_eq!(counts["a"], 5461);
        assert_eq!(counts["b"], 5462);
        assert_eq!(counts["c"], 5461);
    }

    #[test]
    fn ownership_ranges_collapses_contiguous_runs() {
        let mut m = SlotMap::new();
        m.assign_range(0, 100, "a", "1");
        m.assign_range(101, 200, "b", "2");
        m.assign_range(201, 300, "a", "1");
        let r = m.ownership_ranges();
        assert_eq!(r, vec![(0, 100, "a".into()), (101, 200, "b".into()), (201, 300, "a".into())]);
    }

    #[test]
    fn route_local_when_we_own_slot() {
        let mut m = SlotMap::new();
        m.assign_range(0, 16383, "me", "1");
        let r = m.route(b"x", "me");
        assert_eq!(r, RedirectKind::Local);
    }

    #[test]
    fn route_moved_when_other_owns_slot() {
        let mut m = SlotMap::new();
        m.assign_range(0, 16383, "them", "10.0.0.2:6379");
        let r = m.route(b"x", "me");
        match r {
            RedirectKind::Moved(o) => {
                assert_eq!(o.owner_node_id, "them");
                assert_eq!(o.owner_addr, "10.0.0.2:6379");
            }
            _ => panic!("expected Moved"),
        }
    }

    #[test]
    fn route_ask_when_slot_migrating_from_us() {
        let mut m = SlotMap::new();
        m.assign_range(0, 16383, "me", "1");
        let slot = hash_slot(b"x");
        m.mark_migrating(slot, "dest", "10.0.0.3:6379");
        match m.route(b"x", "me") {
            RedirectKind::Ask(o) => assert_eq!(o.owner_node_id, "dest"),
            other => panic!("expected Ask got {other:?}"),
        }
    }

    #[test]
    fn unassigned_slot_routes_local() {
        let m = SlotMap::new();
        // No slots assigned — everything resolves Local (bootstrap).
        let r = m.route(b"x", "anyone");
        assert_eq!(r, RedirectKind::Local);
    }

    #[test]
    fn reassign_finalizes_migration() {
        let mut m = SlotMap::new();
        m.assign_range(0, 16383, "src", "1");
        let slot = hash_slot(b"x");
        m.mark_migrating(slot, "dest", "2");
        assert!(m.is_migrating(slot));
        m.reassign(slot, "dest", "2");
        assert!(!m.is_migrating(slot));
        assert_eq!(m.owner(slot).unwrap().owner_node_id, "dest");
    }

    #[test]
    fn stats_record_and_query() {
        let mut m = SlotMap::new();
        let s = hash_slot(b"k");
        m.record_access(s, 12, 5, 100);
        m.record_access(s, 8, 3, 50);
        let st = m.stats_for(s);
        assert_eq!(st.key_count, 2);
        assert_eq!(st.network_bytes_in, 20);
        assert_eq!(st.network_bytes_out, 8);
        assert_eq!(st.cpu_usec, 150);
    }

    #[test]
    fn top_slots_by_keys_orders_descending() {
        let mut m = SlotMap::new();
        m.record_access(7, 0, 0, 0);
        m.record_access(7, 0, 0, 0);
        m.record_access(7, 0, 0, 0);
        m.record_access(11, 0, 0, 0);
        let top = m.top_slots_by_keys(2);
        assert_eq!(top[0].0, 7);
        assert_eq!(top[0].1, 3);
        assert_eq!(top[1].0, 11);
    }

    #[test]
    fn hashtag_keys_route_to_same_slot() {
        let mut m = SlotMap::new();
        m.assign_range(0, 16383, "me", "1");
        // Two keys with the same hashtag must yield the same slot.
        assert_eq!(hash_slot(b"foo{tag}bar"), hash_slot(b"baz{tag}qux"));
    }
}
