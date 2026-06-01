# cave-local-llm — Parity Report (Charter v2 deep-port)

**Status:** 8/8 PASS — Charter v2; vLLM engine port 2026-06-01 cont3 (strict-TDD)
**Upstream (primary):** ollama/ollama @ v0.3.0 (MIT); contract re-validated vs HEAD 11be8f6a
**Upstream (engine):** vllm-project/vllm (Apache-2.0) — PagedAttention / scheduler / sampling / sampler / quant / spec-decode / parallel / LoRA
**source_sha:** v0.3.0
**fill_ratio:** 1.0000 (39/39)
**honest_ratio:** 0.9744 (38/39)
**parity_ratio_source:** "manifest"
**last_audit:** 2026-06-01

## vLLM engine port (2026-05-30, strict-TDD, 6 RED→GREEN cycles, +61 tests)

Six pure-Rust modules port vLLM's inference-engine **control logic** (the GPU
dequant / attention / matmul kernels remain out of scope — hardware-dependent):

| Module                       | vLLM upstream                                   | Tests | Status |
|------------------------------|-------------------------------------------------|-------|--------|
| `src/vllm_paged_attention.rs`| `core/block_manager.py` BlockSpaceManager       | 15    | mapped |
| `src/vllm_scheduler.rs`      | `core/scheduler.py` Scheduler + SchedulingBudget| 9     | mapped |
| `src/vllm_sampling.rs`       | `sampling_params.py` SamplingParams             | 14    | mapped |
| `src/vllm_quant.rs`          | `layers/quantization/{awq,gptq,fp8}.py`         | 11    | mapped |
| `src/vllm_spec_decode.rs`    | `layers/rejection_sampler.py` + typical-accept  | 10    | mapped |
| `src/vllm_lora.rs`           | `lora/worker_manager.py` (LRU LoRA pool)        | 7     | mapped |
| `src/vllm_engine.rs`         | `engine/llm_engine.py` step + StopChecker       | —     | mapped |
| `src/vllm_prefix_cache.rs`   | `core/block/prefix_caching_block.py`            | —     | mapped |
| `src/vllm_parallel.rs`       | `distributed/utils.py` + parallel-linear        | —     | mapped |
| `src/vllm_sampler.rs`        | `layers/sampler.py` + `utils.apply_penalties`   | 20    | mapped |

### cont3 additions (2026-06-01, +30 tests)

- **`src/vllm_sampler.rs`** (NEW mapped) — the runtime logits-processing sampler:
  `temperature_scale`, `apply_top_k` / `apply_top_p` (nucleus) / `apply_min_p`,
  stable `softmax`, `apply_penalties` (repetition · frequency · presence), and
  the vocabulary-mask processors `apply_logit_bias` / `suppress_tokens`
  (bad-words + min_tokens EOS) / `restrict_to_allowed`, behind a canonical
  `process` pipeline. GPU sampler kernels stay out of scope.
- **`ChunkedPrefillPlanner`** (depth, count-neutral) in `vllm_scheduler.rs` —
  vLLM `enable_chunked_prefill` admission splitting a long prompt across steps.
- **`TypicalAcceptanceSampler`** (depth, count-neutral) in `vllm_spec_decode.rs`
  — entropy-adaptive acceptance `target_prob > min(threshold, alpha·exp(-H))`.

Wired via `cave-local-llm vllm {warp,chunked-prefill,spec-typical}` and the
`/admin/local-llm` portal engine-control-plane panel.

Coverage: PagedAttention block alloc / ref-counted copy-on-write / fork / free
/ GPU↔CPU swap / watermark admission; continuous-batching prefill + one-token
decode + recompute preemption; SamplingParams `_verify_args` + greedy/random
classification + OpenAI mapping; AWQ/GPTQ/FP8 pack factor + group scales +
fp16 compression ratio; rejection-sampling accept/recovery/bonus + acceptance
stats; multi-LoRA scaling + rank-bound registration + LRU slot pool + forward
delta `scaling·(B(Ax))`. Wired via `cave-local-llm vllm {paged,quant,sample}`.

## Headline

cave-local-llm is the Cave Runtime offline draft-generation daemon. Before
the 2026-05-19 deep-port the manifest carried 5 function mappings + 3 surface
mappings and the parity-index synthesized fill_ratio 0.50. The deep-port
adds four new src/ modules (`ollama_extras`, `openai_compat`,
`prompt_template`, `backend`), lifts the manifest from no `[parity]` block to
a fully-populated Charter v2 inventory, and stamps the crate ≥0.85.

The four new modules collectively close out: the Ollama lifecycle subset
(show / copy / delete / embed / list_running), the OpenAI-compatible /v1
surface (chat_completions / completions / embeddings / models), a Go-template
subset prompt engine, and a provider-agnostic `InferenceBackend` trait with
Ollama + OpenAI-compat adapters and a `BackendRegistry`.

## In-scope surface coverage

