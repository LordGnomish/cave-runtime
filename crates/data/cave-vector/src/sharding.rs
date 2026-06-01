// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Sharding + replication.
//!
//! Port of the Qdrant `lib/collection/src/shards` routing model: a
//! consistent-hash ring ([`HashRing`]) maps point ids → shards with minimal
//! churn on resharding, and a [`ReplicaSet`] tracks per-shard replica health
//! with a write-consistency quorum (`ReplicaState`).

use crate::models::PointId;
use std::collections::BTreeMap;

/// Logical shard identifier.
pub type ShardId = u32;

/// FNV-1a 64-bit — a stable, build-independent hash for ring placement.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn hash_key(id: &PointId) -> u64 {
    match id {
        PointId::Num(n) => fnv1a(&n.to_le_bytes()),
        PointId::Uuid(s) => fnv1a(s.as_bytes()),
    }
}

/// Consistent-hash ring with virtual nodes.
#[derive(Debug, Clone)]
pub struct HashRing {
    ring: BTreeMap<u64, ShardId>,
    vnodes: usize,
}

impl HashRing {
    /// Build a ring over `shards`, each placed at `vnodes` ring positions.
    pub fn new(_shards: &[ShardId], vnodes: usize) -> Self {
        Self { ring: BTreeMap::new(), vnodes }
    }

    /// Add a shard's virtual nodes.
    pub fn add_shard(&mut self, _shard: ShardId) {}

    /// Remove a shard's virtual nodes.
    pub fn remove_shard(&mut self, _shard: ShardId) {}

    /// Route a key to its owning shard (first ring node clockwise).
    pub fn route(&self, _id: &PointId) -> Option<ShardId> {
        None
    }

    /// Route a key to `rf` distinct replica shards (clockwise walk).
    pub fn route_replicas(&self, _id: &PointId, _rf: usize) -> Vec<ShardId> {
        Vec::new()
    }
}

/// Health state of a single replica (Qdrant `ReplicaState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplicaState {
    /// Fully synced and serving.
    Active,
    /// Receiving an initial transfer.
    Initializing,
    /// Partially synced (recovering / behind).
    Partial,
    /// Unreachable / failed.
    Dead,
}

/// One replica placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Replica {
    /// Peer/node holding the replica.
    pub peer: u64,
    /// Current health state.
    pub state: ReplicaState,
}

/// The set of replicas backing one shard.
#[derive(Debug, Clone)]
pub struct ReplicaSet {
    /// Shard this set backs.
    pub shard: ShardId,
    /// Replica placements.
    pub replicas: Vec<Replica>,
}

impl ReplicaSet {
    /// New set with all replicas `Initializing`.
    pub fn new(shard: ShardId, peers: &[u64]) -> Self {
        Self {
            shard,
            replicas: peers
                .iter()
                .map(|&peer| Replica { peer, state: ReplicaState::Initializing })
                .collect(),
        }
    }

    /// Peers currently `Active`.
    pub fn active_peers(&self) -> Vec<u64> {
        Vec::new()
    }

    /// Transition a peer's state. Returns whether the peer existed.
    pub fn set_state(&mut self, _peer: u64, _state: ReplicaState) -> bool {
        false
    }

    /// Whether at least `write_factor` replicas are active (write quorum).
    pub fn is_writable(&self, _write_factor: usize) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring3() -> HashRing {
        HashRing::new(&[0, 1, 2], 64)
    }

    #[test]
    fn route_is_deterministic_and_in_range() {
        let r = ring3();
        for i in 0..100u64 {
            let s = r.route(&PointId::Num(i)).unwrap();
            assert!(s < 3);
            assert_eq!(s, r.route(&PointId::Num(i)).unwrap());
        }
    }

    #[test]
    fn keys_spread_across_all_shards() {
        let r = ring3();
        let mut seen = [0usize; 3];
        for i in 0..3000u64 {
            seen[r.route(&PointId::Num(i)).unwrap() as usize] += 1;
        }
        // every shard owns a meaningful slice (not perfectly even, but > 10%).
        for c in seen {
            assert!(c > 300, "uneven distribution: {seen:?}");
        }
    }

    #[test]
    fn adding_shard_moves_minority_of_keys() {
        let r3 = ring3();
        let mut r4 = ring3();
        r4.add_shard(3);
        let n = 3000u64;
        let mut moved = 0;
        for i in 0..n {
            let a = r3.route(&PointId::Num(i)).unwrap();
            let b = r4.route(&PointId::Num(i)).unwrap();
            if a != b {
                moved += 1;
            }
        }
        let frac = moved as f64 / n as f64;
        // consistent hashing: ~1/4 expected; well under half.
        assert!(frac < 0.45, "moved fraction {frac} too high");
        assert!(frac > 0.05, "suspiciously low churn {frac}");
    }

    #[test]
    fn replicas_are_distinct_shards() {
        let r = ring3();
        let reps = r.route_replicas(&PointId::Num(42), 3);
        assert_eq!(reps.len(), 3);
        let mut sorted = reps.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 3, "replicas must be distinct: {reps:?}");
    }

    #[test]
    fn replicas_capped_at_shard_count() {
        let r = ring3();
        // rf larger than shards → at most 3 distinct.
        let reps = r.route_replicas(&PointId::Num(42), 9);
        assert_eq!(reps.len(), 3);
    }

    #[test]
    fn replica_set_quorum() {
        let mut rs = ReplicaSet::new(0, &[10, 11, 12]);
        assert!(rs.active_peers().is_empty());
        assert!(!rs.is_writable(2));
        rs.set_state(10, ReplicaState::Active);
        rs.set_state(11, ReplicaState::Active);
        assert_eq!(rs.active_peers().len(), 2);
        assert!(rs.is_writable(2));
        rs.set_state(11, ReplicaState::Dead);
        assert!(!rs.is_writable(2));
        assert!(rs.is_writable(1));
    }

    #[test]
    fn set_state_unknown_peer_is_false() {
        let mut rs = ReplicaSet::new(0, &[10]);
        assert!(!rs.set_state(99, ReplicaState::Active));
    }
}
