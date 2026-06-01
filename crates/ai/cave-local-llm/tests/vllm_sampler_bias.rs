// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD port of vLLM's per-request logits processors that act on the
// vocabulary mask rather than the distribution shape (vllm-project/vllm
// `vllm/model_executor/layers/sampler.py` min-tokens stop suppression +
// `vllm/entrypoints/openai/logits_processors.py` logit_bias / allowed_token
// handling, Apache-2.0):
//
//   * logit_bias        — add a per-token bias to selected logits
//   * suppress_tokens   — mask given token ids to -inf (bad-words, and the
//                         EOS/stop suppression used until min_tokens is met)
//   * restrict_to_allowed — mask every token NOT in the allowed set to -inf

use cave_local_llm::vllm_sampler::{apply_logit_bias, restrict_to_allowed, suppress_tokens};

const NEG_INF: f32 = f32::NEG_INFINITY;

#[test]
fn logit_bias_adds_to_selected_tokens() {
    let mut logits = vec![1.0_f32, 2.0, 3.0, 4.0];
    apply_logit_bias(&mut logits, &[(1, 10.0), (3, -100.0)]);
    assert_eq!(logits[0], 1.0);
    assert_eq!(logits[1], 12.0);
    assert_eq!(logits[2], 3.0);
    assert_eq!(logits[3], -96.0);
}

#[test]
fn logit_bias_ignores_out_of_range_ids() {
    let mut logits = vec![1.0_f32, 2.0];
    apply_logit_bias(&mut logits, &[(9, 5.0)]);
    assert_eq!(logits, vec![1.0, 2.0]);
}

#[test]
fn suppress_tokens_masks_to_neg_inf() {
    // EOS=0 and stop=2 suppressed (e.g. min_tokens not yet reached).
    let mut logits = vec![5.0_f32, 6.0, 7.0, 8.0];
    suppress_tokens(&mut logits, &[0, 2]);
    assert_eq!(logits[0], NEG_INF);
    assert_eq!(logits[1], 6.0);
    assert_eq!(logits[2], NEG_INF);
    assert_eq!(logits[3], 8.0);
}

#[test]
fn suppress_tokens_empty_is_noop() {
    let mut logits = vec![1.0_f32, 2.0, 3.0];
    suppress_tokens(&mut logits, &[]);
    assert_eq!(logits, vec![1.0, 2.0, 3.0]);
}

#[test]
fn restrict_to_allowed_masks_everything_else() {
    let mut logits = vec![1.0_f32, 2.0, 3.0, 4.0];
    restrict_to_allowed(&mut logits, &[1, 3]);
    assert_eq!(logits[0], NEG_INF);
    assert_eq!(logits[1], 2.0);
    assert_eq!(logits[2], NEG_INF);
    assert_eq!(logits[3], 4.0);
}

#[test]
fn restrict_to_allowed_empty_set_is_noop() {
    // An empty allow-list is treated as "no restriction" (vLLM leaves the
    // row untouched rather than masking the whole vocabulary).
    let mut logits = vec![1.0_f32, 2.0, 3.0];
    restrict_to_allowed(&mut logits, &[]);
    assert_eq!(logits, vec![1.0, 2.0, 3.0]);
}
