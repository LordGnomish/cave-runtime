// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! PagedAttention block manager — a pure-Rust port of vLLM's
//! `BlockSpaceManager` + `BlockAllocator`
//! (vllm-project/vllm `vllm/core/block_manager.py`, Apache-2.0).
//!
//! PagedAttention stores a sequence's KV cache in fixed-size **blocks**
//! (`block_size` tokens each) instead of one contiguous region. A block
//! table maps a sequence's logical blocks to physical block numbers, so
//! sequences can share prefix blocks (prompt caching / beam fork) via
//! reference counting and only diverge on write (copy-on-write).
//!
//! This module ports the bookkeeping — allocation, ref-counting,
//! copy-on-write growth, fork, free, GPU<->CPU swap and watermark-gated
//! admission — without touching tensor data or attention kernels (those
//! belong to a GPU runtime, out of scope for the sovereign control plane).

use std::collections::HashMap;

use thiserror::Error;

/// Where a sequence's KV blocks currently live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Device {
    /// Resident in GPU (fast) block pool.
    Gpu,
    /// Swapped out to the CPU (slow) block pool.
    Cpu,
}

/// Result of a [`BlockSpaceManager::can_allocate`] admission check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocStatus {
    /// Enough free blocks above the watermark — admit now.
    Ok,
    /// Fits in total capacity but not right now — retry after eviction.
    Later,
    /// Request can never fit in the GPU pool — reject permanently.
    Never,
}

/// Failures from block bookkeeping.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum BlockError {
    /// `allocate` called for a sequence that already holds a block table.
    #[error("sequence {0} already has an allocated block table")]
    AlreadyAllocated(u64),
    /// Operation referenced a sequence with no block table.
    #[error("sequence {0} has no allocated block table")]
    NotAllocated(u64),
    /// Not enough free physical blocks to satisfy the request.
    #[error("out of block memory: requested {requested}, available {available}")]
    OutOfMemory {
        /// Blocks the operation needed.
        requested: usize,
        /// Blocks actually free in the target pool.
        available: usize,
    },
    /// Swap requested against the wrong device pool.
    #[error("sequence {seq} is on {actual:?}, not {expected:?}")]
    WrongDevice {
        /// Offending sequence id.
        seq: u64,
        /// Device the caller assumed.
        expected: Device,
        /// Device the sequence is actually on.
        actual: Device,
    },
}

/// A fixed-size pool of physical KV blocks with reference counting.
#[derive(Debug)]
struct BlockAllocator {
    num_blocks: usize,
    free: Vec<usize>,
    ref_count: Vec<usize>,
}

impl BlockAllocator {
    fn new(num_blocks: usize) -> Self {
        // Pop order ascending (0,1,2,…) for deterministic block numbers.
        let free: Vec<usize> = (0..num_blocks).rev().collect();
        Self {
            num_blocks,
            free,
            ref_count: vec![0; num_blocks],
        }
    }

    fn num_free(&self) -> usize {
        self.free.len()
    }

    fn allocate(&mut self) -> Option<usize> {
        let b = self.free.pop()?;
        self.ref_count[b] = 1;
        Some(b)
    }

    fn fork(&mut self, block: usize) {
        self.ref_count[block] += 1;
    }

    /// Decrement a block's ref-count; return true if it became physically free.
    fn free(&mut self, block: usize) -> bool {
        debug_assert!(self.ref_count[block] > 0, "double free of block {block}");
        self.ref_count[block] -= 1;
        if self.ref_count[block] == 0 {
            self.free.push(block);
            true
        } else {
            false
        }
    }

    fn ref_count(&self, block: usize) -> usize {
        self.ref_count[block]
    }
}

/// Per-sequence block table plus the device it currently resides on.
#[derive(Debug, Clone)]
struct SeqBlocks {
    blocks: Vec<usize>,
    device: Device,
}

/// Manages KV block tables for all live sequences across GPU + CPU pools.
#[derive(Debug)]
pub struct BlockSpaceManager {
    block_size: usize,
    watermark_blocks: usize,
    gpu: BlockAllocator,
    cpu: BlockAllocator,
    tables: HashMap<u64, SeqBlocks>,
}

