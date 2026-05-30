// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-local-llm — offline draft-generation daemon using Ollama.
//!
//! Phase 1: Ollama client, manifest reader, draft format, metrics, CLI binary.
//! Phase 3: Priority queue, scheduler guardrails, 24/7 daemon loop.

/// Re-exports the daemon module for managing the main application lifecycle.
pub mod daemon;

/// Re-exports the draft module for handling LLM draft generation formats.
pub mod draft;

/// Re-exports the manifest module for reading configuration and model manifests.
pub mod manifest;

/// Re-exports the metrics module for collecting and exposing system metrics.
pub mod metrics;

/// Re-exports the Ollama client module for communicating with the local LLM server.
pub mod ollama;

/// Lifecycle/extras client (show / pull / copy / delete / embed / ps).
pub mod ollama_extras;

/// OpenAI-compatible client (chat / completions / embeddings / models).
pub mod openai_compat;

/// Prompt template engine — substitution + if + range subset of Go-template.
pub mod prompt_template;

/// Async inference backend trait + Ollama + OpenAI-compat adapters + registry.
pub mod backend;

/// Re-exports the queue module for managing priority-based task scheduling.
pub mod queue;

/// Re-exports the scheduler module for enforcing guardrails and execution policies.
pub mod scheduler;

/// Pure-Rust GGUF header + metadata reader (no tensor data, no inference).
pub mod gguf;

/// GGUF quantization file-type helpers (Q4/Q5/Q8 …) — names, bits/weight, size.
pub mod quant;

/// vLLM PagedAttention block manager — KV-block alloc, ref-counted CoW, swap.
pub mod vllm_paged_attention;

/// vLLM continuous-batching scheduler — budget, prefill/decode, preemption.
pub mod vllm_scheduler;

/// vLLM SamplingParams — sampling contract, validation, OpenAI mapping.
pub mod vllm_sampling;

/// vLLM weight-quant configs — AWQ/GPTQ/FP8 layout, pack factor, ratios.
pub mod vllm_quant;

/// vLLM speculative-decode rejection sampler — accept/recovery/bonus + stats.
pub mod vllm_spec_decode;

/// vLLM multi-LoRA manager — request scaling, slot pool + LRU, forward delta.
pub mod vllm_lora;
