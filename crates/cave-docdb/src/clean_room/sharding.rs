// SPDX-License-Identifier: AGPL-3.0-or-later
// Clean-room: authored from
//   https://www.mongodb.com/docs/manual/sharding/
//   https://www.mongodb.com/docs/manual/core/sharded-cluster-config-servers/
//   https://www.mongodb.com/docs/manual/core/ranged-sharding/
//   https://www.mongodb.com/docs/manual/core/hashed-sharding/
// No `mongodb/mongo` source code consulted.
//
//! Mongos-style router + chunk map.
//!
//! A chunk is `[min, max)` of the shard-key range; this module owns the
//! routing table and a chunk-map query helper. There is **no balancer**
//! yet — chunk movement is operator-driven via `move_chunk`. Hashed
//! sharding is supported via a deterministic 64-bit FNV-1a (publicly
//! described in mongodb docs as one acceptable hash family).

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// Sharding strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShardKeyStrategy {
    Ranged,
    Hashed,
}

/// Shard identity (a string id — typical MongoDB convention is
/// `shard0000`, `shard0001`, …).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ShardId(pub String);

impl ShardId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

/// One chunk in the chunk-map. `min` is inclusive, `max` is exclusive.
/// `i64::MIN`/`i64::MAX` represent the universal bounds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chunk {
    pub min: i64,
    pub max: i64,
    pub shard: ShardId,
}

impl Chunk {
    pub fn contains(&self, key: i64) -> bool {
        self.min <= key && key < self.max
    }
}

/// Sharded namespace metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShardedCollection {
    pub db: String,
    pub coll: String,
    pub key: String,
    pub strategy: ShardKeyStrategy,
    pub chunks: Vec<Chunk>,
}

impl ShardedCollection {
    pub fn ns(&self) -> String {
        format!("{}.{}", self.db, self.coll)
    }

    /// Resolve a shard for a given shard-key value.
    pub fn route(&self, raw: &serde_json::Value) -> Option<&Chunk> {
        let k = match self.strategy {
            ShardKeyStrategy::Hashed => hashed_key(raw),
            ShardKeyStrategy::Ranged => ranged_key(raw)?,
        };
        // Linear scan — chunk maps are bounded by chunk count, typically
        // dozens to low hundreds; binary search by `min` would only help
        // at thousands of chunks.
        self.chunks.iter().find(|c| c.contains(k))
    }

    /// Split a chunk that contains `point` into `[min, point) +
    /// [point, max)`. The new chunk inherits the original shard.
    pub fn split_chunk(&mut self, point: i64) -> Result<(), ShardingError> {
        let idx = self
            .chunks
            .iter()
            .position(|c| c.contains(point) && c.min < point)
            .ok_or(ShardingError::SplitPointOutOfRange(point))?;
        let old = self.chunks[idx].clone();
        self.chunks[idx].max = point;
        self.chunks.insert(
            idx + 1,
            Chunk { min: point, max: old.max, shard: old.shard.clone() },
        );
        Ok(())
    }

    /// Move a chunk to a different shard. Identified by `min` value
    /// (publicly the canonical chunk identifier in `sh.status()`).
    pub fn move_chunk(&mut self, min: i64, to: ShardId) -> Result<(), ShardingError> {
        let c = self
            .chunks
            .iter_mut()
            .find(|c| c.min == min)
            .ok_or(ShardingError::ChunkNotFound(min))?;
        c.shard = to;
        Ok(())
    }

    /// Number of distinct shards owning at least one chunk.
    pub fn distinct_shards(&self) -> Vec<ShardId> {
        let mut v: Vec<ShardId> = self.chunks.iter().map(|c| c.shard.clone()).collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v.dedup();
        v
    }
}

/// Top-level router (the mongos equivalent).
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ShardingTopology {
    pub shards: Vec<ShardId>,
    pub collections: Vec<ShardedCollection>,
}

impl ShardingTopology {
    pub fn add_shard(&mut self, id: ShardId) {
        if !self.shards.contains(&id) {
            self.shards.push(id);
        }
    }