impl BlockSpaceManager {
    /// Build a manager with `num_gpu_blocks` GPU and `num_cpu_blocks` CPU
    /// physical blocks of `block_size` tokens each. `watermark` (0.0..=1.0)
    /// reserves `watermark * num_gpu_blocks` blocks as headroom that
    /// admission must leave free.
    pub fn new(
        block_size: usize,
        num_gpu_blocks: usize,
        num_cpu_blocks: usize,
        watermark: f64,
    ) -> Self {
        let watermark_blocks = (watermark * num_gpu_blocks as f64) as usize;
        Self {
            block_size,
            watermark_blocks,
            gpu: BlockAllocator::new(num_gpu_blocks),
            cpu: BlockAllocator::new(num_cpu_blocks),
            tables: HashMap::new(),
        }
    }

    /// Tokens per physical block.
    pub fn block_size(&self) -> usize {
        self.block_size
    }

    /// Free GPU blocks remaining.
    pub fn num_free_gpu_blocks(&self) -> usize {
        self.gpu.num_free()
    }

    /// Free CPU blocks remaining.
    pub fn num_free_cpu_blocks(&self) -> usize {
        self.cpu.num_free()
    }

    /// Number of physical blocks needed to hold `num_tokens` (ceil-div).
    pub fn blocks_for_tokens(num_tokens: usize, block_size: usize) -> usize {
        num_tokens.div_ceil(block_size)
    }

    /// Length of a sequence's block table (0 if unallocated).
    pub fn block_table_len(&self, seq: u64) -> usize {
        self.tables.get(&seq).map_or(0, |t| t.blocks.len())
    }

    /// Device a sequence resides on (defaults to GPU if unallocated).
    pub fn device_of(&self, seq: u64) -> Device {
        self.tables.get(&seq).map_or(Device::Gpu, |t| t.device)
    }

    /// Admission check for a fresh prompt of `num_tokens` tokens.
    pub fn can_allocate(&self, num_tokens: usize) -> AllocStatus {
        let need = Self::blocks_for_tokens(num_tokens, self.block_size);
        if need > self.gpu.num_blocks {
            // Exceeds the entire GPU pool — can never fit.
            return AllocStatus::Never;
        }
        let free = self.gpu.num_free();
        if free >= need + self.watermark_blocks {
            AllocStatus::Ok
        } else {
            AllocStatus::Later
        }
    }

    /// Allocate a fresh block table for `seq` covering `num_tokens`.
    pub fn allocate(&mut self, seq: u64, num_tokens: usize) -> Result<(), BlockError> {
        if self.tables.contains_key(&seq) {
            return Err(BlockError::AlreadyAllocated(seq));
        }
        let need = Self::blocks_for_tokens(num_tokens, self.block_size);
        if need > self.gpu.num_free() {
            return Err(BlockError::OutOfMemory {
                requested: need,
                available: self.gpu.num_free(),
            });
        }
        let mut blocks = Vec::with_capacity(need);
        for _ in 0..need {
            blocks.push(self.gpu.allocate().expect("checked free above"));
        }
        self.tables.insert(
            seq,
            SeqBlocks {
                blocks,
                device: Device::Gpu,
            },
        );
        Ok(())
    }

    /// Grow `seq` to hold `num_tokens_after` tokens, appending one slot.
    ///
    /// Returns `Some((src, dst))` when the last block was shared and had to
    /// be copied (copy-on-write); `None` when it grew in place or appended a
    /// brand-new block.
    pub fn append_slot(
        &mut self,
        seq: u64,
        num_tokens_after: usize,
    ) -> Result<Option<(usize, usize)>, BlockError> {
        let needed = Self::blocks_for_tokens(num_tokens_after, self.block_size);
        let (cur_len, device, last_block) = {
            let t = self.tables.get(&seq).ok_or(BlockError::NotAllocated(seq))?;
            (t.blocks.len(), t.device, t.blocks.last().copied())
        };
        let pool = match device {
            Device::Gpu => &mut self.gpu,
            Device::Cpu => &mut self.cpu,
        };
        if needed > cur_len {
            // Need a brand-new block at the tail.
            if pool.num_free() == 0 {
                return Err(BlockError::OutOfMemory {
                    requested: 1,
                    available: 0,
                });
            }
            let nb = pool.allocate().expect("checked free above");
            self.tables.get_mut(&seq).unwrap().blocks.push(nb);
            return Ok(None);
        }
        // Same block count: writing into the existing last block. If it is
        // shared with another sequence, copy-on-write into a fresh block.
        if let Some(last) = last_block {
            if pool.ref_count(last) > 1 {
                if pool.num_free() == 0 {
                    return Err(BlockError::OutOfMemory {
                        requested: 1,
                        available: 0,
                    });
                }
                let nb = pool.allocate().expect("checked free above");
                pool.free(last); // drop this sequence's hold on the shared block
                let blocks = &mut self.tables.get_mut(&seq).unwrap().blocks;
                *blocks.last_mut().unwrap() = nb;
                return Ok(Some((last, nb)));
            }
        }
        Ok(None)
    }

