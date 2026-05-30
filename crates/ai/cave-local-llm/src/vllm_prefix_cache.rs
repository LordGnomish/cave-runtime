// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD port of vLLM's automatic prefix caching block allocator
// (vllm-project/vllm `vllm/core/block/prefix_caching_block.py`, Apache-2.0).
//
// Every *full* KV block is content-addressed by a chained hash of its parent
// block's hash and its own token ids. Sequences that share a token prefix
// therefore produce identical block hashes for the shared region, and the
// allocator returns the *same* physical block (ref-counted) — vLLM's prefix
// "cache hit", which lets a new request skip recomputing KV for tokens an
// earlier request already attended to.
//
// Freed blocks are not wiped: a block whose ref-count hits zero is moved to an
// LRU evictor while keeping its content-hash mapping, so an identical prefix
// arriving later still hits it. Only when the pool is exhausted and a *miss*
// needs a physical block does the least-recently-freed cached block get evicted
// (its hash mapping dropped) and its physical block recycled.
//
// This is the pure block-bookkeeping layer. Actual KV tensors live on the
// accelerator and are out of scope for the in-process runtime (the host owns
// the device); this allocator models the sharing/eviction policy that decides
// which physical blocks back which logical blocks.

use std::collections::{HashMap, VecDeque};

/// FNV-1a 64-bit offset basis — the seed used for a first (parentless) block.
pub const NONE_HASH_SEED: u64 = 0xcbf2_9ce4_8422_2325;

const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

#[inline]
fn fnv_mix(mut hash: u64, byte: u8) -> u64 {
    hash ^= byte as u64;
    hash.wrapping_mul(FNV_PRIME)
}

/// Content hash of one block: a chained FNV-1a over the parent block's hash
/// (or [`NONE_HASH_SEED`] for the first block) followed by the block's tokens.
///
/// Chaining the parent hash makes the value position-dependent: identical
/// tokens at different prefix depths hash differently, exactly as vLLM's
/// `PrefixCachingBlock.content_hash` requires for correct prefix matching.
pub fn content_hash(parent: Option<u64>, tokens: &[u32]) -> u64 {
    let mut h = NONE_HASH_SEED;
    // Fold the parent hash in first so the chain is order-sensitive.
    let parent = parent.unwrap_or(NONE_HASH_SEED);
    for b in parent.to_le_bytes() {
        h = fnv_mix(h, b);
    }
    for &t in tokens {
        for b in t.to_le_bytes() {
            h = fnv_mix(h, b);
        }
    }
    h
}

/// Chain content hashes for every *full* block of `tokens`, left to right.
///
/// A trailing partial block (fewer than `block_size` tokens) is not hashable —
/// it is still mutable in vLLM — and is omitted from the result.
pub fn compute_block_hashes(tokens: &[u32], block_size: usize) -> Vec<u64> {
    assert!(block_size > 0, "block_size must be positive");
    let mut hashes = Vec::with_capacity(tokens.len() / block_size);
    let mut parent: Option<u64> = None;
    for chunk in tokens.chunks(block_size) {
        if chunk.len() < block_size {
            break; // partial trailing block stays mutable
        }
        let h = content_hash(parent, chunk);
        hashes.push(h);
        parent = Some(h);
    }
    hashes
}

/// Errors from the prefix-caching allocator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefixError {
    /// No physical block is available and none can be evicted (all pinned).
    OutOfBlocks,
}

impl std::fmt::Display for PrefixError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PrefixError::OutOfBlocks => f.write_str("no free or evictable KV blocks"),
        }
    }
}

impl std::error::Error for PrefixError {}

/// The result of allocating an immutable (full) block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Allocation {
    /// The physical block id backing this content.
    pub block_id: usize,
    /// The block's content hash.
    pub content_hash: u64,
    /// `true` when the content was already resident (KV recompute avoided).
    pub cache_hit: bool,
}

/// Content-addressed, ref-counted, LRU-evicting block allocator.
#[derive(Debug)]
pub struct PrefixCachingAllocator {
    block_size: usize,
    num_total: usize,
    /// Per-physical-block reference count.
    refcounts: Vec<usize>,
    /// Resident content hash per physical block (`None` if the block is blank).
    resident: Vec<Option<u64>>,
    /// content-hash → physical block, for every cached block (ref>0 or freed).
    hash_to_block: HashMap<u64, usize>,
    /// Never-used physical blocks (no resident content).
    blank: Vec<usize>,
    /// Zero-ref but still-cached blocks, least-recently-freed at the front.
    evictor: VecDeque<usize>,
    hits: u64,
    queries: u64,
}