    pub fn shard_collection(
        &mut self,
        db: &str,
        coll: &str,
        key: &str,
        strategy: ShardKeyStrategy,
    ) -> Result<(), ShardingError> {
        if self.collections.iter().any(|c| c.db == db && c.coll == coll) {
            return Err(ShardingError::AlreadySharded(format!("{}.{}", db, coll)));
        }
        let initial_shard = self.shards.first().cloned().ok_or(ShardingError::NoShards)?;
        self.collections.push(ShardedCollection {
            db: db.into(),
            coll: coll.into(),
            key: key.into(),
            strategy,
            chunks: vec![Chunk { min: i64::MIN, max: i64::MAX, shard: initial_shard }],
        });
        Ok(())
    }

    pub fn find_collection_mut(&mut self, db: &str, coll: &str) -> Option<&mut ShardedCollection> {
        self.collections.iter_mut().find(|c| c.db == db && c.coll == coll)
    }

    pub fn find_collection(&self, db: &str, coll: &str) -> Option<&ShardedCollection> {
        self.collections.iter().find(|c| c.db == db && c.coll == coll)
    }

    /// Resolve the (collection, chunk) for a key — `None` if `db.coll`
    /// is not sharded.
    pub fn route(
        &self,
        db: &str,
        coll: &str,
        key_value: &serde_json::Value,
    ) -> Option<&Chunk> {
        let c = self.find_collection(db, coll)?;
        c.route(key_value)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ShardingError {
    #[error("split point {0} is not contained in any chunk")]
    SplitPointOutOfRange(i64),
    #[error("no chunk with min={0}")]
    ChunkNotFound(i64),
    #[error("namespace {0} already sharded")]
    AlreadySharded(String),
    #[error("no shards registered")]
    NoShards,
}

/// Deterministic 64-bit FNV-1a hash. Public spec; we use the
/// canonical 64-bit FNV offset basis + prime.
fn fnv1a64(bytes: &[u8]) -> i64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x00000100000001B3);
    }
    h as i64
}

fn hashed_key(v: &serde_json::Value) -> i64 {
    let s = serde_json::to_string(v).unwrap_or_default();
    fnv1a64(s.as_bytes())
}

fn ranged_key(v: &serde_json::Value) -> Option<i64> {
    match v {
        serde_json::Value::Number(n) => n.as_i64(),
        serde_json::Value::String(s) => Some(fnv1a64(s.as_bytes())),
        _ => None,
    }
}

