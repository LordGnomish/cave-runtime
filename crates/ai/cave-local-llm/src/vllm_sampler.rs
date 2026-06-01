// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! vLLM logits-processing sampler — a pure-Rust port of the runtime warpers
//! that turn raw model logits into the sampling distribution
//! (vllm-project/vllm `vllm/model_executor/layers/sampler.py` +
//! `vllm/model_executor/layers/utils.py::apply_penalties`, Apache-2.0).
//!
//! The request-level knobs live in [`crate::vllm_sampling::SamplingParams`];
//! this module is the *math* those knobs drive, applied per decode step to a
//! single logits row in vLLM's canonical order:
//!
//! 1. **penalties** — repetition (multiplicative), then presence + frequency
//!    (additive), against the prompt + output token history;
//! 2. **temperature** — divide logits by `temperature` (skipped for greedy);
//! 3. **top-k** — keep only the `k` largest logits, mask the rest to `-inf`;
//! 4. **top-p** — keep the smallest set of top tokens whose cumulative
//!    probability mass reaches `top_p` (nucleus), always keeping the argmax;
//! 5. **min-p** — drop tokens whose probability is below `min_p · p_max`.
//!
//! The GPU kernels that execute this on-device are out of scope; operating on
//! a plain `&mut [f32]` keeps the control logic deterministic and testable.

use crate::vllm_sampling::SamplingParams;

/// Below this temperature, sampling is treated as greedy (no scaling).
const SAMPLING_EPS: f32 = 1e-5;

/// Numerically-stable softmax over a logits row. `-inf` entries (masked
/// tokens) map to exactly `0.0`; the surviving probabilities sum to 1.
pub fn softmax(logits: &[f32]) -> Vec<f32> {
    let max = logits
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold(f32::NEG_INFINITY, f32::max);
    if !max.is_finite() {
        // All masked — degenerate; return a uniform-zero row.
        return vec![0.0; logits.len()];
    }
    let mut exps: Vec<f32> = logits
        .iter()
        .map(|&v| if v.is_finite() { (v - max).exp() } else { 0.0 })
        .collect();
    let sum: f32 = exps.iter().sum();
    if sum > 0.0 {
        for e in &mut exps {
            *e /= sum;
        }
    }
    exps
}

/// Divide every logit by `temperature` (vLLM `_apply_temperature`).
///
/// A temperature at/below [`SAMPLING_EPS`] (greedy) or exactly `1.0` is a
/// no-op — greedy argmax is unaffected by uniform scaling.
pub fn temperature_scale(logits: &mut [f32], temperature: f32) {
    if temperature < SAMPLING_EPS || temperature == 1.0 {
        return;
    }
    for l in logits.iter_mut() {
        *l /= temperature;
    }
}

/// Keep only the `top_k` largest logits, masking the rest to `-inf`
/// (vLLM `_apply_top_k_top_p`, top-k half).
///
/// `top_k == -1` (or `>=` vocab size) disables truncation.
pub fn apply_top_k(logits: &mut [f32], top_k: i32) {
    if top_k < 1 {
        return;
    }
    let k = top_k as usize;
    if k >= logits.len() {
        return;
    }
    // Threshold = the k-th largest value; anything strictly below it is masked.
    let mut sorted: Vec<f32> = logits.iter().copied().filter(|v| v.is_finite()).collect();
    sorted.sort_by(|a, b| b.partial_cmp(a).unwrap());
    let threshold = sorted[k - 1];
    for l in logits.iter_mut() {
        if *l < threshold {
            *l = f32::NEG_INFINITY;
        }
    }
}

/// Nucleus truncation: keep the smallest set of highest-probability tokens
/// whose cumulative mass reaches `top_p`, masking the long tail to `-inf`
/// (vLLM `_apply_top_k_top_p`, top-p half). The argmax token always survives.
pub fn apply_top_p(logits: &mut [f32], top_p: f32) {
    if top_p >= 1.0 {
        return;
    }
    let probs = softmax(logits);
    // Indices sorted by ascending probability (vLLM sorts ascending and masks
    // the low-mass prefix whose cumulative sum is <= 1 - top_p).
    let mut order: Vec<usize> = (0..probs.len()).collect();
    order.sort_by(|&a, &b| probs[a].partial_cmp(&probs[b]).unwrap());
    let cutoff = 1.0 - top_p;
    let mut cumulative = 0.0_f32;
    let n = order.len();
    for (rank, &idx) in order.iter().enumerate() {
        // Never mask the largest-probability token (the final entry).
        if rank == n - 1 {
            break;
        }
        cumulative += probs[idx];
        if cumulative <= cutoff {
            logits[idx] = f32::NEG_INFINITY;
        }
    }
}

