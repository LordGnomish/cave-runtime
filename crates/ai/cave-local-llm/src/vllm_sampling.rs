// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! vLLM `SamplingParams` — a pure-Rust port of vLLM's request-level sampling
//! contract (vllm-project/vllm `vllm/sampling_params.py`, Apache-2.0).
//!
//! Carries the knobs a serving request controls — temperature, nucleus
//! (`top_p`), top-k, `min_p`, presence/frequency/repetition penalties, `n`,
//! `best_of`, stop strings, and logprobs — plus the `_verify_args` bounds
//! checks and the greedy-vs-random sampling classification. The OpenAI-compat
//! request shape maps onto it via [`SamplingParams::from_openai`].

use thiserror::Error;

/// Below this temperature, sampling is treated as greedy (argmax).
const SAMPLING_EPS: f32 = 1e-5;

/// How tokens are drawn from the output distribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SamplingType {
    /// `temperature == 0`: deterministic argmax.
    Greedy,
    /// Stochastic sampling, no fixed seed.
    Random,
    /// Stochastic sampling with a per-request RNG seed.
    RandomSeed,
}

/// Validation failures mirroring vLLM's `_verify_args` `ValueError`s.
#[derive(Debug, Error, PartialEq)]
pub enum SamplingError {
    /// `temperature < 0`.
    #[error("temperature must be non-negative, got {0}")]
    Temperature(f32),
    /// `top_p` outside `(0, 1]`.
    #[error("top_p must be in (0, 1], got {0}")]
    TopP(f32),
    /// `top_k` is neither -1 (disabled) nor >= 1.
    #[error("top_k must be -1 (disable) or >= 1, got {0}")]
    TopK(i32),
    /// `min_p` outside `[0, 1]`.
    #[error("min_p must be in [0, 1], got {0}")]
    MinP(f32),
    /// `presence_penalty` outside `[-2, 2]`.
    #[error("presence_penalty must be in [-2, 2], got {0}")]
    PresencePenalty(f32),
    /// `frequency_penalty` outside `[-2, 2]`.
    #[error("frequency_penalty must be in [-2, 2], got {0}")]
    FrequencyPenalty(f32),
    /// `repetition_penalty` not strictly positive.
    #[error("repetition_penalty must be > 0, got {0}")]
    RepetitionPenalty(f32),
    /// `n < 1`.
    #[error("n must be >= 1, got {0}")]
    N(usize),
    /// `best_of < n`.
    #[error("best_of ({best_of}) must be >= n ({n})")]
    BestOf {
        /// Requested best_of.
        best_of: usize,
        /// Requested n.
        n: usize,
    },
    /// `best_of > 1` with greedy sampling.
    #[error("best_of must be 1 when using greedy sampling")]
    GreedyBestOf,
    /// `max_tokens < 1`.
    #[error("max_tokens must be >= 1, got {0}")]
    MaxTokens(usize),
    /// `min_tokens > max_tokens`.
    #[error("min_tokens ({min_tokens}) must be <= max_tokens ({max_tokens})")]
    MinTokens {
        /// Requested min_tokens.
        min_tokens: usize,
        /// Requested max_tokens.
        max_tokens: usize,
    },
}

/// Request-level sampling parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct SamplingParams {
    /// Number of output sequences to return.
    pub n: usize,
    /// Number of candidates generated; the top `n` are returned.
    pub best_of: usize,
    /// Penalty on tokens already present in the output `[-2, 2]`.
    pub presence_penalty: f32,
    /// Penalty scaled by token frequency `[-2, 2]`.
    pub frequency_penalty: f32,
    /// Multiplicative repetition penalty (> 0; 1.0 = off).
    pub repetition_penalty: f32,
    /// Softmax temperature (>= 0; 0 = greedy).
    pub temperature: f32,
    /// Nucleus sampling cumulative-probability cutoff `(0, 1]`.
    pub top_p: f32,
    /// Top-k cutoff (-1 disables, else >= 1).
    pub top_k: i32,
    /// Minimum token probability relative to the max `[0, 1]`.
    pub min_p: f32,
    /// Optional RNG seed for reproducible sampling.
    pub seed: Option<u64>,
    /// Stop strings that end generation (excluded from output).
    pub stop: Vec<String>,
    /// Stop token ids that end generation.
    pub stop_token_ids: Vec<u32>,
    /// Hard cap on generated tokens (None = model default).
    pub max_tokens: Option<usize>,
    /// Minimum tokens to generate before honoring stops/EOS.
    pub min_tokens: usize,
    /// Number of per-token logprobs to return.
    pub logprobs: Option<usize>,
    /// Keep generating past the EOS token.
    pub ignore_eos: bool,
}

impl Default for SamplingParams {
    fn default() -> Self {
        Self {
            n: 1,
            best_of: 1,
            presence_penalty: 0.0,
            frequency_penalty: 0.0,
            repetition_penalty: 1.0,
            temperature: 1.0,
            top_p: 1.0,
            top_k: -1,
            min_p: 0.0,
            seed: None,
            stop: Vec::new(),
            stop_token_ids: Vec::new(),
            max_tokens: Some(16),
            min_tokens: 0,
            logprobs: None,
            ignore_eos: false,
        }
    }
}

