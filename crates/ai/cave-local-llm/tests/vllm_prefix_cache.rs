// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD port of vLLM's automatic prefix caching block allocator
// (vllm-project/vllm `vllm/core/block/prefix_caching_block.py`, Apache-2.0).
//
// The core idea: every *full* KV block is content-addressed by a chained hash
// of (parent-block-hash, token-ids). Two sequences that share a token prefix
// produce identical block hashes for the shared blocks, so the allocator hands
// back the *same* physical block (ref-counted) instead of recomputing its KV —
// the "cache hit". Freed cached blocks are not wiped; they linger in an LRU
// evictor and can be re-hit until physically reused for different content.
//
// These tests pin: content-hash determinism + chaining, cache hit/miss on
// re-allocation, prefix sharing across divergent sequences, ref-count
// lifecycle, freed-but-cached reuse, LRU eviction, out-of-blocks, hit-rate.

use cave_local_llm::vllm_prefix_cache::{
    compute_block_hashes, content_hash, PrefixCachingAllocator, PrefixError,
};

// ── content hashing ──────────────────────────────────────────────────────────

#[test]
fn content_hash_is_deterministic_and_token_sensitive() {
    assert_eq!(content_hash(None, &[1, 2, 3]), content_hash(None, &[1, 2, 3]));
    assert_ne!(content_hash(None, &[1, 2, 3]), content_hash(None, &[1, 2, 4]));
    // Same tokens, different parent prefix → different hash (chaining matters).
    let p = content_hash(None, &[9, 9]);
    assert_ne!(content_hash(None, &[1, 2]), content_hash(Some(p), &[1, 2]));
}

#[test]
fn compute_block_hashes_chains_full_blocks_only() {
    // 6 tokens, block_size 2 → three full blocks; partial tail excluded.
    let h = compute_block_hashes(&[1, 2, 3, 4, 5, 6], 2);
    assert_eq!(h.len(), 3);
    // Chained: block i's hash feeds block i+1.
    assert_eq!(h[0], content_hash(None, &[1, 2]));
    assert_eq!(h[1], content_hash(Some(h[0]), &[3, 4]));
    assert_eq!(h[2], content_hash(Some(h[1]), &[5, 6]));

    // A partial trailing block is not hashed.
    let h2 = compute_block_hashes(&[1, 2, 3], 2);
    assert_eq!(h2.len(), 1);
    assert_eq!(h2[0], content_hash(None, &[1, 2]));
}

#[test]
fn shared_prefix_diverges_at_first_differing_block() {
    let a = compute_block_hashes(&[1, 2, 3, 4, 5, 6], 2);
    let b = compute_block_hashes(&[1, 2, 3, 4, 9, 9], 2);
    assert_eq!(a[0], b[0]); // [1,2] shared
    assert_eq!(a[1], b[1]); // [3,4] shared
    assert_ne!(a[2], b[2]); // [5,6] vs [9,9] diverge
}

// ── allocation: hit / miss / ref counts ──────────────────────────────────────

#[test]
fn reallocating_identical_block_is_a_cache_hit() {
    let mut alloc = PrefixCachingAllocator::new(8, 2);
    let first = alloc.allocate_immutable(None, &[1, 2]).unwrap();
    assert!(!first.cache_hit);
    assert_eq!(alloc.ref_count(first.block_id), 1);

    let second = alloc.allocate_immutable(None, &[1, 2]).unwrap();
    assert!(second.cache_hit);
    assert_eq!(second.block_id, first.block_id); // same physical block
    assert_eq!(alloc.ref_count(first.block_id), 2); // ref-counted
    assert_eq!(alloc.num_cached_blocks(), 1);
}

#[test]
fn distinct_content_takes_distinct_blocks() {
    let mut alloc = PrefixCachingAllocator::new(8, 2);
    let a = alloc.allocate_immutable(None, &[1, 2]).unwrap();
    let b = alloc.allocate_immutable(None, &[3, 4]).unwrap();
    assert_ne!(a.block_id, b.block_id);
    assert!(!a.cache_hit && !b.cache_hit);
    assert_eq!(alloc.num_cached_blocks(), 2);
}

