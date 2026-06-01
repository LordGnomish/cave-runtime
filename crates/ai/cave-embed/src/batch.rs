// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Dynamic batching planner.
//!
//! infinity's `BatchHandler` queues incoming sentences and dispatches them in
//! micro-batches, sorting by sequence length so each padded batch wastes as
//! little compute as possible (similar-length sequences padded together). We
//! re-implement that planning step as a pure function: given a set of items
//! and the batch caps, produce length-sorted batches that respect both the
//! item-count cap and the padded-token budget.

/// One queued item to embed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchItem {
    /// Caller-assigned id used to re-associate results with requests.
    pub id: usize,
    /// Token length of the input (drives padding cost).
    pub len: usize,
}

/// Batching caps.
#[derive(Debug, Clone, Copy)]
pub struct BatchConfig {
    /// Maximum number of items per batch.
    pub max_batch_size: usize,
    /// Maximum padded tokens per batch (`count * max_len_in_batch`).
    pub max_tokens_per_batch: usize,
}

impl Default for BatchConfig {
    fn default() -> Self {
        Self {
            max_batch_size: 32,
            max_tokens_per_batch: 16_384,
        }
    }
}

/// Padded-token waste of a batch: `count * max_len - sum(len)`.
pub fn padding_waste(batch: &[BatchItem]) -> usize {
    if batch.is_empty() {
        return 0;
    }
    let max_len = batch.iter().map(|i| i.len).max().unwrap_or(0);
    let real: usize = batch.iter().map(|i| i.len).sum();
    batch.len() * max_len - real
}

/// Plan length-sorted micro-batches honoring both caps. An item longer than
/// the token budget on its own still receives a singleton batch (never dropped).
pub fn plan_batches(items: Vec<BatchItem>, cfg: &BatchConfig) -> Vec<Vec<BatchItem>> {
    if items.is_empty() {
        return Vec::new();
    }
    let max_batch = cfg.max_batch_size.max(1);
    // Length-sort ascending so each batch groups similar-length sequences,
    // minimizing the padding that a uniform-length batch must add.
    let mut sorted = items;
    sorted.sort_by_key(|i| i.len);

    let mut batches: Vec<Vec<BatchItem>> = Vec::new();
    let mut current: Vec<BatchItem> = Vec::new();
    let mut cur_max = 0usize;

    for item in sorted {
        // With ascending sort the incoming item is the new max-length, so the
        // padded cost of the prospective batch is (count+1) * item.len.
        let prospective_max = cur_max.max(item.len);
        let prospective_tokens = (current.len() + 1) * prospective_max;
        let fits = current.len() < max_batch && prospective_tokens <= cfg.max_tokens_per_batch;
        if !current.is_empty() && !fits {
            batches.push(std::mem::take(&mut current));
            cur_max = 0;
        }
        cur_max = cur_max.max(item.len);
        current.push(item);
    }
    if !current.is_empty() {
        batches.push(current);
    }
    batches
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(lens: &[usize]) -> Vec<BatchItem> {
        lens.iter()
            .enumerate()
            .map(|(id, &len)| BatchItem { id, len })
            .collect()
    }

    fn total_count(batches: &[Vec<BatchItem>]) -> usize {
        batches.iter().map(|b| b.len()).sum()
    }

    #[test]
    fn empty_in_empty_out() {
        assert!(plan_batches(vec![], &BatchConfig::default()).is_empty());
    }

    #[test]
    fn respects_max_batch_size() {
        let cfg = BatchConfig {
            max_batch_size: 2,
            max_tokens_per_batch: 1_000_000,
        };
        let b = plan_batches(items(&[5, 5, 5, 5, 5]), &cfg);
        assert!(b.iter().all(|batch| batch.len() <= 2));
        assert_eq!(total_count(&b), 5);
    }

    #[test]
    fn respects_token_budget() {
        let cfg = BatchConfig {
            max_batch_size: 100,
            max_tokens_per_batch: 20, // count*max_len <= 20
        };
        let b = plan_batches(items(&[10, 10, 10]), &cfg);
        // each batch: count*10 <= 20 => max 2 items per batch
        assert!(b.iter().all(|batch| batch.len() * 10 <= 20));
        assert_eq!(total_count(&b), 3);
    }

    #[test]
    fn oversized_item_gets_singleton_batch() {
        let cfg = BatchConfig {
            max_batch_size: 8,
            max_tokens_per_batch: 5,
        };
        let b = plan_batches(items(&[100]), &cfg);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].len(), 1);
        assert_eq!(b[0][0].id, 0);
    }

    #[test]
    fn length_sort_reduces_padding_vs_input_order() {
        let cfg = BatchConfig {
            max_batch_size: 2,
            max_tokens_per_batch: 1_000_000,
        };
        let input = items(&[10, 1, 9, 2]);
        let planned = plan_batches(input.clone(), &cfg);
        let planned_waste: usize = planned.iter().map(|b| padding_waste(b)).sum();
        // Naive input-order pairing: (10,1),(9,2)
        let naive_waste = padding_waste(&input[0..2]) + padding_waste(&input[2..4]);
        assert!(
            planned_waste < naive_waste,
            "planned {planned_waste} should beat naive {naive_waste}"
        );
    }

    #[test]
    fn preserves_all_items() {
        let cfg = BatchConfig {
            max_batch_size: 3,
            max_tokens_per_batch: 50,
        };
        let b = plan_batches(items(&[4, 8, 1, 7, 3, 9, 2]), &cfg);
        let mut ids: Vec<usize> = b.iter().flatten().map(|i| i.id).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec![0, 1, 2, 3, 4, 5, 6]);
    }
}
