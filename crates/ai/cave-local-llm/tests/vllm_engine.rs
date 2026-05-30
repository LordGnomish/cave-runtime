// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD port of vLLM's `LLMEngine` continuous-batching step loop and the
// output-processor `StopChecker` (vllm-project/vllm
// `vllm/engine/llm_engine.py` + `vllm/engine/output_processor/stop_checker.py`,
// Apache-2.0).
//
// The engine ties three already-ported subsystems together:
//   * PagedAttention block manager (`vllm_paged_attention`) for KV admission,
//   * the continuous-batching `SchedulingBudget` (`vllm_scheduler`),
//   * `SamplingParams` (`vllm_sampling`) for per-request stop contract,
// and adds the missing output-processing layer: append the sampled token,
// run the StopChecker, and emit a `RequestOutput` with a finish reason.
//
// The model itself is abstracted behind the `StepModel` seam (vLLM's
// model-executor boundary). Tests supply deterministic stub models so the
// control flow — prefill admission, iteration-level decode, EOS/stop-token/
// length termination, min-tokens suppression — can be exercised without GPUs.

use cave_local_llm::vllm_engine::{
    EngineConfig, FinishReason, LLMEngine, SeqView, StepModel,
};
use cave_local_llm::vllm_sampling::SamplingParams;

// ── Stub models (the model-executor seam) ────────────────────────────────────

/// Emits a fixed, per-request scripted stream of tokens; once a request's
/// script is exhausted it repeats the last token. Indexed by how many output
/// tokens the sequence already has.
struct ScriptedModel {
    scripts: std::collections::HashMap<u64, Vec<u32>>,
    /// Default token for requests with no script / past the end.
    filler: u32,
}

impl ScriptedModel {
    fn new(filler: u32) -> Self {
        Self {
            scripts: std::collections::HashMap::new(),
            filler,
        }
    }
    fn with(mut self, id: u64, toks: Vec<u32>) -> Self {
        self.scripts.insert(id, toks);
        self
    }
}