impl PrefixCachingAllocator {
    /// Build an allocator over `num_blocks` physical blocks of `block_size`.
    pub fn new(num_blocks: usize, block_size: usize) -> Self {
        assert!(block_size > 0, "block_size must be positive");
        Self {
            block_size,
            num_total: num_blocks,
            refcounts: vec![0; num_blocks],
            resident: vec![None; num_blocks],
            hash_to_block: HashMap::new(),
            blank: (0..num_blocks).rev().collect(), // pop low ids first
            evictor: VecDeque::new(),
            hits: 0,
            queries: 0,
        }
    }

    /// Tokens per block.
    pub fn block_size(&self) -> usize {
        self.block_size
    }

    /// Total physical blocks in the pool.
    pub fn num_total_blocks(&self) -> usize {
        self.num_total
    }

    /// Blocks that can be handed out without unref-ing a pinned block:
    /// blank blocks plus zero-ref cached blocks in the evictor.
    pub fn num_free_blocks(&self) -> usize {
        self.blank.len() + self.evictor.len()
    }

    /// Distinct content hashes currently resident (pinned or cached-free).
    pub fn num_cached_blocks(&self) -> usize {
        self.hash_to_block.len()
    }

    /// Reference count of a physical block.
    pub fn ref_count(&self, block_id: usize) -> usize {
        self.refcounts[block_id]
    }

    /// Read-only prefix-cache probe: whether the block for `tokens` chained
    /// under `parent` is currently resident (would be a hit). Does not mutate
    /// ref-counts or eviction order — vLLM's `get_prefix_cache_hit` lookup.
    pub fn is_cached(&self, parent: Option<u64>, tokens: &[u32]) -> bool {
        self.hash_to_block
            .contains_key(&content_hash(parent, tokens))
    }

    /// Total allocation queries seen.
    pub fn cache_queries(&self) -> u64 {
        self.queries
    }

    /// Allocation queries that hit a resident block.
    pub fn cache_hits(&self) -> u64 {
        self.hits
    }

    /// Fraction of queries served from cache (0.0 when no queries yet).
    pub fn hit_rate(&self) -> f64 {
        if self.queries == 0 {
            0.0
        } else {
            self.hits as f64 / self.queries as f64
        }
    }

    /// Allocate the immutable (full) block holding `tokens`, chained under
    /// `parent` (the previous block's content hash, or `None` for the first).
    ///
    /// On a cache hit the resident block's ref-count is incremented; on a miss
    /// a blank or evicted physical block is claimed.
    pub fn allocate_immutable(
        &mut self,
        parent: Option<u64>,
        tokens: &[u32],
    ) -> Result<Allocation, PrefixError> {
        self.queries += 1;
        let h = content_hash(parent, tokens);

        if let Some(&bid) = self.hash_to_block.get(&h) {
            // Cache hit. If it was sitting in the evictor, pin it again.
            if self.refcounts[bid] == 0 {
                self.remove_from_evictor(bid);
            }
            self.refcounts[bid] += 1;
            self.hits += 1;
            return Ok(Allocation {
                block_id: bid,
                content_hash: h,
                cache_hit: true,
            });
        }

        // Miss: claim a physical block (blank first, else evict LRU).
        let bid = self.claim_block()?;
        if let Some(old) = self.resident[bid].take() {
            self.hash_to_block.remove(&old);
        }
        self.resident[bid] = Some(h);
        self.hash_to_block.insert(h, bid);
        self.refcounts[bid] = 1;
        Ok(Allocation {
            block_id: bid,
            content_hash: h,
            cache_hit: false,
        })
    }

    /// Release one reference to `block_id`. When it reaches zero the block is
    /// kept cached (moved to the LRU evictor), not wiped.
    pub fn free(&mut self, block_id: usize) {
        let rc = &mut self.refcounts[block_id];
        if *rc == 0 {
            return;
        }
        *rc -= 1;
        if *rc == 0 {
            self.evictor.push_back(block_id);
        }
    }

    /// Pop a usable physical block: a blank one if any, otherwise evict the
    /// least-recently-freed cached block.
    fn claim_block(&mut self) -> Result<usize, PrefixError> {
        if let Some(bid) = self.blank.pop() {
            return Ok(bid);
        }
        self.evictor.pop_front().ok_or(PrefixError::OutOfBlocks)
    }

    /// Remove `block_id` from the evictor queue (it was just re-pinned).
    fn remove_from_evictor(&mut self, block_id: usize) {
        if let Some(pos) = self.evictor.iter().position(|&b| b == block_id) {
            self.evictor.remove(pos);
        }
    }
}
