// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD port of vLLM's PagedAttention block manager
// (vllm/core/block_manager.py + vllm/core/block/* — BlockSpaceManager +
// BlockAllocator: physical-block allocation, ref-counted copy-on-write,
// fork, free, GPU<->CPU swap, watermark admission).
//
// Upstream: vllm-project/vllm (Apache-2.0).

use cave_local_llm::vllm_paged_attention::{
    AllocStatus, BlockError, BlockSpaceManager, Device,
};

// ── BlockAllocator-level invariants (exposed via the manager) ───────────────

#[test]
fn new_manager_reports_all_blocks_free() {
    let m = BlockSpaceManager::new(16, 8, 4, 0.0);
    assert_eq!(m.num_free_gpu_blocks(), 8);
    assert_eq!(m.num_free_cpu_blocks(), 4);
    assert_eq!(m.block_size(), 16);
}

#[test]
fn ceil_div_blocks_required_for_token_count() {
    // 16 tokens/block: 1 token -> 1 block, 16 -> 1, 17 -> 2, 33 -> 3.
    assert_eq!(BlockSpaceManager::blocks_for_tokens(1, 16), 1);
    assert_eq!(BlockSpaceManager::blocks_for_tokens(16, 16), 1);
    assert_eq!(BlockSpaceManager::blocks_for_tokens(17, 16), 2);
    assert_eq!(BlockSpaceManager::blocks_for_tokens(33, 16), 3);
    assert_eq!(BlockSpaceManager::blocks_for_tokens(0, 16), 0);
}

// ── Admission: can_allocate with watermark ──────────────────────────────────

#[test]
fn can_allocate_ok_when_enough_free_above_watermark() {
    // 8 gpu blocks, watermark 0.0 -> 0 reserved. 32 tokens = 2 blocks.
    let m = BlockSpaceManager::new(16, 8, 0, 0.0);
    assert_eq!(m.can_allocate(32), AllocStatus::Ok);
}

#[test]
fn can_allocate_later_when_transiently_short_but_fits_total() {
    // watermark 0.5 of 8 = 4 reserved. Need 5 blocks (65 tokens): total 8 >= 5
    // so it could fit eventually, but free-above-watermark (8-4=4) < 5 -> Later.
    let m = BlockSpaceManager::new(16, 8, 0, 0.5);
    assert_eq!(m.can_allocate(65), AllocStatus::Later);
}

#[test]
fn can_allocate_never_when_request_exceeds_total_capacity() {
    // Need 9 blocks but only 8 exist, ever -> Never.
    let m = BlockSpaceManager::new(16, 8, 0, 0.0);
    assert_eq!(m.can_allocate(16 * 9), AllocStatus::Never);
}

// ── Allocate / free round-trips ─────────────────────────────────────────────

#[test]
fn allocate_consumes_blocks_and_free_returns_them() {
    let mut m = BlockSpaceManager::new(16, 8, 0, 0.0);
    m.allocate(1, 32).expect("allocate 2 blocks"); // 2 blocks
    assert_eq!(m.num_free_gpu_blocks(), 6);
    assert_eq!(m.block_table_len(1), 2);
    m.free(1);
    assert_eq!(m.num_free_gpu_blocks(), 8);
    assert_eq!(m.block_table_len(1), 0);
}

#[test]
fn allocate_twice_same_seq_is_rejected() {
    let mut m = BlockSpaceManager::new(16, 8, 0, 0.0);
    m.allocate(1, 16).unwrap();
    assert!(matches!(
        m.allocate(1, 16),
        Err(BlockError::AlreadyAllocated(1))
    ));
}

#[test]
fn out_of_memory_allocate_errors() {
    let mut m = BlockSpaceManager::new(16, 2, 0, 0.0);
    assert!(matches!(
        m.allocate(1, 16 * 3),
        Err(BlockError::OutOfMemory { .. })
    ));
}

// ── append_slot: grow-by-token + copy-on-write ──────────────────────────────

