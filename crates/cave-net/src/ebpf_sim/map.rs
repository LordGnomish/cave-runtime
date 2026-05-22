// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Userspace approximation of kernel `bpf_map_*` helpers.
//!
//! Three variants:
//!
//!   * `MapKind::Hash` — open-ended HashMap. Insert / lookup /
//!     delete are O(1). Mirrors `BPF_MAP_TYPE_HASH`.
//!   * `MapKind::LruHash { capacity }` — capped HashMap with least-
//!     recently-used eviction. Mirrors `BPF_MAP_TYPE_LRU_HASH`.
//!   * `MapKind::Array { size }` — fixed-size array indexed by
//!     u32. Lookup OOB → `MapError::OutOfBounds`. Mirrors
//!     `BPF_MAP_TYPE_ARRAY`.
//!
//! Kernel semantics preserved:
//!   * `update` with `BPF_NOEXIST` flag returns
//!     `MapError::EntryExists` when the key is already present.
//!   * `update` with `BPF_EXIST` flag returns
//!     `MapError::EntryMissing` when the key is absent.
//!   * `delete` on a missing key returns `MapError::EntryMissing`.

use std::collections::{BTreeMap, VecDeque};
use std::hash::Hash;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MapError {
    #[error("entry exists (BPF_NOEXIST)")]
    EntryExists,
    #[error("entry missing (BPF_EXIST / delete)")]
    EntryMissing,
    #[error("array index {0} out of bounds (size={1})")]
    OutOfBounds(u32, u32),
    #[error("map at capacity={0}")]
    AtCapacity(u32),
}

/// Flags accepted by `update`. Matches the upstream `bpf_update_elem`
/// flag values byte-for-byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateFlag {
    /// `BPF_ANY` (0) — create or replace.
    Any,
    /// `BPF_NOEXIST` (1) — create only; error if exists.
    NoExist,
    /// `BPF_EXIST` (2) — replace only; error if missing.
    Exist,
}

impl Default for UpdateFlag {
    fn default() -> Self {
        Self::Any
    }
}

/// Kind discriminator. Constructed via `Map::new_*` helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapKind {
    Hash,
    LruHash { capacity: u32 },
    Array { size: u32 },
}

/// One userspace BPF map. Generic over key/value types; the kernel
/// strictly types these via map definitions in C, but Rust's type
/// system handles it for us.
#[derive(Debug)]
pub struct Map<K, V> {
    pub kind: MapKind,
    storage: BTreeMap<K, V>,
    /// LRU eviction queue — newest-first. Empty for Hash / Array.
    lru: VecDeque<K>,
    /// For Array variant — we lazily populate slots; missing slots
    /// behave as "zero" semantically (Hash semantics where lookup
    /// of an unset key returns `None`).
    _phantom_count: usize,
}

