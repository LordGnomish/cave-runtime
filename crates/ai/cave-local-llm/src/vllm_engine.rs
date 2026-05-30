// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD port of vLLM's `LLMEngine` continuous-batching step loop and the
// output-processor `StopChecker` (vllm-project/vllm `vllm/engine/llm_engine.py`
// and `vllm/engine/output_processor/stop_checker.py`, Apache-2.0).
//
// This module is the orchestration seam that ties three already-ported
// subsystems together:
//   * [`crate::vllm_paged_attention::BlockSpaceManager`] for KV admission and
//     per-token slot growth,
//   * [`crate::vllm_scheduler::SchedulingBudget`] for `max_num_seqs` /
//     `max_num_batched_tokens` gating,
//   * [`crate::vllm_sampling::SamplingParams`] for the per-request stop
//     contract (eos / stop-token-ids / min-tokens / max-tokens / ignore-eos),
// and adds the output-processing layer vLLM calls `process_outputs`: append the
// sampled token, run the `StopChecker`, and emit a [`RequestOutput`] carrying a
// finish reason.
//
// The model itself stays behind the [`StepModel`] trait — vLLM's model-executor
// boundary. The engine never touches weights; it batches scheduled sequences,
// hands their token views to the model, and processes whatever token the model
// sampled. This keeps the continuous-batching control flow testable with
// deterministic stub models and leaves real GPU execution to the host (which is
// why concrete model runners remain a documented scope-cut).

use crate::vllm_paged_attention::{AllocStatus, BlockSpaceManager};
use crate::vllm_sampling::SamplingParams;
use crate::vllm_scheduler::SchedulingBudget;
use std::collections::VecDeque;

/// A read-only view of one scheduled sequence handed to the model each step.
///
/// Mirrors the per-request slice of vLLM's `ModelRunnerInput`: the immutable
/// prompt and the output tokens generated so far. The model returns exactly one
/// next token per `SeqView` in the batch.
pub struct SeqView<'a> {
    /// Caller-assigned request id.
    pub id: u64,
    /// The original prompt token ids (never mutated).
    pub prompt_tokens: &'a [u32],
    /// Output tokens generated so far (grows by one per honored step).
    pub output_tokens: &'a [u32],
}

/// The model-executor seam. An implementation maps a batch of sequence views to
/// one next-token id per sequence, preserving batch order.
pub trait StepModel {
    /// Sample one next token per batched sequence (aligned to `batch` order).
    fn step(&mut self, batch: &[SeqView<'_>]) -> Vec<u32>;
}

/// Why a sequence stopped generating — vLLM's `RequestOutput.finish_reason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishReason {
    /// An EOS token or a configured stop-token-id was emitted.
    Stop,
    /// The `max_tokens` length cap (or model context limit) was reached.
    Length,
}

/// Per-step output for one sequence — vLLM's `RequestOutput`.
#[derive(Debug, Clone)]
pub struct RequestOutput {
    /// The request this output belongs to.
    pub request_id: u64,
    /// Token(s) produced for this sequence this step (zero or one here).
    ///
    /// Empty when the step's sampled token was an EOS that terminated the
    /// sequence (EOS is not surfaced in the output stream), one token
    /// otherwise — including a stop-token-id, which vLLM *does* include.
    pub new_token_ids: Vec<u32>,
    /// Whether the sequence finished this step.
    pub finished: bool,
    /// Set iff `finished`.
    pub finish_reason: Option<FinishReason>,
}

/// Static engine configuration (the subset of vLLM's `*Config` that drives the
/// scheduling and KV-admission control flow).
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Per-step token budget (`max_num_batched_tokens`).
    pub max_num_batched_tokens: usize,
    /// Max concurrently-running sequences (`max_num_seqs`).
    pub max_num_seqs: usize,
    /// PagedAttention block size (tokens per block).
    pub block_size: usize,
    /// Total GPU KV blocks.
    pub num_gpu_blocks: usize,
    /// The model's end-of-sequence token id.
    pub eos_token_id: u32,
}

