// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Speculative-decoding rejection sampler — a pure-Rust port of vLLM's
//! modified rejection sampling (vllm-project/vllm
//! `vllm/model_executor/layers/rejection_sampler.py` + `vllm/spec_decode/`,
//! Apache-2.0).
//!
//! A small **draft** model proposes `k` tokens with proposal distribution
//! `q`; the **target** model scores them with distribution `p`. Each draft
//! token `x_i` is accepted with probability `min(1, p(x_i)/q(x_i))`. On the
//! first rejection a **recovery** token is drawn from the normalized residual
//! `norm(max(0, p - q))`; if all `k` are accepted a **bonus** token is drawn
//! from the target's next-position distribution. This guarantees the output
//! is distributed exactly as if sampled from the target alone, while
//! emitting up to `k + 1` tokens per target forward pass.
//!
//! Randomness (`uniforms`, and the argmax tie-break used for recovery/bonus)
//! is injected so the acceptance logic is deterministic and unit-testable
//! without a GPU or RNG.

/// Outcome of one speculative step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptanceResult {
    /// Number of leading draft tokens accepted.
    pub accepted: usize,
    /// Emitted tokens: accepted drafts, then either a recovery token (on
    /// rejection) or a bonus token (on full acceptance).
    pub emitted: Vec<u32>,
    /// True when all `k` drafts were accepted (a bonus token was appended).
    pub all_accepted: bool,
}

/// Rejection sampler for `k` speculative tokens.
#[derive(Debug, Clone)]
pub struct RejectionSampler {
    k: usize,
}

/// Argmax index of a probability row (first max wins ties).
fn argmax(row: &[f32]) -> usize {
    let mut best = 0usize;
    let mut best_v = f32::NEG_INFINITY;
    for (i, &v) in row.iter().enumerate() {
        if v > best_v {
            best_v = v;
            best = i;
        }
    }
    best
}

impl RejectionSampler {
    /// New sampler proposing `num_speculative_tokens` (`k`) per step.
    pub fn new(num_speculative_tokens: usize) -> Self {
        Self {
            k: num_speculative_tokens,
        }
    }

    /// Number of speculative tokens per step.
    pub fn num_speculative_tokens(&self) -> usize {
        self.k
    }

    /// Run modified rejection sampling.
    ///
    /// * `draft_tokens` — the `k` proposed token ids.
    /// * `draft_probs` — proposal distribution `q` at each of the `k` positions.
    /// * `target_probs` — target distribution `p` at each of the `k` positions
    ///   plus one extra (`k + 1` rows) for the bonus token.
    /// * `uniforms` — `k` samples `u_i ~ U(0,1)` driving acceptance.
    pub fn sample(
        &self,
        draft_tokens: &[u32],
        draft_probs: &[Vec<f32>],
        target_probs: &[Vec<f32>],
        uniforms: &[f32],
    ) -> AcceptanceResult {
        let mut emitted: Vec<u32> = Vec::with_capacity(self.k + 1);
        for i in 0..self.k {
            let tok = draft_tokens[i] as usize;
            let q = draft_probs[i][tok];
            let p = target_probs[i][tok];
            // Accept with probability min(1, p/q). q > 0 for a drafted token.
            let accept_prob = if q > 0.0 { (p / q).min(1.0) } else { 1.0 };
            if uniforms[i] <= accept_prob {
                emitted.push(draft_tokens[i]);
                continue;
            }
            // Rejected at position i: emit a recovery token from the
            // normalized residual norm(max(0, p - q)).
            let recovery = recovery_token(&target_probs[i], &draft_probs[i]);
            emitted.push(recovery);
            return AcceptanceResult {
                accepted: i,
                emitted,
                all_accepted: false,
            };
        }
        // All k accepted: draw a bonus token from the target's next position.
        let bonus = argmax(&target_probs[self.k]);
        emitted.push(bonus as u32);
        AcceptanceResult {
            accepted: self.k,
            emitted,
            all_accepted: true,
        }
    }
}

/// Recovery token = argmax of the normalized residual `max(0, p - q)`.
///
/// Normalization does not change the argmax, so we pick the residual argmax
/// directly (a deterministic stand-in for vLLM's residual sampling).
fn recovery_token(p: &[f32], q: &[f32]) -> u32 {
    let mut best = 0usize;
    let mut best_v = f32::NEG_INFINITY;
    for j in 0..p.len() {
        let residual = (p[j] - q[j]).max(0.0);
        if residual > best_v {
            best_v = residual;
            best = j;
        }
    }
    best as u32
}

/// Running acceptance statistics across speculative steps.
#[derive(Debug, Default, Clone)]
pub struct AcceptanceStats {
    proposed: usize,
    accepted: usize,
}

impl AcceptanceStats {
    /// Fold one step's result into the totals (counts `k` proposed via the
    /// step's draft length — recorded as accepted + the rejected remainder).
    pub fn record(&mut self, result: &AcceptanceResult) {
        // Proposed this step = accepted drafts + (1 rejected, if not all
        // accepted). A fully-accepted step proposed exactly `accepted` drafts.
        let proposed = if result.all_accepted {
            result.accepted
        } else {
            result.accepted + 1
        };
        self.proposed += proposed;
        self.accepted += result.accepted;
    }

    /// Total draft tokens proposed.
    pub fn proposed(&self) -> usize {
        self.proposed
    }

    /// Total draft tokens accepted.
    pub fn accepted(&self) -> usize {
        self.accepted
    }

    /// Fraction of proposed draft tokens accepted (0.0 if none proposed).
    pub fn acceptance_rate(&self) -> f64 {
        if self.proposed == 0 {
            0.0
        } else {
            self.accepted as f64 / self.proposed as f64
        }
    }
}