impl StepModel for ScriptedModel {
    fn step(&mut self, batch: &[SeqView<'_>]) -> Vec<u32> {
        batch
            .iter()
            .map(|s| {
                let n = s.output_tokens.len();
                self.scripts
                    .get(&s.id)
                    .and_then(|v| v.get(n).copied())
                    .unwrap_or(self.filler)
            })
            .collect()
    }
}

fn params_max(max_tokens: usize) -> SamplingParams {
    SamplingParams {
        max_tokens: Some(max_tokens),
        ..Default::default()
    }
}

// ── basic length-terminated decode ───────────────────────────────────────────

#[test]
fn single_request_decodes_until_max_tokens() {
    let cfg = EngineConfig {
        max_num_batched_tokens: 256,
        max_num_seqs: 8,
        block_size: 16,
        num_gpu_blocks: 64,
        eos_token_id: 0,
    };
    let model = ScriptedModel::new(7);
    let mut engine = LLMEngine::new(cfg, model);

    // prompt of 3 tokens, generate exactly 4.
    engine.add_request(1, vec![10, 11, 12], params_max(4));
    assert!(engine.has_unfinished_requests());

    let mut all: Vec<u32> = Vec::new();
    let mut finish = None;
    let mut steps = 0;
    while engine.has_unfinished_requests() {
        let outs = engine.step();
        for o in outs {
            all.extend_from_slice(&o.new_token_ids);
            if o.finished {
                finish = o.finish_reason;
            }
        }
        steps += 1;
        assert!(steps < 100, "diverged");
    }

    assert_eq!(all, vec![7, 7, 7, 7]);
    assert_eq!(finish, Some(FinishReason::Length));
    assert!(!engine.has_unfinished_requests());
}

// ── EOS termination ──────────────────────────────────────────────────────────

#[test]
fn eos_token_finishes_with_stop_reason() {
    let cfg = EngineConfig {
        max_num_batched_tokens: 256,
        max_num_seqs: 8,
        block_size: 16,
        num_gpu_blocks: 64,
        eos_token_id: 99,
    };
    // emits 5, 6, then EOS(99) — should stop after EOS, EOS not in output.
    let model = ScriptedModel::new(0).with(1, vec![5, 6, 99, 8, 8]);
    let mut engine = LLMEngine::new(cfg, model);
    engine.add_request(1, vec![1, 2], params_max(100));

    let mut all = Vec::new();
    let mut reason = None;
    while engine.has_unfinished_requests() {
        for o in engine.step() {
            all.extend_from_slice(&o.new_token_ids);
            if o.finished {
                reason = o.finish_reason;
            }
        }
    }
    assert_eq!(all, vec![5, 6]); // EOS itself is not appended to output
    assert_eq!(reason, Some(FinishReason::Stop));
}

#[test]
fn ignore_eos_keeps_generating_through_eos() {
    let cfg = EngineConfig {
        max_num_batched_tokens: 256,
        max_num_seqs: 8,
        block_size: 16,
        num_gpu_blocks: 64,
        eos_token_id: 99,
    };
    let model = ScriptedModel::new(0).with(1, vec![5, 99, 6, 7]);
    let mut engine = LLMEngine::new(cfg, model);
    let mut p = params_max(4);
    p.ignore_eos = true;
    engine.add_request(1, vec![1], p);

    let mut all = Vec::new();
    let mut reason = None;
    while engine.has_unfinished_requests() {
        for o in engine.step() {
            all.extend_from_slice(&o.new_token_ids);
            if o.finished {
                reason = o.finish_reason;
            }
        }
    }
    // EOS appended like any other token; stops on length.
    assert_eq!(all, vec![5, 99, 6, 7]);
    assert_eq!(reason, Some(FinishReason::Length));
}

// ── stop_token_ids ───────────────────────────────────────────────────────────

#[test]
fn stop_token_id_finishes_and_is_included() {
    let cfg = EngineConfig {
        max_num_batched_tokens: 256,
        max_num_seqs: 8,
        block_size: 16,
        num_gpu_blocks: 64,
        eos_token_id: 0,
    };
    let model = ScriptedModel::new(0).with(1, vec![5, 6, 42, 7]);
    let mut engine = LLMEngine::new(cfg, model);
    let mut p = params_max(100);
    p.stop_token_ids = vec![42];
    engine.add_request(1, vec![1], p);

    let mut all = Vec::new();
    let mut reason = None;
    while engine.has_unfinished_requests() {
        for o in engine.step() {
            all.extend_from_slice(&o.new_token_ids);
            if o.finished {
                reason = o.finish_reason;
            }
        }
    }
    // vLLM includes the stop token in the output (unlike EOS).
    assert_eq!(all, vec![5, 6, 42]);
    assert_eq!(reason, Some(FinishReason::Stop));
}

// ── min_tokens suppresses early stop ─────────────────────────────────────────

#[test]
fn min_tokens_suppresses_eos_and_stop_tokens() {
    let cfg = EngineConfig {
        max_num_batched_tokens: 256,
        max_num_seqs: 8,
        block_size: 16,
        num_gpu_blocks: 64,
        eos_token_id: 99,
    };
    // EOS at position 1 and stop-token 42 at position 2 must be ignored until
    // min_tokens (3) generated; real stop is the EOS at position 3.
    let model = ScriptedModel::new(0).with(1, vec![5, 99, 42, 99, 8]);
    let mut engine = LLMEngine::new(cfg, model);
    let mut p = params_max(100);
    p.min_tokens = 3;
    p.stop_token_ids = vec![42];
    engine.add_request(1, vec![1], p);

    let mut all = Vec::new();
    let mut reason = None;
    while engine.has_unfinished_requests() {
        for o in engine.step() {
            all.extend_from_slice(&o.new_token_ids);
            if o.finished {
                reason = o.finish_reason;
            }
        }
    }
    // positions 0,1,2 emitted (5, 99-as-text, 42-as-text) because < min_tokens;
    // EOS at position 3 honored.
    assert_eq!(all, vec![5, 99, 42]);
    assert_eq!(reason, Some(FinishReason::Stop));
}

// ── continuous batching of multiple requests ─────────────────────────────────

#[test]
fn two_requests_batch_and_finish_independently() {
    let cfg = EngineConfig {
        max_num_batched_tokens: 256,
        max_num_seqs: 8,
        block_size: 16,
        num_gpu_blocks: 64,
        eos_token_id: 0,
    };
    let model = ScriptedModel::new(0)
        .with(1, vec![1, 1])
        .with(2, vec![2, 2, 2, 2]);
    let mut engine = LLMEngine::new(cfg, model);
    engine.add_request(1, vec![10], params_max(2));
    engine.add_request(2, vec![20, 21], params_max(4));

    let mut got: std::collections::HashMap<u64, Vec<u32>> = std::collections::HashMap::new();
    let mut finished: std::collections::HashMap<u64, FinishReason> = Default::default();
    while engine.has_unfinished_requests() {
        for o in engine.step() {
            got.entry(o.request_id).or_default().extend(o.new_token_ids);
            if o.finished {
                finished.insert(o.request_id, o.finish_reason.unwrap());
            }
        }
    }

    assert_eq!(got[&1], vec![1, 1]);
    assert_eq!(got[&2], vec![2, 2, 2, 2]);
    assert_eq!(finished[&1], FinishReason::Length);
    assert_eq!(finished[&2], FinishReason::Length);
}

// ── max_num_seqs throttles concurrent admission ──────────────────────────────

#[test]
fn max_num_seqs_limits_concurrent_running() {
    let cfg = EngineConfig {
        max_num_batched_tokens: 1024,
        max_num_seqs: 1, // only one sequence runs at a time
        block_size: 16,
        num_gpu_blocks: 64,
        eos_token_id: 0,
    };
    let model = ScriptedModel::new(3);
    let mut engine = LLMEngine::new(cfg, model);
    engine.add_request(1, vec![1], params_max(2));
    engine.add_request(2, vec![2], params_max(2));

    // First step must admit only request 1 (budget caps to 1 running seq).
    let first = engine.step();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].request_id, 1);

    // Drain everything; both must still complete.
    let mut done = std::collections::HashSet::new();
    if first[0].finished {
        done.insert(1);
    }
    while engine.has_unfinished_requests() {
        for o in engine.step() {
            if o.finished {
                done.insert(o.request_id);
            }
        }
    }
    assert!(done.contains(&1) && done.contains(&2));
}

// ── prompt that exceeds the whole KV pool is rejected up front ───────────────

#[test]
fn oversized_prompt_is_rejected() {
    let cfg = EngineConfig {
        max_num_batched_tokens: 100_000,
        max_num_seqs: 8,
        block_size: 4,
        num_gpu_blocks: 2, // pool holds 8 tokens total
        eos_token_id: 0,
    };
    let model = ScriptedModel::new(1);
    let mut engine = LLMEngine::new(cfg, model);
    // 20-token prompt needs 5 blocks but only 2 exist.
    let err = engine.try_add_request(1, vec![0; 20], params_max(1));
    assert!(err.is_err());
    assert!(!engine.has_unfinished_requests());
}
