// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Memory eviction policies for cave-cache.

use std::time::Instant;

use crate::config::EvictionPolicy;
use crate::db::Db;
use crate::types::Entry;

/// Try to evict keys if maxmemory is exceeded.
/// Returns the number of keys evicted.
pub fn evict_if_needed(db: &mut Db, policy: EvictionPolicy, max_memory: Option<usize>) -> usize {
    let Some(max) = max_memory else {
        return 0;
    };

    let current = estimate_memory(db);
    if current <= max {
        return 0;
    }

    let to_evict = (current - max) / 64 + 1; // rough estimate of keys to evict
    let evicted = evict_keys(db, policy, to_evict);
    evicted
}

pub fn evict_keys(db: &mut Db, policy: EvictionPolicy, count: usize) -> usize {
    match policy {
        EvictionPolicy::NoEviction => 0,
        EvictionPolicy::AllKeysRandom => evict_random(db, count, false),
        EvictionPolicy::VolatileRandom => evict_random(db, count, true),
        EvictionPolicy::AllKeysLru => evict_lru(db, count, false),
        EvictionPolicy::VolatileLru => evict_lru(db, count, true),
        EvictionPolicy::AllKeysLfu => evict_lfu(db, count, false),
        EvictionPolicy::VolatileLfu => evict_lfu(db, count, true),
        EvictionPolicy::VolatileTtl => evict_ttl(db, count),
    }
}

fn evict_random(db: &mut Db, count: usize, volatile_only: bool) -> usize {
    let candidates: Vec<Vec<u8>> = db
        .keys
        .iter()
        .filter(|(_, e)| !volatile_only || e.expires_at.is_some())
        .map(|(k, _)| k.clone())
        .take(count * 5)
        .collect();

    let to_remove: Vec<Vec<u8>> = candidates.into_iter().take(count).collect();
    let evicted = to_remove.len();
    for k in to_remove {
        db.keys.remove(&k);
    }
    evicted
}

fn evict_lru(db: &mut Db, count: usize, volatile_only: bool) -> usize {
    let mut candidates: Vec<(Vec<u8>, u64)> = db
        .keys
        .iter()
        .filter(|(_, e)| !volatile_only || e.expires_at.is_some())
        .map(|(k, e)| (k.clone(), e.lru_clock))
        .collect();

    candidates.sort_by_key(|(_, clock)| *clock); // oldest first
    let evicted = candidates.len().min(count);

    for (k, _) in &candidates[..evicted] {
        db.keys.remove(k);
    }
    evicted
}

fn evict_lfu(db: &mut Db, count: usize, volatile_only: bool) -> usize {
    let mut candidates: Vec<(Vec<u8>, u8)> = db
        .keys
        .iter()
        .filter(|(_, e)| !volatile_only || e.expires_at.is_some())
        .map(|(k, e)| (k.clone(), e.lfu_freq))
        .collect();

    candidates.sort_by_key(|(_, freq)| *freq); // lowest freq first
    let evicted = candidates.len().min(count);

    for (k, _) in &candidates[..evicted] {
        db.keys.remove(k);
    }
    evicted
}

fn evict_ttl(db: &mut Db, count: usize) -> usize {
    let now = Instant::now();
    let mut candidates: Vec<(Vec<u8>, u64)> = db
        .keys
        .iter()
        .filter_map(|(k, e)| {
            e.expires_at.map(|t| {
                let remaining = if t > now {
                    (t - now).as_millis() as u64
                } else {
                    0
                };
                (k.clone(), remaining)
            })
        })
        .collect();

    candidates.sort_by_key(|(_, ttl)| *ttl); // soonest expiry first
    let evicted = candidates.len().min(count);

    for (k, _) in &candidates[..evicted] {
        db.keys.remove(k);
    }
    evicted
}

/// Rough memory estimate for a database (bytes).
pub fn estimate_memory(db: &Db) -> usize {
    db.keys.len() * 256 // very rough: 256 bytes per key average
}
