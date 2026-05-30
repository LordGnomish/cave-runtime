// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD port of vLLM's continuous-batching scheduler
// (vllm-project/vllm `vllm/core/scheduler.py`, Apache-2.0): a
// SchedulingBudget (token + sequence caps), waiting/running queues, prefill
// admission, iteration-level decode batching (one token per running seq),
// and recompute preemption when the PagedAttention block pool is exhausted.
//
// Lifecycle of a request (single sequence per group):
//   add_request -> waiting
//   schedule(): (1) reap finished, (2) decode running (reserve one KV slot
//   each, preempt on block exhaustion), (3) admit waiting as prefill while
//   budget + blocks allow.
//   record_generation(id): the model emitted one token for a decoded seq.

use cave_local_llm::vllm_scheduler::{Scheduler, SchedulerConfig, SchedulingBudget, SeqStatus};

// ── SchedulingBudget ────────────────────────────────────────────────────────

#[test]
fn budget_admits_within_both_caps() {
    let mut b = SchedulingBudget::new(100, 4);
    assert!(b.can_schedule(50, 2));
    b.add_num_batched_tokens(50);
    b.add_num_seqs(2);
    assert_eq!(b.remaining_token_budget(), 50);
    assert_eq!(b.num_curr_seqs(), 2);
}

#[test]
fn budget_rejects_when_token_cap_exceeded() {
    let mut b = SchedulingBudget::new(100, 4);
    b.add_num_batched_tokens(50);
    b.add_num_seqs(1);
    assert!(!b.can_schedule(60, 1)); // 50 + 60 > 100
    assert!(b.can_schedule(50, 1)); // 50 + 50 == 100 fits
}

#[test]
fn budget_rejects_when_seq_cap_exceeded() {
    let mut b = SchedulingBudget::new(100, 4);
    b.add_num_seqs(2);
    assert!(!b.can_schedule(1, 3)); // 2 + 3 > 4
    assert!(b.can_schedule(1, 2)); // 2 + 2 == 4 fits
}

// ── Prefill admission ───────────────────────────────────────────────────────

fn cfg(tok: usize, seqs: usize) -> SchedulerConfig {
    SchedulerConfig {
        max_num_batched_tokens: tok,
        max_num_seqs: seqs,
        block_size: 16,
        num_gpu_blocks: 256,
    }
}

#[test]
fn schedule_caps_prefill_by_max_num_seqs() {
    let mut s = Scheduler::new(cfg(100_000, 2));
    for _ in 0..3 {
        s.add_request(16, 5);
    }
    let out = s.schedule();
    assert_eq!(out.prefill.len(), 2, "seq cap limits the batch");
    assert_eq!(s.num_running(), 2);
    assert_eq!(s.num_waiting(), 1);
}

#[test]
fn schedule_caps_prefill_by_token_budget() {
    // token budget 40, prompts of 16 tokens each -> only 2 fit (32 <= 40, 48 > 40).
    let mut s = Scheduler::new(cfg(40, 100));
    for _ in 0..3 {
        s.add_request(16, 5);
    }
    let out = s.schedule();
    assert_eq!(out.prefill.len(), 2);
    assert_eq!(out.num_batched_tokens, 32);
}

// ── Decode step batches one token per running sequence ──────────────────────

#[test]
fn decode_step_appends_one_token_per_running_seq() {
    let mut s = Scheduler::new(cfg(100_000, 8));
    let a = s.add_request(16, 10);
    let _ = s.schedule(); // step 1: prefill a
    let out = s.schedule(); // step 2: decode a
    assert_eq!(out.decode, vec![a]);
    assert!(out.prefill.is_empty());
    assert_eq!(out.num_batched_tokens, 1, "decode batches one token per seq");
}

// ── Continuous batching: a finished slot admits waiting work mid-flight ─────

#[test]
fn finished_sequence_frees_slot_for_waiting_request() {
    let mut s = Scheduler::new(cfg(100_000, 2));
    let a = s.add_request(16, 1); // one decode token then done
    let b = s.add_request(16, 1);
    let _c = s.add_request(16, 5);

    let out1 = s.schedule(); // prefill a, b
    assert_eq!(out1.prefill.len(), 2);
    assert_eq!(s.num_waiting(), 1);

    let out2 = s.schedule(); // decode a, b
    assert_eq!(out2.decode.len(), 2);
    s.record_generation(a);
    s.record_generation(b); // both now at max_tokens

    let out3 = s.schedule(); // reap a, b -> admit c
    assert!(out3.finished.contains(&a) && out3.finished.contains(&b));
    assert_eq!(out3.prefill.len(), 1, "the waiting request is admitted");
    assert_eq!(s.num_running(), 1);
    assert_eq!(s.num_waiting(), 0);
}

// ── Recompute preemption when the block pool is exhausted ────────────────────

#[test]
fn block_exhaustion_preempts_lowest_priority_running_seq() {
    // block_size 1, only 3 GPU blocks. Two prompts of 1 token each occupy 2
    // blocks at prefill (1 free). On the next decode step seq A grabs the last
    // block; seq B then OOMs and is preempted back to waiting.
    let mut s = Scheduler::new(SchedulerConfig {
        max_num_batched_tokens: 100_000,
        max_num_seqs: 8,
        block_size: 1,
        num_gpu_blocks: 3,
    });
    let a = s.add_request(1, 10);
    let b = s.add_request(1, 10);

    let out1 = s.schedule(); // prefill a, b
    assert_eq!(out1.prefill.len(), 2);

    let out2 = s.schedule(); // decode: a grabs last block, b preempted
    assert_eq!(out2.decode, vec![a]);
    assert_eq!(out2.preempted, vec![b]);
    assert_eq!(s.status_of(b), Some(SeqStatus::Waiting));
    assert_eq!(s.num_running(), 1);
}

// ── Preempted sequence is re-admitted once capacity returns ─────────────────

#[test]
fn preempted_sequence_is_readmitted_after_capacity_frees() {
    let mut s = Scheduler::new(SchedulerConfig {
        max_num_batched_tokens: 100_000,
        max_num_seqs: 8,
        block_size: 1,
        num_gpu_blocks: 3,
    });
    let a = s.add_request(1, 1); // finishes after one decode token
    let b = s.add_request(1, 10);

    s.schedule(); // prefill a, b
    let out2 = s.schedule(); // decode a; b preempted (OOM)
    assert_eq!(out2.decode, vec![a]);
    assert_eq!(out2.preempted, vec![b]);
    s.record_generation(a); // a hits max_tokens

    let out3 = s.schedule(); // reap a -> blocks free -> re-admit b
    assert!(out3.finished.contains(&a));
    assert_eq!(out3.prefill, vec![b]);
    assert_eq!(s.status_of(b), Some(SeqStatus::Running));
}
