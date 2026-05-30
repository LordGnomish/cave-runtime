// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD port of vLLM's distributed sharding math
// (vllm-project/vllm `vllm/distributed/utils.py` and the parallel-linear /
// VocabParallelEmbedding partition rules in
// `vllm/model_executor/layers/{linear,vocab_parallel_embedding}.py`,
// Apache-2.0).
//
// This is the partitioning *metadata* that a tensor/pipeline-parallel
// deployment computes on every rank to decide which slice of weights and which
// layers it owns. It is pure integer arithmetic — no collectives, no device —
// so it ports cleanly into the in-process runtime; the actual all-reduce /
// all-gather collectives stay a documented host scope-cut.

use cave_local_llm::vllm_parallel::{
    attn_heads_per_rank, column_parallel_shard, get_pp_indices, kv_heads_per_rank,
    pad_vocab_size, pp_partition_custom, row_parallel_shard, vocab_partition, ParallelConfig,
    ParallelError,
};

// ── rank ↔ (tp_rank, pp_rank) topology ───────────────────────────────────────

#[test]
fn world_size_is_tp_times_pp() {
    let cfg = ParallelConfig {
        tensor_parallel_size: 4,
        pipeline_parallel_size: 2,
    };
    assert_eq!(cfg.world_size(), 8);
}

#[test]
fn global_rank_decomposes_tp_inner_pp_outer() {
    // tp=2, pp=2 → global ranks 0..4 laid out as [pp][tp].
    let cfg = ParallelConfig {
        tensor_parallel_size: 2,
        pipeline_parallel_size: 2,
    };
    // rank: (tp_rank, pp_rank)
    assert_eq!((cfg.tp_rank(0), cfg.pp_rank(0)), (0, 0));
    assert_eq!((cfg.tp_rank(1), cfg.pp_rank(1)), (1, 0));
    assert_eq!((cfg.tp_rank(2), cfg.pp_rank(2)), (0, 1));
    assert_eq!((cfg.tp_rank(3), cfg.pp_rank(3)), (1, 1));
}

#[test]
fn tp_and_pp_groups_partition_the_world() {
    let cfg = ParallelConfig {
        tensor_parallel_size: 2,
        pipeline_parallel_size: 2,
    };
    // TP group = ranks sharing a pipeline stage (contiguous tp block).
    assert_eq!(cfg.tp_group(0), vec![0, 1]);
    assert_eq!(cfg.tp_group(2), vec![2, 3]);
    // PP group = ranks sharing a tensor slice (strided by tp_size).
    assert_eq!(cfg.pp_group(0), vec![0, 2]);
    assert_eq!(cfg.pp_group(1), vec![1, 3]);
}

// ── vocab-parallel embedding partition ───────────────────────────────────────

#[test]
fn pad_vocab_size_rounds_up_to_multiple() {
    assert_eq!(pad_vocab_size(50, 4), 52);
    assert_eq!(pad_vocab_size(48, 4), 48); // already aligned
    assert_eq!(pad_vocab_size(1, 8), 8);
}

#[test]
fn vocab_partition_pads_then_splits_evenly() {
    // 50-token vocab over tp=4 → pad to 52, 13 rows per rank.
    let p0 = vocab_partition(50, 0, 4);
    let p1 = vocab_partition(50, 1, 4);
    let p3 = vocab_partition(50, 3, 4);
    assert_eq!(p0.padded_vocab, 52);
    assert_eq!(p0.num_embeddings_per_partition, 13);
    assert_eq!((p0.start, p0.end), (0, 13));
    assert_eq!((p1.start, p1.end), (13, 26));
    assert_eq!((p3.start, p3.end), (39, 52));
}

// ── column / row parallel linear shards ──────────────────────────────────────

#[test]
fn column_parallel_splits_output_features() {
    assert_eq!(column_parallel_shard(4096, 8).unwrap(), 512);
}

#[test]
fn row_parallel_splits_input_features() {
    assert_eq!(row_parallel_shard(11008, 8).unwrap(), 1376);
}

#[test]
fn non_divisible_shard_is_an_error() {
    assert_eq!(
        column_parallel_shard(10, 3).unwrap_err(),
        ParallelError::NotDivisible {
            numerator: 10,
            denominator: 3,
        }
    );
}

// ── attention head sharding (incl. GQA replication) ──────────────────────────

#[test]
fn attention_heads_split_across_ranks() {
    assert_eq!(attn_heads_per_rank(32, 8).unwrap(), 4);
    assert_eq!(attn_heads_per_rank(32, 1).unwrap(), 32);
}

#[test]
fn kv_heads_replicate_when_fewer_than_tp_size() {
    // GQA: 8 KV heads over tp=8 → 1 each.
    assert_eq!(kv_heads_per_rank(8, 8), 1);
    // 8 KV heads over tp=2 → 4 each.
    assert_eq!(kv_heads_per_rank(8, 2), 4);
    // 2 KV heads over tp=8 → replicated, clamped to 1 per rank.
    assert_eq!(kv_heads_per_rank(2, 8), 1);
}

// ── pipeline-parallel layer partition ────────────────────────────────────────

#[test]
fn pp_indices_split_layers_evenly() {
    // 12 layers over pp=4 → 3 each.
    assert_eq!(get_pp_indices(12, 0, 4), (0, 3));
    assert_eq!(get_pp_indices(12, 1, 4), (3, 6));
    assert_eq!(get_pp_indices(12, 3, 4), (9, 12));
}

#[test]
fn pp_indices_last_rank_absorbs_remainder() {
    // 10 layers over pp=4 → 2 each, last rank takes the extra 2.
    assert_eq!(get_pp_indices(10, 0, 4), (0, 2));
    assert_eq!(get_pp_indices(10, 1, 4), (2, 4));
    assert_eq!(get_pp_indices(10, 2, 4), (4, 6));
    assert_eq!(get_pp_indices(10, 3, 4), (6, 10));
}

#[test]
fn pp_custom_partition_list() {
    // Explicit per-stage layer counts (VLLM_PP_LAYER_PARTITION analog).
    let parts = [4, 2, 2, 4];
    assert_eq!(pp_partition_custom(&parts, 0), (0, 4));
    assert_eq!(pp_partition_custom(&parts, 1), (4, 6));
    assert_eq!(pp_partition_custom(&parts, 2), (6, 8));
    assert_eq!(pp_partition_custom(&parts, 3), (8, 12));
}