/// Errors returned when admitting a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineError {
    /// The prompt can never fit the KV pool, regardless of free space.
    PromptTooLong {
        /// Prompt length in tokens.
        prompt_tokens: usize,
        /// Blocks the pool holds in total.
        pool_blocks: usize,
    },
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineError::PromptTooLong {
                prompt_tokens,
                pool_blocks,
            } => write!(
                f,
                "prompt of {prompt_tokens} tokens exceeds the {pool_blocks}-block KV pool"
            ),
        }
    }
}

impl std::error::Error for EngineError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Waiting,
    Running,
}

#[derive(Debug)]
struct Seq {
    id: u64,
    prompt: Vec<u32>,
    output: Vec<u32>,
    params: SamplingParams,
    phase: Phase,
}

impl Seq {
    /// Total token length (prompt + generated).
    fn len(&self) -> usize {
        self.prompt.len() + self.output.len()
    }
}

/// The outcome of the [`StopChecker`] for one sampled token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopAction {
    /// Append the token and keep going.
    Continue,
    /// Append the token, then finish with the given reason.
    AppendThenFinish(FinishReason),
    /// Do not append the token (EOS); finish with `Stop`.
    FinishNoAppend,
}

/// Faithful port of vLLM's `StopChecker.maybe_stop_sequence`, token-level only.
///
/// `would_be_len` is the output length *after* appending this token. Stop
/// signals (EOS, stop-token-ids) are suppressed until `min_tokens` is reached;
/// the EOS token is consumed (not appended) while a stop-token-id is included.
fn check_stop(
    token: u32,
    would_be_len: usize,
    params: &SamplingParams,
    eos_token_id: u32,
) -> StopAction {
    let min_ok = would_be_len >= params.min_tokens;

    // EOS — honored unless ignored, and only past min_tokens. Not appended.
    if min_ok && !params.ignore_eos && token == eos_token_id {
        return StopAction::FinishNoAppend;
    }
    // Explicit stop-token-ids — included in the output, then stop.
    if min_ok && params.stop_token_ids.contains(&token) {
        return StopAction::AppendThenFinish(FinishReason::Stop);
    }
    // Length cap — checked after appending.
    if let Some(max) = params.max_tokens {
        if would_be_len >= max {
            return StopAction::AppendThenFinish(FinishReason::Length);
        }
    }
    StopAction::Continue
}

/// Continuous-batching engine over a PagedAttention KV pool.
pub struct LLMEngine<M: StepModel> {
    config: EngineConfig,
    model: M,
    block_manager: BlockSpaceManager,
    waiting: VecDeque<Seq>,
    running: VecDeque<Seq>,
}

impl<M: StepModel> LLMEngine<M> {
    /// Build an engine with a fresh KV pool and the given model executor.
    pub fn new(config: EngineConfig, model: M) -> Self {
        // CPU swap space mirrors GPU here; engine-level swap is exercised by the
        // block manager's own tests, not the step loop.
        let block_manager =
            BlockSpaceManager::new(config.block_size, config.num_gpu_blocks, config.num_gpu_blocks, 0.0);
        Self {
            config,
            model,
            block_manager,
            waiting: VecDeque::new(),
            running: VecDeque::new(),
        }
    }

    /// Enqueue a request, returning an error if its prompt can never fit.
    pub fn try_add_request(
        &mut self,
        request_id: u64,
        prompt_tokens: Vec<u32>,
        params: SamplingParams,
    ) -> Result<(), EngineError> {
        if self.block_manager.can_allocate(prompt_tokens.len()) == AllocStatus::Never {
            return Err(EngineError::PromptTooLong {
                prompt_tokens: prompt_tokens.len(),
                pool_blocks: self.config.num_gpu_blocks,
            });
        }
        self.waiting.push_back(Seq {
            id: request_id,
            prompt: prompt_tokens,
            output: Vec::new(),
            params,
            phase: Phase::Waiting,
        });
        Ok(())
    }

    /// Enqueue a request, panicking on an over-long prompt.
    ///
    /// Use [`Self::try_add_request`] when the prompt length is untrusted.
    pub fn add_request(&mut self, request_id: u64, prompt_tokens: Vec<u32>, params: SamplingParams) {
        self.try_add_request(request_id, prompt_tokens, params)
            .expect("prompt fits the KV pool");
    }