/// Relative-probability truncation: drop tokens whose probability is below
/// `min_p · p_max` (vLLM `_apply_min_p`). `min_p == 0` disables it.
pub fn apply_min_p(logits: &mut [f32], min_p: f32) {
    if min_p <= 0.0 {
        return;
    }
    let probs = softmax(logits);
    let p_max = probs.iter().copied().fold(0.0_f32, f32::max);
    let threshold = min_p * p_max;
    for (i, l) in logits.iter_mut().enumerate() {
        if probs[i] < threshold {
            *l = f32::NEG_INFINITY;
        }
    }
}

/// Apply the repetition / presence / frequency penalties against the prompt
/// and output token histories (vLLM `apply_penalties`).
///
/// * **repetition** (`> 0`, `1.0` = off): for any token id appearing in the
///   prompt *or* output, `logit /= rep` when `logit > 0`, else `logit *= rep`.
/// * **frequency**: `logit -= frequency · output_count(token)`.
/// * **presence**: `logit -= presence` once for any token present in output.
///
/// `prompt_tokens` / `output_tokens` are the raw id histories; counts are
/// derived here (vLLM precomputes bincounts on-device).
pub fn apply_penalties(
    logits: &mut [f32],
    prompt_tokens: &[u32],
    output_tokens: &[u32],
    presence: f32,
    frequency: f32,
    repetition: f32,
) {
    let vocab = logits.len();
    // Output-token bincounts (drive presence + frequency).
    let mut output_counts = vec![0u32; vocab];
    for &t in output_tokens {
        let t = t as usize;
        if t < vocab {
            output_counts[t] += 1;
        }
    }
    // Prompt-or-output presence mask (drives repetition).
    let mut seen = vec![false; vocab];
    for &t in prompt_tokens.iter().chain(output_tokens.iter()) {
        let t = t as usize;
        if t < vocab {
            seen[t] = true;
        }
    }

    for (i, l) in logits.iter_mut().enumerate() {
        // Repetition penalty (multiplicative), only for repeated tokens.
        if repetition != 1.0 && seen[i] {
            if *l > 0.0 {
                *l /= repetition;
            } else {
                *l *= repetition;
            }
        }
        // Frequency penalty (scaled by output occurrences).
        if frequency != 0.0 {
            *l -= frequency * output_counts[i] as f32;
        }
        // Presence penalty (once if present in output).
        if presence != 0.0 && output_counts[i] > 0 {
            *l -= presence;
        }
    }
}

/// Add a per-token bias to selected logits (vLLM OpenAI `logit_bias`:
/// `logits[token] += bias`). Out-of-range ids are ignored.
pub fn apply_logit_bias(logits: &mut [f32], biases: &[(u32, f32)]) {
    let vocab = logits.len();
    for &(token, bias) in biases {
        let t = token as usize;
        if t < vocab {
            logits[t] += bias;
        }
    }
}

/// Mask the given token ids to `-inf` (vLLM bad-words masking, and the
/// EOS/stop-token suppression applied while `min_tokens` is unmet). Out-of-
/// range ids are ignored; an empty list is a no-op.
pub fn suppress_tokens(logits: &mut [f32], token_ids: &[u32]) {
    let vocab = logits.len();
    for &t in token_ids {
        let t = t as usize;
        if t < vocab {
            logits[t] = f32::NEG_INFINITY;
        }
    }
}

/// Mask every token NOT in `allowed` to `-inf` (vLLM `allowed_token_ids`).
/// An empty allow-list means "no restriction" — the row is left untouched.
pub fn restrict_to_allowed(logits: &mut [f32], allowed: &[u32]) {
    if allowed.is_empty() {
        return;
    }
    let vocab = logits.len();
    let mut keep = vec![false; vocab];
    for &t in allowed {
        let t = t as usize;
        if t < vocab {
            keep[t] = true;
        }
    }
    for (i, l) in logits.iter_mut().enumerate() {
        if !keep[i] {
            *l = f32::NEG_INFINITY;
        }
    }
}

/// Apply the full vLLM warp pipeline for one decode step, in canonical order:
/// penalties → temperature → top-k → top-p → min-p. Returns the resulting
/// sampling probability distribution (`softmax` of the warped logits).
pub fn process(
    logits: &mut [f32],
    params: &SamplingParams,
    prompt_tokens: &[u32],
    output_tokens: &[u32],
) -> Vec<f32> {
    apply_penalties(
        logits,
        prompt_tokens,
        output_tokens,
        params.presence_penalty,
        params.frequency_penalty,
        params.repetition_penalty,
    );
    temperature_scale(logits, params.temperature);
    apply_top_k(logits, params.top_k);
    apply_top_p(logits, params.top_p);
    apply_min_p(logits, params.min_p);
    softmax(logits)
}