impl SamplingParams {
    /// Classify how tokens are drawn (vLLM `SamplingParams.sampling_type`).
    pub fn sampling_type(&self) -> SamplingType {
        if self.temperature < SAMPLING_EPS {
            SamplingType::Greedy
        } else if self.seed.is_some() {
            SamplingType::RandomSeed
        } else {
            SamplingType::Random
        }
    }

    /// Validate the parameters (vLLM `_verify_args` + `_verify_greedy_sampling`).
    pub fn validate(&self) -> Result<(), SamplingError> {
        if self.n < 1 {
            return Err(SamplingError::N(self.n));
        }
        if self.best_of < self.n {
            return Err(SamplingError::BestOf {
                best_of: self.best_of,
                n: self.n,
            });
        }
        if !(-2.0..=2.0).contains(&self.presence_penalty) {
            return Err(SamplingError::PresencePenalty(self.presence_penalty));
        }
        if !(-2.0..=2.0).contains(&self.frequency_penalty) {
            return Err(SamplingError::FrequencyPenalty(self.frequency_penalty));
        }
        if self.repetition_penalty <= 0.0 {
            return Err(SamplingError::RepetitionPenalty(self.repetition_penalty));
        }
        if self.temperature < 0.0 {
            return Err(SamplingError::Temperature(self.temperature));
        }
        if self.top_p <= 0.0 || self.top_p > 1.0 {
            return Err(SamplingError::TopP(self.top_p));
        }
        if self.top_k != -1 && self.top_k < 1 {
            return Err(SamplingError::TopK(self.top_k));
        }
        if !(0.0..=1.0).contains(&self.min_p) {
            return Err(SamplingError::MinP(self.min_p));
        }
        if let Some(max) = self.max_tokens {
            if max < 1 {
                return Err(SamplingError::MaxTokens(max));
            }
            if self.min_tokens > max {
                return Err(SamplingError::MinTokens {
                    min_tokens: self.min_tokens,
                    max_tokens: max,
                });
            }
        }
        // Greedy sampling cannot request more than one candidate.
        if self.temperature < SAMPLING_EPS && self.best_of > 1 {
            return Err(SamplingError::GreedyBestOf);
        }
        Ok(())
    }

    /// Build (and validate) from an OpenAI-compatible request shape.
    pub fn from_openai(req: OpenAiSampling) -> Result<Self, SamplingError> {
        let n = req.n.unwrap_or(1);
        let best_of = req.best_of.unwrap_or(n);
        let defaults = SamplingParams::default();
        let params = SamplingParams {
            n,
            best_of,
            presence_penalty: req.presence_penalty.unwrap_or(0.0),
            frequency_penalty: req.frequency_penalty.unwrap_or(0.0),
            repetition_penalty: req.repetition_penalty.unwrap_or(1.0),
            temperature: req.temperature.unwrap_or(1.0),
            top_p: req.top_p.unwrap_or(1.0),
            top_k: req.top_k.unwrap_or(-1),
            min_p: req.min_p.unwrap_or(0.0),
            seed: req.seed,
            stop: req.stop,
            stop_token_ids: req.stop_token_ids,
            max_tokens: req.max_tokens.or(defaults.max_tokens),
            min_tokens: req.min_tokens.unwrap_or(0),
            logprobs: req.logprobs,
            ignore_eos: req.ignore_eos.unwrap_or(false),
        };
        params.validate()?;
        Ok(params)
    }
}

/// OpenAI-compatible request fields that map onto [`SamplingParams`].
///
/// Every field is optional so the same struct serves `/v1/chat/completions`
/// and `/v1/completions`; unset fields take vLLM defaults.
#[derive(Debug, Clone, Default)]
pub struct OpenAiSampling {
    /// `temperature`.
    pub temperature: Option<f32>,
    /// `top_p`.
    pub top_p: Option<f32>,
    /// `top_k` (vLLM extension).
    pub top_k: Option<i32>,
    /// `min_p` (vLLM extension).
    pub min_p: Option<f32>,
    /// `n`.
    pub n: Option<usize>,
    /// `best_of`.
    pub best_of: Option<usize>,
    /// `presence_penalty`.
    pub presence_penalty: Option<f32>,
    /// `frequency_penalty`.
    pub frequency_penalty: Option<f32>,
    /// `repetition_penalty` (vLLM extension).
    pub repetition_penalty: Option<f32>,
    /// `max_tokens`.
    pub max_tokens: Option<usize>,
    /// `min_tokens` (vLLM extension).
    pub min_tokens: Option<usize>,
    /// `seed`.
    pub seed: Option<u64>,
    /// `stop` strings.
    pub stop: Vec<String>,
    /// `stop_token_ids` (vLLM extension).
    pub stop_token_ids: Vec<u32>,
    /// `logprobs` count.
    pub logprobs: Option<usize>,
    /// `ignore_eos` (vLLM extension).
    pub ignore_eos: Option<bool>,
}