#[test]
fn prefix_sharing_across_sequences_reuses_blocks() {
    let mut alloc = PrefixCachingAllocator::new(8, 2);
    // Sequence 1: tokens [1,2,3,4] → two chained blocks.
    let hashes1 = compute_block_hashes(&[1, 2, 3, 4], 2);
    let s1b0 = alloc.allocate_immutable(None, &[1, 2]).unwrap();
    let s1b1 = alloc.allocate_immutable(Some(hashes1[0]), &[3, 4]).unwrap();
    assert!(!s1b0.cache_hit && !s1b1.cache_hit);

    // Sequence 2: tokens [1,2,9,9] → shares only the first block.
    let s2b0 = alloc.allocate_immutable(None, &[1, 2]).unwrap();
    let s2b1 = alloc.allocate_immutable(Some(hashes1[0]), &[9, 9]).unwrap();
    assert!(s2b0.cache_hit);
    assert_eq!(s2b0.block_id, s1b0.block_id); // shared prefix block
    assert!(!s2b1.cache_hit);
    assert_ne!(s2b1.block_id, s1b1.block_id); // divergent block
}

// ── free / cached reuse / eviction ───────────────────────────────────────────

#[test]
fn freeing_to_zero_refs_keeps_block_cached_for_reuse() {
    let mut alloc = PrefixCachingAllocator::new(4, 2);
    let a = alloc.allocate_immutable(None, &[1, 2]).unwrap();
    let free_after_alloc = alloc.num_free_blocks();
    alloc.free(a.block_id);
    assert_eq!(alloc.ref_count(a.block_id), 0);
    // Block is freed but still resident in the cache.
    assert_eq!(alloc.num_cached_blocks(), 1);
    assert_eq!(alloc.num_free_blocks(), free_after_alloc + 1);

    // Re-allocating the same content hits the *same* physical block and does
    // not consume a fresh physical block.
    let again = alloc.allocate_immutable(None, &[1, 2]).unwrap();
    assert!(again.cache_hit);
    assert_eq!(again.block_id, a.block_id);
    assert_eq!(alloc.ref_count(a.block_id), 1);
}

#[test]
fn ref_counted_block_freed_twice_reaches_zero() {
    let mut alloc = PrefixCachingAllocator::new(4, 2);
    let a = alloc.allocate_immutable(None, &[1, 2]).unwrap();
    let _b = alloc.allocate_immutable(None, &[1, 2]).unwrap(); // ref 2
    assert_eq!(alloc.ref_count(a.block_id), 2);
    alloc.free(a.block_id);
    assert_eq!(alloc.ref_count(a.block_id), 1); // still referenced
    assert_eq!(alloc.num_free_blocks(), 3); // not yet reclaimable
    alloc.free(a.block_id);
    assert_eq!(alloc.ref_count(a.block_id), 0);
}

#[test]
fn lru_evicts_least_recently_freed_cached_block() {
    let mut alloc = PrefixCachingAllocator::new(2, 2);
    let a = alloc.allocate_immutable(None, &[1, 1]).unwrap(); // block for content A
    let b = alloc.allocate_immutable(None, &[2, 2]).unwrap(); // content B
    alloc.free(a.block_id); // A freed first → LRU
    alloc.free(b.block_id); // B freed second
    assert_eq!(alloc.num_cached_blocks(), 2);

    // New distinct content must evict the LRU (A), reusing its physical block.
    let c = alloc.allocate_immutable(None, &[3, 3]).unwrap();
    assert!(!c.cache_hit);
    assert_eq!(c.block_id, a.block_id); // reused A's physical block

    // A's content is gone from the cache now; re-requesting it is a miss.
    let a_again = alloc.allocate_immutable(None, &[1, 1]);
    assert!(a_again.is_err() || !a_again.as_ref().unwrap().cache_hit);

    // B was untouched and is still a hit.
    let b_again = alloc.allocate_immutable(None, &[2, 2]).unwrap();
    assert!(b_again.cache_hit);
    assert_eq!(b_again.block_id, b.block_id);
}

#[test]
fn out_of_blocks_when_nothing_is_evictable() {
    let mut alloc = PrefixCachingAllocator::new(1, 2);
    let _a = alloc.allocate_immutable(None, &[1, 2]).unwrap(); // ref 1, pinned
    let err = alloc.allocate_immutable(None, &[3, 4]);
    assert_eq!(err.unwrap_err(), PrefixError::OutOfBlocks);
}

// ── hit-rate stats ───────────────────────────────────────────────────────────

#[test]
fn hit_rate_tracks_queries_and_hits() {
    let mut alloc = PrefixCachingAllocator::new(8, 2);
    alloc.allocate_immutable(None, &[1, 2]).unwrap(); // miss
    alloc.allocate_immutable(None, &[1, 2]).unwrap(); // hit
    alloc.allocate_immutable(None, &[1, 2]).unwrap(); // hit
    assert_eq!(alloc.cache_queries(), 3);
    assert_eq!(alloc.cache_hits(), 2);
    assert!((alloc.hit_rate() - 2.0 / 3.0).abs() < 1e-9);
}
