// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD port of vLLM's logits-processing sampler
// (vllm-project/vllm `vllm/model_executor/layers/sampler.py` +
// `vllm/model_executor/layers/utils.py`, Apache-2.0): the runtime warpers
// that turn raw logits into the sampling distribution — temperature scaling,
// top-k / top-p (nucleus) truncation, min-p relative-probability truncation,
// and the presence / frequency / repetition penalties. This is the *math*
// behind the request-level `SamplingParams` contract; the GPU kernels that
// run it stay out of scope (hardware-dependent), so the port operates on a
// plain `&mut [f32]` logits row and is fully deterministic.

use cave_local_llm::vllm_sampler::{
    apply_min_p, apply_penalties, apply_top_k, apply_top_p, softmax, temperature_scale,
};

const NEG_INF: f32 = f32::NEG_INFINITY;

// ── temperature ──────────────────────────────────────────────────────────────

#[test]
fn temperature_divides_logits() {
    let mut logits = vec![2.0_f32, 4.0, 8.0];
    temperature_scale(&mut logits, 2.0);
    assert_eq!(logits, vec![1.0, 2.0, 4.0]);
}

#[test]
fn temperature_one_is_noop() {
    let mut logits = vec![1.0_f32, -3.0, 7.5];
    temperature_scale(&mut logits, 1.0);
    assert_eq!(logits, vec![1.0, -3.0, 7.5]);
}

// ── top-k ────────────────────────────────────────────────────────────────────

#[test]
fn top_k_keeps_only_k_largest() {
    // Ascending logits; k=2 keeps the two largest (idx 2,3), masks the rest.
    let mut logits = vec![1.0_f32, 2.0, 3.0, 4.0];
    apply_top_k(&mut logits, 2);
    assert_eq!(logits[0], NEG_INF);
    assert_eq!(logits[1], NEG_INF);
    assert_eq!(logits[2], 3.0);
    assert_eq!(logits[3], 4.0);
}

#[test]
fn top_k_minus_one_disables() {
    let mut logits = vec![1.0_f32, 2.0, 3.0];
    apply_top_k(&mut logits, -1);
    assert_eq!(logits, vec![1.0, 2.0, 3.0]);
}

#[test]
fn top_k_larger_than_vocab_is_noop() {
    let mut logits = vec![5.0_f32, 1.0];
    apply_top_k(&mut logits, 9);
    assert_eq!(logits, vec![5.0, 1.0]);
}

#[test]
fn top_k_one_keeps_single_max() {
    let mut logits = vec![1.0_f32, 9.0, 3.0, 2.0];
    apply_top_k(&mut logits, 1);
    assert_eq!(logits[1], 9.0);
    assert_eq!(logits[0], NEG_INF);
    assert_eq!(logits[2], NEG_INF);
    assert_eq!(logits[3], NEG_INF);
}

// ── top-p (nucleus) ────────────────────────────────────────────────────────

#[test]
fn top_p_keeps_only_dominant_token() {
    // One token utterly dominates the softmax mass.
    let mut logits = vec![0.0_f32, 0.0, 0.0, 10.0];
    apply_top_p(&mut logits, 0.5);
    assert_eq!(logits[0], NEG_INF);
    assert_eq!(logits[1], NEG_INF);
    assert_eq!(logits[2], NEG_INF);
    assert_eq!(logits[3], 10.0);
}

#[test]
fn top_p_one_keeps_all() {
    let mut logits = vec![1.0_f32, 2.0, 3.0];
    apply_top_p(&mut logits, 1.0);
    assert_eq!(logits, vec![1.0, 2.0, 3.0]);
}

#[test]
fn top_p_always_keeps_the_largest_even_when_tiny() {
    // Even a near-zero top_p must keep at least the argmax token.
    let mut logits = vec![1.0_f32, 5.0, 2.0];
    apply_top_p(&mut logits, 1e-6);
    assert_eq!(logits[1], 5.0); // largest survives
    assert_eq!(logits[0], NEG_INF);
    assert_eq!(logits[2], NEG_INF);
}

// ── min-p ────────────────────────────────────────────────────────────────────

#[test]
fn min_p_filters_low_relative_probability() {
    let mut logits = vec![0.0_f32, 0.0, 0.0, 10.0];
    apply_min_p(&mut logits, 0.5);
    // Only the dominant token clears 0.5 * p_max.
    assert_eq!(logits[3], 10.0);
    assert_eq!(logits[0], NEG_INF);
    assert_eq!(logits[1], NEG_INF);
    assert_eq!(logits[2], NEG_INF);
}

#[test]
fn min_p_zero_is_noop() {
    let mut logits = vec![1.0_f32, 2.0, 3.0];
    apply_min_p(&mut logits, 0.0);
    assert_eq!(logits, vec![1.0, 2.0, 3.0]);
}

// ── softmax ──────────────────────────────────────────────────────────────────

#[test]
fn softmax_normalizes_to_one() {
    let probs = softmax(&[1.0, 2.0, 3.0]);
    let sum: f32 = probs.iter().sum();
    assert!((sum - 1.0).abs() < 1e-6);
    // Monotonic: larger logit -> larger prob.
    assert!(probs[2] > probs[1] && probs[1] > probs[0]);
}

#[test]
fn softmax_ignores_masked_neg_inf() {
    let probs = softmax(&[NEG_INF, 0.0, NEG_INF]);
    assert_eq!(probs[0], 0.0);
    assert_eq!(probs[2], 0.0);
    assert!((probs[1] - 1.0).abs() < 1e-6);
}

// ── penalties (cycle-2 surface, asserted minimally here) ─────────────────────

#[test]
fn penalties_signature_is_callable() {
    // Smoke: no penalties configured -> logits unchanged.
    let mut logits = vec![1.0_f32, 2.0, 3.0];
    apply_penalties(&mut logits, &[], &[], 0.0, 0.0, 1.0);
    assert_eq!(logits, vec![1.0, 2.0, 3.0]);
}
