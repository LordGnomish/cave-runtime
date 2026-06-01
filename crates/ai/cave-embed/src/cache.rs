// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! LRU embedding cache.
//!
//! infinity can serve repeated inputs from a cache instead of re-running the
//! model. We key on `sha256(model \0 text)` so identical (model, input) pairs
//! hit, and evict least-recently-used entries when capacity is exceeded.

use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Cache key for a (model, text) pair: hex sha256.
pub fn cache_key(model: &str, text: &str) -> String {
    let mut h = Sha256::new();
    h.update(model.as_bytes());
    h.update([0u8]);
    h.update(text.as_bytes());
    format!("{:x}", h.finalize())
}

struct Entry {
    value: Vec<f32>,
    tick: u64,
}

/// Bounded LRU cache of embedding vectors.
pub struct EmbeddingCache {
    capacity: usize,
    map: HashMap<String, Entry>,
    clock: u64,
    hits: u64,
    misses: u64,
}

impl EmbeddingCache {
    /// Create a cache with the given maximum entry count (min 1).
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            map: HashMap::new(),
            clock: 0,
            hits: 0,
            misses: 0,
        }
    }

    /// Look up an embedding, updating recency + hit/miss stats.
    pub fn get(&mut self, model: &str, text: &str) -> Option<Vec<f32>> {
        let key = cache_key(model, text);
        self.clock += 1;
        let tick = self.clock;
        match self.map.get_mut(&key) {
            Some(entry) => {
                entry.tick = tick;
                self.hits += 1;
                Some(entry.value.clone())
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }

    /// Insert an embedding, evicting the LRU entry if over capacity.
    pub fn put(&mut self, model: &str, text: &str, value: Vec<f32>) {
        let key = cache_key(model, text);
        self.clock += 1;
        let tick = self.clock;
        if let Some(entry) = self.map.get_mut(&key) {
            entry.value = value;
            entry.tick = tick;
            return;
        }
        if self.map.len() >= self.capacity {
            if let Some(lru) = self
                .map
                .iter()
                .min_by_key(|(_, e)| e.tick)
                .map(|(k, _)| k.clone())
            {
                self.map.remove(&lru);
            }
        }
        self.map.insert(key, Entry { value, tick });
    }

    /// Current entry count.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether empty.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Configured capacity.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// (hits, misses).
    pub fn stats(&self) -> (u64, u64) {
        (self.hits, self.misses)
    }

    /// Drop all entries (stats retained).
    pub fn clear(&mut self) {
        self.map.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_separates_model_and_text() {
        assert_ne!(cache_key("m1", "x"), cache_key("m2", "x"));
        assert_ne!(cache_key("m", "a"), cache_key("m", "b"));
        assert_eq!(cache_key("m", "a"), cache_key("m", "a"));
    }

    #[test]
    fn miss_then_put_then_hit() {
        let mut c = EmbeddingCache::new(4);
        assert!(c.get("m", "hello").is_none());
        c.put("m", "hello", vec![1.0, 2.0]);
        assert_eq!(c.get("m", "hello"), Some(vec![1.0, 2.0]));
        assert_eq!(c.stats(), (1, 1));
    }

    #[test]
    fn evicts_least_recently_used() {
        let mut c = EmbeddingCache::new(2);
        c.put("m", "a", vec![1.0]);
        c.put("m", "b", vec![2.0]);
        c.put("m", "c", vec![3.0]); // evicts "a"
        assert_eq!(c.len(), 2);
        assert!(c.get("m", "a").is_none());
        assert_eq!(c.get("m", "c"), Some(vec![3.0]));
    }

    #[test]
    fn get_refreshes_recency() {
        let mut c = EmbeddingCache::new(2);
        c.put("m", "a", vec![1.0]);
        c.put("m", "b", vec![2.0]);
        assert_eq!(c.get("m", "a"), Some(vec![1.0])); // a now most-recent
        c.put("m", "c", vec![3.0]); // should evict "b", not "a"
        assert_eq!(c.get("m", "a"), Some(vec![1.0]));
        assert!(c.get("m", "b").is_none());
    }

    #[test]
    fn put_same_key_updates_in_place() {
        let mut c = EmbeddingCache::new(2);
        c.put("m", "a", vec![1.0]);
        c.put("m", "a", vec![9.0]);
        assert_eq!(c.len(), 1);
        assert_eq!(c.get("m", "a"), Some(vec![9.0]));
    }

    #[test]
    fn clear_empties() {
        let mut c = EmbeddingCache::new(2);
        c.put("m", "a", vec![1.0]);
        c.clear();
        assert!(c.is_empty());
    }
}
