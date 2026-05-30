// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Continuous-batching scheduler — a pure-Rust port of vLLM's iteration-level
//! scheduler (vllm-project/vllm `vllm/core/scheduler.py`, Apache-2.0).
//!
//! Unlike static batching, vLLM schedules at **every decode iteration**: a
//! sequence that finishes immediately frees its KV slot and a waiting request
//! is admitted in its place, keeping the GPU saturated. Admission is gated by
//! a [`SchedulingBudget`] (a token budget — `max_num_batched_tokens` — and a
//! sequence budget — `max_num_seqs`) and by PagedAttention block availability
//! (via [`crate::vllm_paged_attention::BlockSpaceManager`]). When the block
//! pool is exhausted mid-decode, the lowest-priority running sequence is
//! **preempted by recompute** (its blocks are freed and it returns to the
//! front of the waiting queue) so a higher-priority sequence can proceed.
//!
//! One sequence per group is modelled (the common serving case); beam-search
//! multi-sequence groups are out of scope for the sovereign control plane.

use std::collections::VecDeque;

use crate::vllm_paged_attention::{AllocStatus, BlockSpaceManager};

/// Lifecycle state of a scheduled request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeqStatus {
    /// Queued, not yet admitted.
    Waiting,
    /// Admitted and holding KV blocks.
    Running,
    /// Reached `max_tokens` and released.
    Finished,
}

/// Token + sequence admission budget for one scheduler step.
#[derive(Debug, Clone)]
pub struct SchedulingBudget {
    token_budget: usize,
    max_num_seqs: usize,
    num_batched_tokens: usize,
    num_curr_seqs: usize,
}

impl SchedulingBudget {
    /// New budget with a per-step token cap and concurrent-sequence cap.
    pub fn new(token_budget: usize, max_num_seqs: usize) -> Self {
        Self {
            token_budget,
            max_num_seqs,
            num_batched_tokens: 0,
            num_curr_seqs: 0,
        }
    }

    /// True if `num_new_tokens` and `num_new_seqs` both still fit.
    pub fn can_schedule(&self, num_new_tokens: usize, num_new_seqs: usize) -> bool {
        self.num_batched_tokens + num_new_tokens <= self.token_budget
            && self.num_curr_seqs + num_new_seqs <= self.max_num_seqs
    }

    /// Charge `n` tokens against the budget.
    pub fn add_num_batched_tokens(&mut self, n: usize) {
        self.num_batched_tokens += n;
    }

    /// Charge `n` sequences against the budget.
    pub fn add_num_seqs(&mut self, n: usize) {
        self.num_curr_seqs += n;
    }

    /// Tokens still available this step.
    pub fn remaining_token_budget(&self) -> usize {
        self.token_budget.saturating_sub(self.num_batched_tokens)
    }

    /// Tokens charged so far this step.
    pub fn num_batched_tokens(&self) -> usize {
        self.num_batched_tokens
    }

    /// Sequences charged so far this step.
    pub fn num_curr_seqs(&self) -> usize {
        self.num_curr_seqs
    }
}

/// Static scheduler configuration.
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Per-step token budget (`max_num_batched_tokens`).
    pub max_num_batched_tokens: usize,
    /// Max concurrently-running sequences (`max_num_seqs`).
    pub max_num_seqs: usize,
    /// PagedAttention block size (tokens per block).
    pub block_size: usize,
    /// Total GPU KV blocks.
    pub num_gpu_blocks: usize,
}

/// One scheduled sequence (single-sequence group).
#[derive(Debug, Clone)]
struct SeqGroup {
    id: u64,
    prompt_tokens: usize,
    max_tokens: usize,
    generated: usize,
    status: SeqStatus,
}

impl SeqGroup {
    fn total_tokens(&self) -> usize {
        self.prompt_tokens + self.generated
    }
    fn is_finished(&self) -> bool {
        self.generated >= self.max_tokens
    }
}

/// The set of decisions produced by a single [`Scheduler::schedule`] call.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SchedulerOutput {
    /// Requests admitted as prefill this step.
    pub prefill: Vec<u64>,
    /// Running requests decoding one token this step.
    pub decode: Vec<u64>,
    /// Running requests preempted (recompute) this step.
    pub preempted: Vec<u64>,
    /// Requests that reached `max_tokens` and were released.
    pub finished: Vec<u64>,
    /// Total tokens batched this step (prefill prompts + 1/decode-seq).
    pub num_batched_tokens: usize,
}

/// Continuous-batching scheduler over a PagedAttention block pool.
#[derive(Debug)]
pub struct Scheduler {
    config: SchedulerConfig,
    block_manager: BlockSpaceManager,
    waiting: VecDeque<SeqGroup>,
    running: VecDeque<SeqGroup>,
    next_id: u64,
}

impl Scheduler {
    /// Build a scheduler with a fresh GPU block pool (no CPU swap pool).
    pub fn new(config: SchedulerConfig) -> Self {
        let block_manager =
            BlockSpaceManager::new(config.block_size, config.num_gpu_blocks, 0, 0.0);
        Self {
            config,
            block_manager,
            waiting: VecDeque::new(),
            running: VecDeque::new(),
            next_id: 0,
        }
    }