#[test]
fn append_slot_no_new_block_when_room_in_last_block() {
    let mut m = BlockSpaceManager::new(16, 8, 0, 0.0);
    m.allocate(1, 1).unwrap(); // 1 block, holds up to 16 tokens
    let cow = m.append_slot(1, 2).unwrap(); // now 2 tokens, still 1 block
    assert_eq!(cow, None);
    assert_eq!(m.block_table_len(1), 1);
    assert_eq!(m.num_free_gpu_blocks(), 7);
}

#[test]
fn append_slot_allocates_new_block_on_boundary() {
    let mut m = BlockSpaceManager::new(16, 8, 0, 0.0);
    m.allocate(1, 16).unwrap(); // exactly 1 full block
    let cow = m.append_slot(1, 17).unwrap(); // 17th token -> new block
    assert_eq!(cow, None);
    assert_eq!(m.block_table_len(1), 2);
    assert_eq!(m.num_free_gpu_blocks(), 6);
}

#[test]
fn append_slot_triggers_copy_on_write_for_shared_block() {
    let mut m = BlockSpaceManager::new(16, 8, 0, 0.0);
    m.allocate(1, 1).unwrap();
    m.fork(1, 2).unwrap(); // seq 2 shares seq 1's single block (ref_count=2)
    // Writing the next token for seq 1 must copy the shared last block.
    let cow = m.append_slot(1, 2).unwrap();
    let (src, dst) = cow.expect("copy-on-write src->dst expected");
    assert_ne!(src, dst);
    // After CoW the block is no longer shared by seq 1.
    assert_eq!(m.block_table_len(1), 1);
    assert_eq!(m.block_table_len(2), 1);
}

// ── fork shares physical blocks (ref-counting) ──────────────────────────────

#[test]
fn fork_shares_blocks_without_extra_allocation() {
    let mut m = BlockSpaceManager::new(16, 8, 0, 0.0);
    m.allocate(1, 32).unwrap(); // 2 blocks, 6 free
    m.fork(1, 2).unwrap();
    // No new physical blocks consumed by the fork.
    assert_eq!(m.num_free_gpu_blocks(), 6);
    assert_eq!(m.block_table_len(2), 2);
    // Freeing the parent leaves the child's shared blocks alive.
    m.free(1);
    assert_eq!(m.num_free_gpu_blocks(), 6);
    // Freeing the child finally releases them.
    m.free(2);
    assert_eq!(m.num_free_gpu_blocks(), 8);
}

// ── swap_out / swap_in (GPU <-> CPU) ────────────────────────────────────────

#[test]
fn swap_out_moves_blocks_to_cpu_and_returns_mapping() {
    let mut m = BlockSpaceManager::new(16, 8, 4, 0.0);
    m.allocate(1, 32).unwrap(); // 2 gpu blocks
    assert_eq!(m.num_free_gpu_blocks(), 6);
    let mapping = m.swap_out(1).unwrap();
    assert_eq!(mapping.len(), 2); // gpu_block -> cpu_block pairs
    assert_eq!(m.num_free_gpu_blocks(), 8); // gpu freed
    assert_eq!(m.num_free_cpu_blocks(), 2); // cpu consumed
    assert_eq!(m.device_of(1), Device::Cpu);
}

#[test]
fn swap_in_moves_blocks_back_to_gpu() {
    let mut m = BlockSpaceManager::new(16, 8, 4, 0.0);
    m.allocate(1, 32).unwrap();
    m.swap_out(1).unwrap();
    let mapping = m.swap_in(1).unwrap();
    assert_eq!(mapping.len(), 2);
    assert_eq!(m.num_free_cpu_blocks(), 4); // cpu released
    assert_eq!(m.num_free_gpu_blocks(), 6); // gpu reconsumed
    assert_eq!(m.device_of(1), Device::Gpu);
}

#[test]
fn can_swap_in_checks_gpu_headroom() {
    let mut m = BlockSpaceManager::new(16, 2, 4, 0.0);
    m.allocate(1, 32).unwrap(); // uses both gpu blocks
    m.swap_out(1).unwrap(); // gpu now free (2), cpu holds 2
    assert!(m.can_swap_in(1));
    // Occupy gpu with another seq so swap-in no longer fits.
    m.allocate(2, 32).unwrap();
    assert!(!m.can_swap_in(1));
}
