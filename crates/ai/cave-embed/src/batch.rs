// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Dynamic token-budget batching.
//!
//! infinity packs incoming inputs into batches bounded by both a maximum batch
//! size and a maximum-tokens-per-batch budget, sorting by sequence length first
//! so each batch pads to a similar length and wastes little compute. This module
//! computes that packing plan over token lengths, returning the original input
//! indices grouped per batch (caller maps results back). An input longer than
//! the per-batch budget is never dropped — it is emitted in a batch of its own.

/// Bounds on a single batch.
#[derive(Debug, Clone, Copy)]
pub struct BatchLimits {
    /// Maximum number of inputs in one batch.
    pub max_batch_size: usize,
    /// Maximum summed token length of one batch.
    pub max_tokens_per_batch: usize,
}

impl Default for BatchLimits {
    fn default() -> Self {
        // Defaults mirror a modest CPU serving profile.
        BatchLimits {
            max_batch_size: 32,
            max_tokens_per_batch: 8_192,
        }
    }
}

/// Plan batches over per-input token lengths.
///
/// Returns a list of batches; each batch is a list of indices into `lengths`.
/// Indices are emitted in descending-length order so similar lengths group
/// together. Every index appears in exactly one batch.
pub fn plan_batches(lengths: &[usize], limits: BatchLimits) -> Vec<Vec<usize>> {
    if lengths.is_empty() {
        return Vec::new();
    }
    let max_size = limits.max_batch_size.max(1);
    let budget = limits.max_tokens_per_batch.max(1);

    // Sort indices by descending length (ties broken by index for determinism).
    let mut order: Vec<usize> = (0..lengths.len()).collect();
    order.sort_by(|&a, &b| lengths[b].cmp(&lengths[a]).then(a.cmp(&b)));

    let mut batches: Vec<Vec<usize>> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    let mut current_tokens = 0usize;

    for idx in order {
        let len = lengths[idx];
        let fits_size = current.len() < max_size;
        let fits_budget = current.is_empty() || current_tokens + len <= budget;
        if fits_size && fits_budget {
            current.push(idx);
            current_tokens += len;
        } else {
            // Flush the current batch and start a new one with this item. An
            // oversized single item lands in its own (over-budget) batch here.
            if !current.is_empty() {
                batches.push(std::mem::take(&mut current));
            }
            current.push(idx);
            current_tokens = len;
        }
    }
    if !current.is_empty() {
        batches.push(current);
    }
    batches
}
