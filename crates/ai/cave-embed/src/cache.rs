// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Embedding result cache (LRU memoization).
//!
//! Re-indexing pipelines embed the same text repeatedly; memoizing the result
//! avoids re-running the model. The cache keys on `(model, dimensions, input)`,
//! evicts the least-recently-used entry once `capacity` is exceeded, and tracks
//! hit/miss counts. It uses interior mutability (a single mutex) so a shared
//! `&EmbeddingCache` can serve concurrent requests.

use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

struct Entry {
    value: Vec<f32>,
    // Monotonic recency stamp; higher = more recently used.
    last_used: u64,
}

struct Inner {
    map: HashMap<String, Entry>,
    clock: u64,
    hits: u64,
    misses: u64,
}

/// A capacity-bounded LRU cache of embedding vectors.
pub struct EmbeddingCache {
    capacity: usize,
    inner: Mutex<Inner>,
}

impl EmbeddingCache {
    /// Create a cache holding at most `capacity` entries. `capacity == 0`
    /// disables caching entirely.
    pub fn new(capacity: usize) -> Self {
        EmbeddingCache {
            capacity,
            inner: Mutex::new(Inner {
                map: HashMap::new(),
                clock: 0,
                hits: 0,
                misses: 0,
            }),
        }
    }

    /// Build a stable cache key from the request shape that affects the output.
    pub fn key(model: &str, dimensions: Option<usize>, input: &str) -> String {
        let mut h = Sha256::new();
        h.update(model.as_bytes());
        h.update([0u8]);
        match dimensions {
            Some(d) => {
                h.update(b"d");
                h.update(d.to_le_bytes());
            }
            None => h.update(b"n"),
        }
        h.update([0u8]);
        h.update(input.as_bytes());
        hex::encode(h.finalize())
    }

    /// Fetch a cached vector, updating recency and hit/miss counters.
    pub fn get(&self, key: &str) -> Option<Vec<f32>> {
        if self.capacity == 0 {
            return None;
        }
        let mut g = self.inner.lock();
        g.clock += 1;
        let now = g.clock;
        if let Some(e) = g.map.get_mut(key) {
            e.last_used = now;
            let v = e.value.clone();
            g.hits += 1;
            Some(v)
        } else {
            g.misses += 1;
            None
        }
    }

    /// Insert a vector, evicting the least-recently-used entry if over capacity.
    pub fn put(&self, key: String, value: Vec<f32>) {
        if self.capacity == 0 {
            return;
        }
        let mut g = self.inner.lock();
        g.clock += 1;
        let now = g.clock;
        g.map.insert(key, Entry { value, last_used: now });
        while g.map.len() > self.capacity {
            // Find and remove the entry with the smallest last_used stamp.
            if let Some(victim) = g
                .map
                .iter()
                .min_by_key(|(_, e)| e.last_used)
                .map(|(k, _)| k.clone())
            {
                g.map.remove(&victim);
            } else {
                break;
            }
        }
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.inner.lock().map.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `(hits, misses)` since construction.
    pub fn stats(&self) -> (u64, u64) {
        let g = self.inner.lock();
        (g.hits, g.misses)
    }
}