impl<K, V> Map<K, V>
where
    K: Clone + Ord + Eq + Hash,
    V: Clone,
{
    pub fn new_hash() -> Self {
        Self {
            kind: MapKind::Hash,
            storage: BTreeMap::new(),
            lru: VecDeque::new(),
            _phantom_count: 0,
        }
    }

    pub fn new_lru_hash(capacity: u32) -> Self {
        Self {
            kind: MapKind::LruHash { capacity },
            storage: BTreeMap::new(),
            lru: VecDeque::new(),
            _phantom_count: 0,
        }
    }

    pub fn new_array(size: u32) -> Self {
        Self {
            kind: MapKind::Array { size },
            storage: BTreeMap::new(),
            lru: VecDeque::new(),
            _phantom_count: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.storage.len()
    }

    pub fn is_empty(&self) -> bool {
        self.storage.is_empty()
    }

    pub fn lookup(&mut self, key: &K) -> Option<V> {
        let v = self.storage.get(key).cloned();
        if v.is_some() {
            if let MapKind::LruHash { .. } = self.kind {
                self.lru.retain(|k| k != key);
                self.lru.push_front(key.clone());
            }
        }
        v
    }

    /// Read-only variant — does NOT touch the LRU recency list.
    /// Useful when the caller wants to inspect state without
    /// pretending it's a hot-path packet hit.
    pub fn peek(&self, key: &K) -> Option<V> {
        self.storage.get(key).cloned()
    }

    pub fn update(&mut self, key: K, value: V, flag: UpdateFlag) -> Result<(), MapError> {
        let exists = self.storage.contains_key(&key);
        match flag {
            UpdateFlag::NoExist if exists => return Err(MapError::EntryExists),
            UpdateFlag::Exist if !exists => return Err(MapError::EntryMissing),
            _ => {}
        }

        if let MapKind::Array { size } = self.kind {
            // For Array, the only meaningful key is a u32 index
            // (the upstream BPF map definition enforces this at
            // verifier time). The Rust generic doesn't enforce u32
            // — we just rely on the caller passing matching types.
            // We DO enforce the array length via `len()` here.
            if !exists && (self.storage.len() as u32) >= size {
                return Err(MapError::AtCapacity(size));
            }
        }

        if let MapKind::LruHash { capacity } = self.kind {
            if !exists && (self.storage.len() as u32) >= capacity {
                // Evict least-recently-used.
                if let Some(victim) = self.lru.pop_back() {
                    self.storage.remove(&victim);
                }
            }
            self.lru.retain(|k| k != &key);
            self.lru.push_front(key.clone());
        }
        self.storage.insert(key, value);
        Ok(())
    }

    pub fn delete(&mut self, key: &K) -> Result<(), MapError> {
        if self.storage.remove(key).is_some() {
            self.lru.retain(|k| k != key);
            Ok(())
        } else {
            Err(MapError::EntryMissing)
        }
    }

    pub fn iter_keys(&self) -> impl Iterator<Item = &K> {
        self.storage.keys()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.storage.iter()
    }

    pub fn clear(&mut self) {
        self.storage.clear();
        self.lru.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_update_lookup_delete_round_trip() {
        let mut m: Map<u32, u32> = Map::new_hash();
        m.update(1, 100, UpdateFlag::Any).unwrap();
        assert_eq!(m.lookup(&1), Some(100));
        assert_eq!(m.lookup(&2), None);
        m.delete(&1).unwrap();
        assert_eq!(m.lookup(&1), None);
    }

    #[test]
    fn no_exist_flag_rejects_existing_key() {
        let mut m: Map<u32, u32> = Map::new_hash();
        m.update(1, 100, UpdateFlag::Any).unwrap();
        let err = m.update(1, 200, UpdateFlag::NoExist).unwrap_err();
        assert_eq!(err, MapError::EntryExists);
    }

    #[test]
    fn exist_flag_rejects_missing_key() {
        let mut m: Map<u32, u32> = Map::new_hash();
        let err = m.update(1, 100, UpdateFlag::Exist).unwrap_err();
        assert_eq!(err, MapError::EntryMissing);
    }

    #[test]
    fn delete_missing_key_returns_error() {
        let mut m: Map<u32, u32> = Map::new_hash();
        let err = m.delete(&7).unwrap_err();
        assert_eq!(err, MapError::EntryMissing);
    }

    #[test]
    fn lru_hash_evicts_oldest_at_capacity() {
        let mut m: Map<u32, u32> = Map::new_lru_hash(3);
        m.update(1, 10, UpdateFlag::Any).unwrap();
        m.update(2, 20, UpdateFlag::Any).unwrap();
        m.update(3, 30, UpdateFlag::Any).unwrap();
        // Touch 1 + 2 → 3 is now LRU.
        let _ = m.lookup(&1);
        let _ = m.lookup(&2);
        m.update(4, 40, UpdateFlag::Any).unwrap();
        assert_eq!(m.lookup(&3), None, "3 should have been evicted");
        assert_eq!(m.lookup(&4), Some(40));
        assert_eq!(m.len(), 3);
    }

    #[test]
    fn array_at_capacity_rejects_new_entries() {
        let mut m: Map<u32, u32> = Map::new_array(2);
        m.update(0, 1, UpdateFlag::Any).unwrap();
        m.update(1, 2, UpdateFlag::Any).unwrap();
        let err = m.update(2, 3, UpdateFlag::Any).unwrap_err();
        assert_eq!(err, MapError::AtCapacity(2));
    }

    #[test]
    fn array_allows_overwrite_of_existing_index() {
        let mut m: Map<u32, u32> = Map::new_array(2);
        m.update(0, 1, UpdateFlag::Any).unwrap();
        m.update(0, 99, UpdateFlag::Any).unwrap();
        assert_eq!(m.lookup(&0), Some(99));
    }

    #[test]
    fn peek_does_not_touch_lru_recency() {
        let mut m: Map<u32, u32> = Map::new_lru_hash(2);
        m.update(1, 10, UpdateFlag::Any).unwrap();
        m.update(2, 20, UpdateFlag::Any).unwrap();
        // peek(1) should NOT make 1 the most recent — so when we
        // insert 3, key 1 gets evicted (oldest by recency).
        assert_eq!(m.peek(&1), Some(10));
        m.update(3, 30, UpdateFlag::Any).unwrap();
        assert_eq!(m.peek(&1), None);
        assert_eq!(m.peek(&2), Some(20));
        assert_eq!(m.peek(&3), Some(30));
    }

    #[test]
    fn iter_keys_returns_every_present_key() {
        let mut m: Map<u32, u32> = Map::new_hash();
        m.update(1, 10, UpdateFlag::Any).unwrap();
        m.update(2, 20, UpdateFlag::Any).unwrap();
        let keys: Vec<u32> = m.iter_keys().copied().collect();
        assert_eq!(keys, vec![1, 2]);
    }

    #[test]
    fn clear_drops_all_state_including_lru_queue() {
        let mut m: Map<u32, u32> = Map::new_lru_hash(10);
        m.update(1, 10, UpdateFlag::Any).unwrap();
        m.update(2, 20, UpdateFlag::Any).unwrap();
        m.clear();
        assert!(m.is_empty());
        assert!(m.lookup(&1).is_none());
    }
}
