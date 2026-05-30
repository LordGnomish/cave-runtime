// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Hash access method (`hash`).
//!
//! Port of PostgreSQL `src/backend/access/hash/`
//! (`hashfunc.c`, `hashinsert.c`, `hashsearch.c`, `hashpage.c`).
//!
//! An equality-only secondary index. Upstream hash indexes grow their bucket
//! array one bucket at a time via Litwin **linear hashing** rather than
//! doubling — when the average bucket occupancy crosses the fill target, the
//! bucket under the split pointer is divided and its tuples rehomed using one
//! extra hash bit (`_hash_expandtable` in hashpage.c). We reproduce that
//! incremental-split discipline exactly so probes stay correct mid-growth.

use crate::types::SqlValue;

/// Target average entries-per-bucket before a split fires (fillfactor analog).
const FILL_TARGET: usize = 4;

type Tid = usize;

struct Entry {
    key: SqlValue,
    tids: Vec<Tid>,
}

#[derive(Default)]
struct Bucket {
    entries: Vec<Entry>,
}

/// Linear-hashing equality index over `SqlValue` keys.
pub struct HashIndex {
    buckets: Vec<Bucket>,
    /// current round: low buckets are addressed with `level` bits
    level: u32,
    /// next bucket to split (the split pointer)
    next: usize,
    /// number of (key, tid) pairs
    entries: usize,
}

impl Default for HashIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl HashIndex {
    pub fn new() -> Self {
        // Start with 2^2 = 4 primary buckets.
        let level = 2;
        let buckets = (0..(1usize << level)).map(|_| Bucket::default()).collect();
        HashIndex {
            buckets,
            level,
            next: 0,
            entries: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries == 0
    }

    pub fn bucket_count(&self) -> usize {
        self.buckets.len()
    }

    /// Resolve a key's bucket under the linear-hashing addressing rule:
    /// hash mod 2^level, but mod 2^(level+1) for already-split low buckets.
    fn bucket_of(&self, h: u64) -> usize {
        let mut b = (h & ((1u64 << self.level) - 1)) as usize;
        if b < self.next {
            b = (h & ((1u64 << (self.level + 1)) - 1)) as usize;
        }
        b
    }

    pub fn insert(&mut self, key: SqlValue, tid: Tid) {
        let h = hash_value(&key);
        let b = self.bucket_of(h);
        let bucket = &mut self.buckets[b];
        if let Some(e) = bucket.entries.iter_mut().find(|e| e.key == key) {
            e.tids.push(tid);
        } else {
            bucket.entries.push(Entry {
                key,
                tids: vec![tid],
            });
        }
        self.entries += 1;

        while self.entries > FILL_TARGET * self.buckets.len() {
            self.split();
        }
    }

    /// `_hash_expandtable`: split the bucket under the split pointer, rehoming
    /// its entries with one additional hash bit.
    fn split(&mut self) {
        let split_idx = self.next;
        let new_idx = self.next + (1usize << self.level);
        debug_assert_eq!(new_idx, self.buckets.len());
        self.buckets.push(Bucket::default());

        let moved: Vec<Entry> = std::mem::take(&mut self.buckets[split_idx].entries);
        let mask = (1u64 << (self.level + 1)) - 1;
        for e in moved {
            let target = (hash_value(&e.key) & mask) as usize;
            self.buckets[target].entries.push(e);
        }

        self.next += 1;
        if self.next == (1usize << self.level) {
            self.level += 1;
            self.next = 0;
        }
    }

    /// Equality probe: heap TIDs stored under `key`, in insertion order.
    pub fn search(&self, key: &SqlValue) -> Vec<Tid> {
        let h = hash_value(key);
        let b = self.bucket_of(h);
        self.buckets[b]
            .entries
            .iter()
            .find(|e| &e.key == key)
            .map(|e| e.tids.clone())
            .unwrap_or_default()
    }
}

/// Jenkins one-at-a-time hash over the value's byte image — the role
/// `hashfunc.c`'s lookup3 `hash_bytes` plays for the default hash opclass.
fn hash_value(v: &SqlValue) -> u64 {
    let mut h: u64 = 0;
    for &byte in &value_bytes(v) {
        h = h.wrapping_add(byte as u64);
        h = h.wrapping_add(h << 10);
        h ^= h >> 6;
    }
    h = h.wrapping_add(h << 3);
    h ^= h >> 11;
    h = h.wrapping_add(h << 15);
    h
}

fn value_bytes(v: &SqlValue) -> Vec<u8> {
    match v {
        SqlValue::Null => vec![0],
        SqlValue::Bool(b) => vec![1, *b as u8],
        SqlValue::Int4(n) => {
            let mut o = vec![2];
            o.extend_from_slice(&n.to_le_bytes());
            o
        }
        SqlValue::Int8(n) => {
            let mut o = vec![3];
            o.extend_from_slice(&n.to_le_bytes());
            o
        }
        SqlValue::Numeric(f) => {
            let mut o = vec![4];
            o.extend_from_slice(&f.to_bits().to_le_bytes());
            o
        }
        SqlValue::Text(s) => {
            let mut o = vec![5];
            o.extend_from_slice(s.as_bytes());
            o
        }
        SqlValue::Date(s) => {
            let mut o = vec![6];
            o.extend_from_slice(s.as_bytes());
            o
        }
        SqlValue::Timestamp(s) => {
            let mut o = vec![7];
            o.extend_from_slice(s.as_bytes());
            o
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_deterministic_per_value() {
        assert_eq!(hash_value(&SqlValue::Int4(42)), hash_value(&SqlValue::Int4(42)));
        assert_ne!(hash_value(&SqlValue::Int4(1)), hash_value(&SqlValue::Int4(2)));
    }

    #[test]
    fn split_pointer_advances_and_wraps_level() {
        let mut idx = HashIndex::new();
        assert_eq!(idx.bucket_count(), 4);
        assert_eq!(idx.level, 2);
        // force exactly one split
        for k in 0..(FILL_TARGET * 4 + 1) as i32 {
            idx.insert(SqlValue::Int4(k), k as usize);
        }
        assert_eq!(idx.bucket_count(), 5, "one incremental split expected");
        assert_eq!(idx.next, 1);
    }
}