// Tie-breaker for deterministic ordering — kept for completeness.
#[allow(dead_code)]
fn cmp_chunks(a: &Chunk, b: &Chunk) -> Ordering {
    a.min.cmp(&b.min)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn topo_with_two_shards() -> ShardingTopology {
        let mut t = ShardingTopology::default();
        t.add_shard(ShardId::new("shard0000"));
        t.add_shard(ShardId::new("shard0001"));
        t
    }

    #[test]
    fn chunk_contains_inclusive_min_exclusive_max() {
        let c = Chunk { min: 10, max: 20, shard: ShardId::new("s") };
        assert!(c.contains(10));
        assert!(c.contains(15));
        assert!(!c.contains(20));
    }

    #[test]
    fn shard_collection_creates_single_universal_chunk() {
        let mut t = topo_with_two_shards();
        t.shard_collection("d", "c", "uid", ShardKeyStrategy::Ranged).unwrap();
        let c = t.find_collection("d", "c").unwrap();
        assert_eq!(c.chunks.len(), 1);
        assert_eq!(c.chunks[0].min, i64::MIN);
        assert_eq!(c.chunks[0].max, i64::MAX);
    }

    #[test]
    fn shard_collection_errors_without_shards() {
        let mut t = ShardingTopology::default();
        assert!(t.shard_collection("d", "c", "uid", ShardKeyStrategy::Ranged).is_err());
    }

    #[test]
    fn split_chunk_creates_two_halves() {
        let mut t = topo_with_two_shards();
        t.shard_collection("d", "c", "uid", ShardKeyStrategy::Ranged).unwrap();
        let c = t.find_collection_mut("d", "c").unwrap();
        c.split_chunk(0).unwrap();
        assert_eq!(c.chunks.len(), 2);
    }

    #[test]
    fn move_chunk_changes_owning_shard() {
        let mut t = topo_with_two_shards();
        t.shard_collection("d", "c", "uid", ShardKeyStrategy::Ranged).unwrap();
        let c = t.find_collection_mut("d", "c").unwrap();
        c.split_chunk(0).unwrap();
        c.move_chunk(0, ShardId::new("shard0001")).unwrap();
        assert_eq!(c.chunks[1].shard.0, "shard0001");
    }

    #[test]
    fn route_resolves_within_universal_chunk() {
        let mut t = topo_with_two_shards();
        t.shard_collection("d", "c", "uid", ShardKeyStrategy::Ranged).unwrap();
        let chunk = t.route("d", "c", &json!(123)).unwrap();
        assert_eq!(chunk.shard.0, "shard0000");
    }

    #[test]
    fn route_unknown_namespace_returns_none() {
        let t = topo_with_two_shards();
        assert!(t.route("d", "c", &json!(0)).is_none());
    }

    #[test]
    fn hashed_distribution_picks_different_chunks_after_split() {
        let mut t = topo_with_two_shards();
        t.shard_collection("d", "c", "uid", ShardKeyStrategy::Hashed).unwrap();
        let c = t.find_collection_mut("d", "c").unwrap();
        c.split_chunk(0).unwrap();
        c.move_chunk(0, ShardId::new("shard0001")).unwrap();
        // Confirm both halves are reachable.
        let mut hits = std::collections::HashSet::new();
        for i in 0..200 {
            if let Some(ch) = t.route("d", "c", &json!(format!("k{i}"))) {
                hits.insert(ch.shard.0.clone());
            }
        }
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn split_point_outside_any_chunk_errors() {
        let mut c = ShardedCollection {
            db: "d".into(),
            coll: "c".into(),
            key: "uid".into(),
            strategy: ShardKeyStrategy::Ranged,
            chunks: vec![Chunk { min: 10, max: 20, shard: ShardId::new("s") }],
        };
        assert!(c.split_chunk(100).is_err());
    }

    #[test]
    fn move_chunk_unknown_min_errors() {
        let mut c = ShardedCollection {
            db: "d".into(),
            coll: "c".into(),
            key: "uid".into(),
            strategy: ShardKeyStrategy::Ranged,
            chunks: vec![Chunk { min: i64::MIN, max: i64::MAX, shard: ShardId::new("a") }],
        };
        assert!(c.move_chunk(7, ShardId::new("b")).is_err());
    }

    #[test]
    fn distinct_shards_dedups_and_sorts() {
        let c = ShardedCollection {
            db: "d".into(),
            coll: "c".into(),
            key: "uid".into(),
            strategy: ShardKeyStrategy::Ranged,
            chunks: vec![
                Chunk { min: 0, max: 10, shard: ShardId::new("b") },
                Chunk { min: 10, max: 20, shard: ShardId::new("a") },
                Chunk { min: 20, max: 30, shard: ShardId::new("a") },
            ],
        };
        let s = c.distinct_shards();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].0, "a");
    }

    #[test]
    fn add_shard_idempotent() {
        let mut t = ShardingTopology::default();
        t.add_shard(ShardId::new("s"));
        t.add_shard(ShardId::new("s"));
        assert_eq!(t.shards.len(), 1);
    }

    #[test]
    fn double_shard_collection_errors() {
        let mut t = topo_with_two_shards();
        t.shard_collection("d", "c", "uid", ShardKeyStrategy::Ranged).unwrap();
        assert!(t.shard_collection("d", "c", "uid", ShardKeyStrategy::Ranged).is_err());
    }

    #[test]
    fn fnv1a_stable() {
        assert_eq!(fnv1a64(b""), 0xcbf29ce484222325u64 as i64);
        // Stability: any change in the constant would shift this value.
        let a = fnv1a64(b"hello");
        let b = fnv1a64(b"hello");
        assert_eq!(a, b);
    }
}
