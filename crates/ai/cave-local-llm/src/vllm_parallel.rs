// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD port of vLLM's distributed sharding math
// (vllm-project/vllm `vllm/distributed/utils.py` plus the parallel-linear and
// `VocabParallelEmbedding` partition rules, Apache-2.0).
//
// Every rank in a tensor/pipeline-parallel deployment runs this same integer
// arithmetic to decide which slice of each weight matrix and which transformer
// layers it is responsible for. Ranks are laid out with the tensor-parallel
// dimension *inner* and the pipeline-parallel dimension *outer*:
//
//     global_rank = pp_rank * tp_size + tp_rank
//
// so a contiguous block of `tp_size` ranks forms one tensor-parallel group
// (one pipeline stage), and ranks strided by `tp_size` form a pipeline group.
//
// Only the partition metadata is ported here. The collectives that move
// activations between ranks (all-reduce after a row-parallel matmul, all-gather
// after a column-parallel one, point-to-point across pipeline stages) require a
// real multi-device backend and remain a documented host scope-cut.

/// Errors from the sharding helpers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelError {
    /// A dimension was not evenly divisible by the parallel size.
    NotDivisible {
        /// The dimension being split.
        numerator: usize,
        /// The parallel size it must divide by.
        denominator: usize,
    },
}

impl std::fmt::Display for ParallelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParallelError::NotDivisible {
                numerator,
                denominator,
            } => write!(f, "{numerator} is not divisible by {denominator}"),
        }
    }
}

impl std::error::Error for ParallelError {}

/// vLLM's `divide`: assert even divisibility and return the quotient.
fn try_divide(numerator: usize, denominator: usize) -> Result<usize, ParallelError> {
    if denominator == 0 || numerator % denominator != 0 {
        return Err(ParallelError::NotDivisible {
            numerator,
            denominator,
        });
    }
    Ok(numerator / denominator)
}

/// Tensor- and pipeline-parallel sizes for a deployment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParallelConfig {
    /// Number of tensor-parallel shards (inner dimension).
    pub tensor_parallel_size: usize,
    /// Number of pipeline stages (outer dimension).
    pub pipeline_parallel_size: usize,
}

impl ParallelConfig {
    /// Total ranks = `tp_size * pp_size`.
    pub fn world_size(&self) -> usize {
        self.tensor_parallel_size * self.pipeline_parallel_size
    }

    /// Tensor-parallel rank of a global rank (`global % tp_size`).
    pub fn tp_rank(&self, global_rank: usize) -> usize {
        global_rank % self.tensor_parallel_size
    }

    /// Pipeline-parallel rank of a global rank (`global / tp_size`).
    pub fn pp_rank(&self, global_rank: usize) -> usize {
        global_rank / self.tensor_parallel_size
    }

    /// The tensor-parallel group containing `global_rank`: the contiguous block
    /// of `tp_size` ranks that share this rank's pipeline stage.
    pub fn tp_group(&self, global_rank: usize) -> Vec<usize> {
        let base = self.pp_rank(global_rank) * self.tensor_parallel_size;
        (base..base + self.tensor_parallel_size).collect()
    }

    /// The pipeline-parallel group containing `global_rank`: the ranks strided
    /// by `tp_size` that share this rank's tensor slice.
    pub fn pp_group(&self, global_rank: usize) -> Vec<usize> {
        let tp = self.tp_rank(global_rank);
        (0..self.pipeline_parallel_size)
            .map(|pp| pp * self.tensor_parallel_size + tp)
            .collect()
    }
}

/// Round `vocab` up to the next multiple of `multiple` — vLLM's
/// `pad_vocab_size`, which keeps the embedding table evenly shardable across
/// tensor-parallel ranks.
pub fn pad_vocab_size(vocab: usize, multiple: usize) -> usize {
    vocab.div_ceil(multiple) * multiple
}

/// The slice of a `VocabParallelEmbedding` owned by one tensor-parallel rank.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VocabShard {
    /// Global vocab size after padding to a multiple of `tp_size`.
    pub padded_vocab: usize,
    /// Rows on this rank (`padded_vocab / tp_size`).
    pub num_embeddings_per_partition: usize,
    /// First global vocab index this rank owns (inclusive).
    pub start: usize,
    /// One past the last global vocab index this rank owns (exclusive).
    pub end: usize,
}

/// Partition a vocab of `global_vocab` rows across `tp_size` ranks: pad to a
/// multiple of `tp_size`, then hand each rank a contiguous equal block
/// (`vocab_range_from_global_vocab_size`).
pub fn vocab_partition(global_vocab: usize, tp_rank: usize, tp_size: usize) -> VocabShard {
    let padded = pad_vocab_size(global_vocab, tp_size);
    let per = padded / tp_size;
    let start = tp_rank * per;
    VocabShard {
        padded_vocab: padded,
        num_embeddings_per_partition: per,
        start,
        end: start + per,
    }
}

/// Per-rank output size of a `ColumnParallelLinear` (output features split).
pub fn column_parallel_shard(out_features: usize, tp_size: usize) -> Result<usize, ParallelError> {
    try_divide(out_features, tp_size)
}

/// Per-rank input size of a `RowParallelLinear` (input features split).
pub fn row_parallel_shard(in_features: usize, tp_size: usize) -> Result<usize, ParallelError> {
    try_divide(in_features, tp_size)
}

/// Attention query heads owned by each rank (`num_heads / tp_size`).
pub fn attn_heads_per_rank(num_heads: usize, tp_size: usize) -> Result<usize, ParallelError> {
    try_divide(num_heads, tp_size)
}

/// Key/value heads owned by each rank under grouped-query attention.
///
/// When `num_kv_heads >= tp_size` they split evenly; when there are fewer KV
/// heads than ranks they are *replicated*, so each rank still holds at least
/// one (vLLM clamps `num_kv_heads // tp_size` up to 1).
pub fn kv_heads_per_rank(num_kv_heads: usize, tp_size: usize) -> usize {
    (num_kv_heads / tp_size).max(1)
}

/// Pipeline-parallel layer range `[start, end)` for `pp_rank`, vLLM's default
/// `get_pp_indices`: an even split where the *last* stage absorbs any
/// remainder.
pub fn get_pp_indices(num_layers: usize, pp_rank: usize, pp_size: usize) -> (usize, usize) {
    let layers_per_partition = num_layers / pp_size;
    let start = pp_rank * layers_per_partition;
    let end = if pp_rank == pp_size - 1 {
        num_layers
    } else {
        start + layers_per_partition
    };
    (start, end)
}

/// Pipeline-parallel layer range from an explicit per-stage partition list
/// (vLLM's `VLLM_PP_LAYER_PARTITION` override): the range is the prefix sum of
/// the preceding stages' layer counts.
pub fn pp_partition_custom(partition: &[usize], pp_rank: usize) -> (usize, usize) {
    let start: usize = partition[..pp_rank].iter().sum();
    (start, start + partition[pp_rank])
}
