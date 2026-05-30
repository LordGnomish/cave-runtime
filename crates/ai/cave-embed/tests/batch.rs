// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// TDD cycle 3 — tokenizer + dynamic token-budget batcher.
//
// infinity maximizes throughput by (a) measuring each input's token length,
// (b) truncating to the model context window, and (c) packing inputs into
// batches bounded by both a max batch size and a max-tokens-per-batch budget,
// sorting by length first so each batch wastes little padding.

use cave_embed::batch::{plan_batches, BatchLimits};
use cave_embed::tokenize::{count_tokens, tokenize, truncate};

#[test]
fn tokenize_splits_and_lowercases() {
    assert_eq!(tokenize("Hello,  WORLD!"), vec!["hello", "world"]);
    assert_eq!(tokenize("a1 b2-c3"), vec!["a1", "b2", "c3"]);
    assert!(tokenize("   ").is_empty());
}

#[test]
fn count_tokens_matches_tokenize_len() {
    assert_eq!(count_tokens("the quick brown fox"), 4);
    assert_eq!(count_tokens(""), 0);
}

#[test]
fn truncate_caps_token_count() {
    let t = truncate("one two three four five", 3);
    assert_eq!(count_tokens(&t), 3);
    assert_eq!(t, "one two three");
}

#[test]
fn truncate_noop_when_under_limit() {
    assert_eq!(truncate("a b", 10), "a b");
}

#[test]
fn batcher_respects_max_batch_size() {
    // 5 items, each 1 token; max 2 per batch → 3 batches of [2,2,1].
    let lens = vec![1usize, 1, 1, 1, 1];
    let limits = BatchLimits {
        max_batch_size: 2,
        max_tokens_per_batch: 1_000,
    };
    let batches = plan_batches(&lens, limits);
    assert_eq!(batches.len(), 3);
    assert!(batches.iter().all(|b| b.len() <= 2));
    // every original index appears exactly once
    let mut all: Vec<usize> = batches.iter().flatten().copied().collect();
    all.sort();
    assert_eq!(all, vec![0, 1, 2, 3, 4]);
}

#[test]
fn batcher_respects_token_budget() {
    // token budget 10; items of 8 and 5 cannot share (13 > 10).
    let lens = vec![8usize, 5, 4];
    let limits = BatchLimits {
        max_batch_size: 100,
        max_tokens_per_batch: 10,
    };
    let batches = plan_batches(&lens, limits);
    // sorted desc: 8, 5, 4. 8 alone; 5+4=9<=10 together.
    assert_eq!(batches.len(), 2);
    for b in &batches {
        let tok: usize = b.iter().map(|&i| lens[i]).sum();
        assert!(tok <= 10, "batch exceeds token budget: {tok}");
    }
}

#[test]
fn oversized_single_item_gets_own_batch() {
    // one item exceeds the per-batch budget on its own — must still be emitted.
    let lens = vec![50usize, 3];
    let limits = BatchLimits {
        max_batch_size: 100,
        max_tokens_per_batch: 10,
    };
    let batches = plan_batches(&lens, limits);
    let all: Vec<usize> = batches.iter().flatten().copied().collect();
    assert_eq!(all.len(), 2, "no item may be dropped");
    assert!(batches.iter().any(|b| b == &vec![0]));
}

#[test]
fn batcher_sorts_by_length_descending() {
    let lens = vec![1usize, 9, 3];
    let limits = BatchLimits {
        max_batch_size: 1,
        max_tokens_per_batch: 1_000,
    };
    let batches = plan_batches(&lens, limits);
    // each its own batch, ordered by descending length: idx 1(9), 2(3), 0(1)
    assert_eq!(batches, vec![vec![1], vec![2], vec![0]]);
}

#[test]
fn empty_input_yields_no_batches() {
    let batches = plan_batches(&[], BatchLimits::default());
    assert!(batches.is_empty());
}