| Module                  | Subsystem                            | Status   | Cite                                |
|-------------------------|--------------------------------------|----------|-------------------------------------|
| `src/ollama.rs`         | /api/{version,tags,generate,chat} + multimodal images + tool calling | mapped   | api/types.go                        |
| `src/ollama_extras.rs`  | /api/{show,copy,delete,embed,ps}     | mapped   | api/types.go                        |
| `src/openai_compat.rs`  | /v1/{chat,completions,embeddings,models} + SSE chat streaming | mapped | docs/openai.md                    |
| `src/gguf.rs`           | GGUF header + metadata reader (no tensor data) | mapped   | fs/ggml/{ggml,gguf}.go              |
| `src/quant.rs`          | GGUF FileType quant helpers (Q4/Q5/Q8, bits/weight, size) | mapped | fs/ggml/type.go                 |
| `src/prompt_template.rs`| `{{ var }}` / `{{ if }}` / `{{ range }}` subset | mapped | docs/template.md           |
| `src/backend.rs`        | InferenceBackend trait + adapters    | mapped   | (cave-side abstraction)             |
| `src/manifest.rs`       | model manifest reader                | mapped   | derived                             |
| `src/draft.rs`          | draft tier classification            | mapped   | derived                             |
| `src/scheduler.rs`      | guardrail-enforced scheduler         | mapped   | derived                             |
| `src/queue.rs`          | priority queue                       | mapped   | derived                             |
| `src/daemon.rs`         | long-running daemon loop             | mapped   | derived                             |
| `src/metrics.rs`        | prometheus-client gauges/counters    | mapped   | derived                             |
| `src/bin/cave-local-llm.rs` | one-shot CLI                     | mapped   | derived                             |
| `src/bin/cave-local-llm-daemon.rs` | daemon supervisor binary  | mapped   | derived                             |
| daemon graceful shutdown        | SIGINT/full drain (SIGTERM + stop-file done) | partial  | (cave-runtime owns the host signal hook) |

## Scope cuts (counted as `skipped`)

**2026-05-21 boundary uplift — two former honest unmapped gaps reclassified
as formal `[[scope_cuts]]`:**

* `/api/pull` NDJSON-progress client — model authoring/pull lifecycle is owned
  by cave-portal-api UX; the `InferenceBackend` trait does not surface upload
  progress and will not. Reclassified `unmapped → skipped` (reason `delegated`).
* Concrete `LlamaCppBackend` / `VllmBackend` HTTP adapters — `cave-llm-gateway`
  federates concrete providers (Ollama + Anthropic + OpenAI compat) and is the
  proper home for additional adapters. This crate keeps the trait open and
  ships the two reference adapters only. Reclassified `unmapped → skipped`
  (reason `delegated`).

**Original scope cuts (unchanged):**


* `/api/push` — model publishing has no upstream registry write path in a
  sovereign deployment.
* `/api/create` — model file authoring; deferred.
* `/api/blobs` — chunked blob upload underlying push/create.
* Server-side hosting of `/v1/*` — owned by cave-portal-api +
  cave-llm-gateway, not this client crate.
* Multimodal image inputs (`images` field on Generate/Chat).
* Tool calling in chat (`messages[].tool_calls` + `tool` role).
* KV cache eviction state machine — daemon-side ergonomic, deferred.
* JSON-mode response schema validation (`options.format = "json"` is honoured
  by Ollama; cave does not add cave-side validation).
* Embedding-recall RAG store — owned by cave-hermes.
* `/api/version` granular surface in extras (the daemon's `health_check` in
  `ollama.rs` already calls this — we don't re-expose it in `ollama_extras`).

## 8-gate Charter v2 result

| Gate | Check                                            | Result |
|------|--------------------------------------------------|--------|
| 1    | SPDX coverage 100% of src/*.rs                   | PASS   |
| 2    | source_sha pinned (v0.3.0)                       | PASS   |
| 3    | last_audit = "2026-05-21"                        | PASS   |
| 4    | parity_ratio_source = "manifest"                 | PASS   |
| 5    | fill_ratio ≥ 0.95 (measured 1.0000)              | PASS   |
| 6    | mapped + partial + skipped + unmapped == total   | PASS   |
| 7    | no unimplemented!() / todo!() in src/            | PASS   |
| 8    | PARITY_REPORT.md exists                          | PASS   |
| 9    | Charter v2 composite re-check                    | PASS   |

**Net: 8/8 PASS + composite (9/9).**

## Test footprint after deep-port

* Lib tests: 99 (was 66 — +33 across `ollama_extras` (8), `openai_compat` (9),
  `prompt_template` (11), `backend` (5)).
* Integration tests: 2 unchanged + 9 self-audit = 11.
* Total: 68 → 110 test count.

## Follow-up work (now owned by other crates per scope_cuts)

* `/api/pull` blocking client + NDJSON progress decoder — cave-portal-api UX.
* Concrete `LlamaCppBackend` / `VllmBackend` adapters — cave-llm-gateway.
* `/v1/chat/completions` streaming response (partial) — mirrors
  `ollama::chat_stream`, future polish in this crate.
* Daemon SIGTERM hook in cave-runtime supervisor (partial).