    /// Enqueue a request; returns its assigned id.
    pub fn add_request(&mut self, prompt_tokens: usize, max_tokens: usize) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.waiting.push_back(SeqGroup {
            id,
            prompt_tokens,
            max_tokens,
            generated: 0,
            status: SeqStatus::Waiting,
        });
        id
    }

    /// Running sequences.
    pub fn num_running(&self) -> usize {
        self.running.len()
    }

    /// Waiting sequences.
    pub fn num_waiting(&self) -> usize {
        self.waiting.len()
    }

    /// Current status of a request (None if reaped/unknown).
    pub fn status_of(&self, id: u64) -> Option<SeqStatus> {
        if let Some(g) = self.running.iter().find(|g| g.id == id) {
            return Some(g.status);
        }
        if let Some(g) = self.waiting.iter().find(|g| g.id == id) {
            return Some(g.status);
        }
        None
    }

    /// Record that the model emitted one token for a decoded running sequence.
    pub fn record_generation(&mut self, id: u64) {
        if let Some(g) = self.running.iter_mut().find(|g| g.id == id) {
            g.generated += 1;
        }
    }

    /// Run one scheduler iteration: reap finished, decode running (preempting
    /// on block exhaustion), then admit waiting requests as prefill.
    pub fn schedule(&mut self) -> SchedulerOutput {
        let mut out = SchedulerOutput::default();
        let mut budget = SchedulingBudget::new(
            self.config.max_num_batched_tokens,
            self.config.max_num_seqs,
        );

        // (1) Reap finished sequences and free their blocks.
        let mut still_running: VecDeque<SeqGroup> = VecDeque::new();
        while let Some(g) = self.running.pop_front() {
            if g.is_finished() {
                self.block_manager.free(g.id);
                out.finished.push(g.id);
            } else {
                still_running.push_back(g);
            }
        }
        self.running = still_running;

        // (2) Decode running sequences, reserving one KV slot each. Preempt
        //     the lowest-priority (last) not-yet-decoded sequence on OOM.
        let mut preempted_this_step: Vec<u64> = Vec::new();
        let mut decoded: VecDeque<SeqGroup> = VecDeque::new();
        loop {
            let g = match self.running.pop_front() {
                Some(g) => g,
                None => break,
            };
            if !budget.can_schedule(1, 1) {
                // Out of token/seq budget: defer back to the front for next step.
                self.running.push_front(g);
                break;
            }
            let want = g.total_tokens() + 1;
            match self.block_manager.append_slot(g.id, want) {
                Ok(_) => {
                    budget.add_num_batched_tokens(1);
                    budget.add_num_seqs(1);
                    out.decode.push(g.id);
                    decoded.push_back(g);
                }
                Err(_) => {
                    // Block pool exhausted. Preempt the lowest-priority victim:
                    // the last sequence still queued behind us, else self.
                    if let Some(victim) = self.running.pop_back() {
                        self.block_manager.free(victim.id);
                        preempted_this_step.push(victim.id);
                        out.preempted.push(victim.id);
                        let mut v = victim;
                        v.status = SeqStatus::Waiting;
                        v.generated = 0; // recompute from prompt
                        self.waiting.push_front(v);
                        // Retry the current sequence now that room may exist.
                        self.running.push_front(g);
                    } else {
                        self.block_manager.free(g.id);
                        preempted_this_step.push(g.id);
                        out.preempted.push(g.id);
                        let mut v = g;
                        v.status = SeqStatus::Waiting;
                        v.generated = 0;
                        self.waiting.push_front(v);
                    }
                }
            }
        }
        // Decoded sequences keep running; charge their seqs into the budget.
        self.running = decoded;

        // (3) Admit waiting requests as prefill while budget + blocks allow.
        //     Sequences preempted this very step are not re-admitted now.
        let mut deferred: VecDeque<SeqGroup> = VecDeque::new();
        while let Some(mut g) = self.waiting.pop_front() {
            if preempted_this_step.contains(&g.id) {
                deferred.push_back(g);
                continue;
            }
            let prompt = g.prompt_tokens;
            if !budget.can_schedule(prompt, 1) {
                self.waiting.push_front(g);
                break;
            }
            if self.block_manager.can_allocate(prompt) != AllocStatus::Ok {
                // Cannot fit right now; stop admitting (preserve order).
                self.waiting.push_front(g);
                break;
            }
            self.block_manager
                .allocate(g.id, prompt)
                .expect("can_allocate checked above");
            budget.add_num_batched_tokens(prompt);
            budget.add_num_seqs(1);
            g.status = SeqStatus::Running;
            out.prefill.push(g.id);
            self.running.push_back(g);
        }
        // Restore any deferred (preempted-this-step) sequences to the front.
        while let Some(g) = deferred.pop_back() {
            self.waiting.push_front(g);
        }

        out.num_batched_tokens = budget.num_batched_tokens();
        out
    }
}