    /// Whether any sequence is still waiting or running.
    pub fn has_unfinished_requests(&self) -> bool {
        !self.waiting.is_empty() || !self.running.is_empty()
    }

    /// Number of sequences currently in the running batch.
    pub fn num_running(&self) -> usize {
        self.running.len()
    }

    /// Number of sequences waiting for admission.
    pub fn num_waiting(&self) -> usize {
        self.waiting.len()
    }

    /// Execute one continuous-batching step:
    ///   1. admit waiting sequences as prefill while budget + KV blocks allow,
    ///   2. run the whole running batch through the model (one token each),
    ///   3. process each sampled token through the StopChecker, growing the KV
    ///      table, and finishing/freeing sequences that stop.
    ///
    /// Returns one [`RequestOutput`] per sequence that ran this step.
    pub fn step(&mut self) -> Vec<RequestOutput> {
        let mut budget =
            SchedulingBudget::new(self.config.max_num_batched_tokens, self.config.max_num_seqs);
        // Currently-running sequences each occupy a seq slot and one decode
        // token of this step's budget.
        for _ in &self.running {
            budget.add_num_seqs(1);
            budget.add_num_batched_tokens(1);
        }

        // (1) Admission — preserve FIFO order; stop at the first non-fit.
        while let Some(front) = self.waiting.front() {
            let prompt_len = front.prompt.len();
            if !budget.can_schedule(prompt_len, 1) {
                break;
            }
            if self.block_manager.can_allocate(prompt_len) != AllocStatus::Ok {
                break;
            }
            let mut seq = self.waiting.pop_front().expect("front existed");
            self.block_manager
                .allocate(seq.id, prompt_len)
                .expect("can_allocate checked Ok");
            budget.add_num_batched_tokens(prompt_len);
            budget.add_num_seqs(1);
            seq.phase = Phase::Running;
            self.running.push_back(seq);
        }

        if self.running.is_empty() {
            return Vec::new();
        }

        // (2) Build the batch and run the model executor.
        let batch: Vec<SeqView<'_>> = self
            .running
            .iter()
            .map(|s| SeqView {
                id: s.id,
                prompt_tokens: &s.prompt,
                output_tokens: &s.output,
            })
            .collect();
        let tokens = self.model.step(&batch);
        debug_assert_eq!(tokens.len(), self.running.len());

        // (3) Process outputs. Iterate the running queue, draining into either
        //     a kept-running queue or finishing (freeing KV blocks).
        let eos = self.config.eos_token_id;
        let mut outputs = Vec::with_capacity(self.running.len());
        let mut still_running: VecDeque<Seq> = VecDeque::with_capacity(self.running.len());
        let drained: Vec<Seq> = self.running.drain(..).collect();
        for (mut seq, token) in drained.into_iter().zip(tokens) {
            // Reserve a KV slot for the about-to-be-appended token.
            if self.block_manager.append_slot(seq.id, seq.len() + 1).is_err() {
                // KV pool exhausted: recompute-preempt this sequence (free and
                // requeue from its prompt). It produces no output this step.
                self.block_manager.free(seq.id);
                seq.output.clear();
                seq.phase = Phase::Waiting;
                self.waiting.push_front(seq);
                continue;
            }

            let would_be_len = seq.output.len() + 1;
            match check_stop(token, would_be_len, &seq.params, eos) {
                StopAction::Continue => {
                    seq.output.push(token);
                    outputs.push(RequestOutput {
                        request_id: seq.id,
                        new_token_ids: vec![token],
                        finished: false,
                        finish_reason: None,
                    });
                    still_running.push_back(seq);
                }
                StopAction::AppendThenFinish(reason) => {
                    seq.output.push(token);
                    outputs.push(RequestOutput {
                        request_id: seq.id,
                        new_token_ids: vec![token],
                        finished: true,
                        finish_reason: Some(reason),
                    });
                    self.block_manager.free(seq.id);
                }
                StopAction::FinishNoAppend => {
                    outputs.push(RequestOutput {
                        request_id: seq.id,
                        new_token_ids: Vec::new(),
                        finished: true,
                        finish_reason: Some(FinishReason::Stop),
                    });
                    self.block_manager.free(seq.id);
                }
            }
        }
        self.running = still_running;
        outputs
    }
}
