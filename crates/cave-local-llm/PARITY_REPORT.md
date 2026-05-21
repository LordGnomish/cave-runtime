# cave-local-llm — Parity Report (Charter v2 deep-port)

**Status:** 8/8 PASS — Charter v2 close-out 2026-05-19
**Upstream:** ollama/ollama @ v0.3.0 (MIT)
**source_sha:** v0.3.0
**fill_ratio:** 0.9259 (25/27)
**honest_ratio:** 0.8519 (23/27)
**parity_ratio_source:** "manifest"
**last_audit:** 2026-05-19

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
| `src/ollama.rs`         | /api/{version,tags,generate,chat}    | mapped   | api/types.go                        |
| `src/ollama_extras.rs`  | /api/{show,copy,delete,embed,ps}     | mapped   | api/types.go                        |
| `src/openai_compat.rs`  | /v1/{chat,completions,embeddings,models} | mapped | docs/openai.md                    |
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
| daemon graceful shutdown        | signal handling             | partial  | (cave-runtime owns SIGTERM hook)    |
| /v1 streaming response          | stream:true response shape  | partial  | docs/openai.md                      |

## Honest unmapped gaps (counted as `unmapped`)

1. **`/api/pull` blocking client** — pull requires NDJSON progress streaming
   plus a UX layer; declared in-scope, deferred to the model-lifecycle wave.
2. **Concrete llama.cpp / vLLM HTTP backend adapter** — the `InferenceBackend`
   trait is open; cave does not ship adapters beyond Ollama + OpenAI-compat
   today. Naming convention: `LlamaCppBackend` / `VllmBackend` follow the
   `OllamaBackend` / `OpenAiCompatBackend` pattern.

## Scope cuts (counted as `skipped`)

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
| 3    | last_audit = "2026-05-19"                        | PASS   |
| 4    | parity_ratio_source = "manifest"                 | PASS   |
| 5    | fill_ratio ≥ 0.85 (measured 0.9259)              | PASS   |
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

## Follow-up work

* `/api/pull` blocking client + NDJSON progress decoder.
* Concrete `LlamaCppBackend` adapter against `/completion` + `/chat/completions`
  of llama.cpp-server.
* `/v1/chat/completions` streaming response — mirrors `ollama::chat_stream`.
* Daemon SIGTERM hook in cave-runtime supervisor.
