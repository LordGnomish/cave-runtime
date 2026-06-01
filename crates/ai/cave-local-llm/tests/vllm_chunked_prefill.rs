// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD port of vLLM's chunked-prefill admission (vllm-project/vllm
// `vllm/core/scheduler.py` `_schedule_chunked_prefill` / `_get_num_new_tokens`
// with `enable_chunked_prefill=True`, Apache-2.0).
//
// Without chunked prefill a long prompt must fit the per-step token budget
// whole. With it, a prompt is split across consecutive scheduler steps: each
// step greedily fills `max_num_batched_tokens` with prefill chunks — completing
// prompts that fit and emitting a partial chunk of the next that consumes the
// rest of the budget. A partially-prefilled sequence resumes next step.

use cave_local_llm::vllm_scheduler::{ChunkedPrefillPlanner, PrefillChunk};

#[test]
fn small_prompt_completes_in_one_step() {
    let mut p = ChunkedPrefillPlanner::new(100);
    p.add(1, 30);
    let step = p.step();
    assert_eq!(
        step,
        vec![PrefillChunk {
            id: 1,
            tokens: 30,
            done: true
        }]
    );
    assert!(p.is_empty());
}

#[test]
fn long_prompt_is_chunked_across_steps() {
    let mut p = ChunkedPrefillPlanner::new(40);
    p.add(7, 100);
    // 100 tokens / 40-budget -> 40, 40, 20.
    assert_eq!(
        p.step(),
        vec![PrefillChunk {
            id: 7,
            tokens: 40,
            done: false
        }]
    );
    assert_eq!(
        p.step(),
        vec![PrefillChunk {
            id: 7,
            tokens: 40,
            done: false
        }]
    );
    assert_eq!(
        p.step(),
        vec![PrefillChunk {
            id: 7,
            tokens: 20,
            done: true
        }]
    );
    assert!(p.is_empty());
}

#[test]
fn budget_packs_multiple_small_prompts_in_one_step() {
    let mut p = ChunkedPrefillPlanner::new(100);
    p.add(1, 30);
    p.add(2, 40);
    let step = p.step();
    assert_eq!(step.len(), 2);
    assert!(step.iter().all(|c| c.done));
    assert_eq!(step[0].id, 1);
    assert_eq!(step[1].id, 2);
    assert!(p.is_empty());
}

#[test]
fn partial_chunk_consumes_rest_of_budget_then_yields() {
    let mut p = ChunkedPrefillPlanner::new(50);
    p.add(1, 30); // completes (30), leaving 20 budget
    p.add(2, 100); // partial chunk of 20, not done
    let step = p.step();
    assert_eq!(
        step,
        vec![
            PrefillChunk { id: 1, tokens: 30, done: true },
            PrefillChunk { id: 2, tokens: 20, done: false },
        ]
    );
    assert!(!p.is_empty()); // seq 2 still prefilling
    assert_eq!(p.num_waiting(), 1);
}

#[test]
fn empty_planner_yields_nothing() {
    let mut p = ChunkedPrefillPlanner::new(64);
    assert_eq!(p.step(), Vec::<PrefillChunk>::new());
    assert!(p.is_empty());
}