    /// Fork `child` from `parent`, sharing all of the parent's blocks.
    pub fn fork(&mut self, parent: u64, child: u64) -> Result<(), BlockError> {
        if self.tables.contains_key(&child) {
            return Err(BlockError::AlreadyAllocated(child));
        }
        let parent_blocks = self
            .tables
            .get(&parent)
            .ok_or(BlockError::NotAllocated(parent))?
            .clone();
        let pool = match parent_blocks.device {
            Device::Gpu => &mut self.gpu,
            Device::Cpu => &mut self.cpu,
        };
        for &b in &parent_blocks.blocks {
            pool.fork(b);
        }
        self.tables.insert(child, parent_blocks);
        Ok(())
    }

    /// Release all blocks held by `seq` (decrementing shared ref-counts).
    pub fn free(&mut self, seq: u64) {
        if let Some(t) = self.tables.remove(&seq) {
            let pool = match t.device {
                Device::Gpu => &mut self.gpu,
                Device::Cpu => &mut self.cpu,
            };
            for b in t.blocks {
                pool.free(b);
            }
        }
    }

    /// Move `seq`'s blocks from GPU to CPU; returns `(gpu_block, cpu_block)`
    /// pairs describing the copy the runtime must perform.
    pub fn swap_out(&mut self, seq: u64) -> Result<Vec<(usize, usize)>, BlockError> {
        let t = self.tables.get(&seq).ok_or(BlockError::NotAllocated(seq))?;
        if t.device != Device::Gpu {
            return Err(BlockError::WrongDevice {
                seq,
                expected: Device::Gpu,
                actual: t.device,
            });
        }
        let gpu_blocks = t.blocks.clone();
        if self.cpu.num_free() < gpu_blocks.len() {
            return Err(BlockError::OutOfMemory {
                requested: gpu_blocks.len(),
                available: self.cpu.num_free(),
            });
        }
        let mut mapping = Vec::with_capacity(gpu_blocks.len());
        let mut new_blocks = Vec::with_capacity(gpu_blocks.len());
        for gb in gpu_blocks {
            let cb = self.cpu.allocate().expect("checked cpu free above");
            mapping.push((gb, cb));
            new_blocks.push(cb);
            self.gpu.free(gb);
        }
        let t = self.tables.get_mut(&seq).unwrap();
        t.blocks = new_blocks;
        t.device = Device::Cpu;
        Ok(mapping)
    }

    /// True if `seq` (currently on CPU) would fit back into the GPU pool.
    pub fn can_swap_in(&self, seq: u64) -> bool {
        match self.tables.get(&seq) {
            Some(t) if t.device == Device::Cpu => self.gpu.num_free() >= t.blocks.len(),
            _ => false,
        }
    }

    /// Move `seq`'s blocks from CPU back to GPU; returns `(cpu_block,
    /// gpu_block)` copy pairs.
    pub fn swap_in(&mut self, seq: u64) -> Result<Vec<(usize, usize)>, BlockError> {
        let t = self.tables.get(&seq).ok_or(BlockError::NotAllocated(seq))?;
        if t.device != Device::Cpu {
            return Err(BlockError::WrongDevice {
                seq,
                expected: Device::Cpu,
                actual: t.device,
            });
        }
        let cpu_blocks = t.blocks.clone();
        if self.gpu.num_free() < cpu_blocks.len() {
            return Err(BlockError::OutOfMemory {
                requested: cpu_blocks.len(),
                available: self.gpu.num_free(),
            });
        }
        let mut mapping = Vec::with_capacity(cpu_blocks.len());
        let mut new_blocks = Vec::with_capacity(cpu_blocks.len());
        for cb in cpu_blocks {
            let gb = self.gpu.allocate().expect("checked gpu free above");
            mapping.push((cb, gb));
            new_blocks.push(gb);
            self.cpu.free(cb);
        }
        let t = self.tables.get_mut(&seq).unwrap();
        t.blocks = new_blocks;
        t.device = Device::Gpu;
        Ok(mapping)
    }
}
