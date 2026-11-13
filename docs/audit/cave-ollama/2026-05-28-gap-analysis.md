# cave-ollama honest_ratio uplift — gap analysis

**Date:** 2026-05-28
**Upstream:** [`ollama/ollama`](https://github.com/ollama/ollama) (MIT) — baseline `/tmp/ollama-baseline` @ `11be8f6ac87479bbb0cb3370de9f46c678681b53`
**Cave crate:** `crates/ai/cave-local-llm` — the canonical Ollama deep-port (manifest `[upstream] org=ollama repo=ollama`; `cave-upstream/src/projects.rs` maps `Ollama → cave-local-llm` family).

> **Naming note.** There is no `crates/*/cave-ollama`. Per the repo rule *"orchestrator maps upstream → existing crate first"* (see `cave-vulns-is-defectdojo`, `cave-cilium` abort), Ollama's deep-port already lives in `cave-local-llm`. Creating a `cave-ollama` crate would be a forbidden new-crate. **This uplift extends `cave-local-llm`.**

## Upstream LOC (core surface, excluding `*_test.go`)

| Area | Dirs | Non-test Go LOC |
|---|---|---|
| API types + server + llm + fs + parser + template | `api/ server/ llm/ fs/ parser/ template/` | **25 618** |
| `api/types.go` (request/response contract) | | 1 343 |
| `server/routes.go` (HTTP surface) | | ~2 600 |
| `parser/parser.go` (Modelfile) | | 737 |
| `fs/ggml/{ggml,gguf,type}.go` (GGUF + quant) | | ~1 100 |

Ollama is a 25k-LOC Go monorepo whose bulk is the llama.cpp/GGML runtime bridge, model-registry pull/push, and the full HTTP host. `cave-local-llm` is a **sovereign-deployment client + draft daemon** (~5.2k Rust LOC, 139 tests) that covers the in-scope HTTP contract, not the inference runtime itself (that is an injectable `InferenceBackend` trait — concrete llama.cpp/vLLM adapters are an explicit scope-cut delegated to `cave-llm-gateway`/`cave-portal-api`).

## Pre-uplift parity (manifest @ origin/main)

```
mapped=13  partial=2  skipped=12  unmapped=0  total=27
fill_ratio   = 1.0000   (mapped+partial+skipped)/total
honest_ratio = 0.9259   (mapped+skipped)/total = 25/27
```

`honest_ratio` is dragged below 1.0 only by the **2 partials**. The 12 skipped items are honest scope-cuts. To lift honest_ratio we (a) resolve partials with real implementation and (b) reclassify skipped→mapped where we now implement the feature, decomposing the coarse "llama.cpp adapters" bucket into the concrete pure-Rust pieces we add.

## Gap matrix — priority components

| # | Component | Upstream ref | Pre-uplift cave state | Gap | Action |
|---|---|---|---|---|---|
| 1 | **REST `/api/{generate,chat,embeddings,tags}`** | `server/routes.go`, `api/types.go` | `ollama.rs` (generate/chat ±stream, tags), `ollama_extras.rs` (embed/show/ps/copy/delete) | client-complete | ✅ already mapped |
| 2 | **Multimodal image input** | `Message.Images []ImageData`, `GenerateRequest.Images` | absent on `GenerateRequest`/`ChatMessage` | no `images` field | 🟥→🟩 **TDD cycle 1** (skipped→mapped) |
| 3 | **Tool / function calling** | `ChatRequest.Tools`, `Message.ToolCalls`, `Tool/ToolFunction/ToolCall/ToolCallFunction` | absent | no tool types, no `tools`/`tool_calls` | 🟥→🟩 **TDD cycle 2** (skipped→mapped) |
| 4 | **OpenAI `/v1` streaming** | `docs/openai.md` SSE `chat.completion.chunk` | `openai_compat.rs` non-stream only | no `chat_completions_stream` | 🟧→🟩 **TDD cycle 3** (partial→mapped) |
| 5 | **GGUF model reader** | `fs/ggml/gguf.go` (magic `0x46554747`, version, tensor/kv counts, metadata KVs) | none | no GGUF parse | 🟥→🟩 **TDD cycle 4** (new mapped) |
| 6 | **Quantization helpers (Q4/Q5/Q8…)** | `fs/ggml/type.go` `FileType` enum + `ParseFileType`/`String` | none | no quant-type model | 🟥→🟩 **TDD cycle 5** (new mapped) |
| 7 | **Prompt template engine** | `template/` Go-template | `prompt_template.rs` (var/if/range subset) | subset by design | ✅ mapped (subset scope-cut) |
| 8 | **Inference engine** | `llm/` llama.cpp bridge | `backend.rs` `InferenceBackend` trait + Ollama/OpenAI adapters | trait open by design | ✅ mapped (concrete adapters scope-cut → `cave-llm-gateway`) |
| 9 | **KV-cache + streaming response** | runtime cache state machine | NDJSON streaming present (`build_ndjson_stream`); KV-cache eviction is runtime-internal | eviction state machine is runtime-owned | ⬛ skipped (honest — runtime, not client) |
| 10 | **Modelfile parser** | `parser/parser.go` | `manifest.rs` reads cave manifest, not Modelfile | full Modelfile grammar | ⬛ skipped (model-authoring scope-cut — `/api/create` family) |
| 11 | **Daemon graceful shutdown** | n/a (cave-specific) | `daemon.rs` handles SIGTERM + stop-file | lacks SIGINT/full drain | 🟧 **kept partial (honest, not inflated)** |

### Engine decision: in-process Rust, no llama.cpp FFI (deliberate)

Upstream Ollama *is* a llama.cpp host. cave-local-llm is **not** — it is an HTTP client + draft daemon that talks to a running Ollama (or any OpenAI-compat server) over the network, with inference behind an injectable `InferenceBackend` trait. Therefore:

- **No `llama-cpp-rs` FFI binding and no `candle-rs` in-process weights.** Adding either would pull a multi-hundred-MB native build into a client crate and duplicate the runtime that `cave-llm-gateway` owns. The trait stays open for a concrete adapter to live in the gateway crate.
- The **GGUF reader** (cycle 4) and **quantization helpers** (cycle 5) are *pure-Rust metadata* utilities (header/KV parsing, FileType↔name mapping, bits-per-weight/size estimation). They do **not** run inference — they let the registry/daemon introspect a `.gguf` file and reason about quant trade-offs without shelling to `llama-quantize`. This is the honest, dependency-free slice of `fs/ggml`.

This decision is recorded here per the task's "bilinçli decision raporda" requirement.

## TDD plan (test → FAIL → impl → PASS, one commit each; test+impl in one commit forbidden)

1. Multimodal images — `GenerateRequest.images`, `ChatMessage.images` (base64), `ImageData` helper.
2. Tool calling — `Tool`, `ToolFunction`, `ToolCall`, `ToolCallFunction`, `ChatRequest.tools`, `ChatMessage.tool_calls`.
3. OpenAI `/v1` streaming — `OpenAiCompatClient::chat_completions_stream` (SSE `data:` frames + `[DONE]`).
4. GGUF reader — new `gguf.rs`: magic/version/tensor-count/kv-count + typed metadata values.
5. Quantization helpers — new `quant.rs`: `QuantType` enum, `parse`/`to_string`, `bits_per_weight`, `estimate_size`.

## Projected post-uplift parity

```
mapped  = 13 + 1(stream partial→mapped) + 2(multimodal,tools skipped→mapped) + 2(gguf,quant new) = 18
partial = 1   (daemon shutdown — honest)
skipped = 12 - 2(multimodal,tools)                                                                = 10
total   = 27 + 2(gguf,quant decomposed from llama.cpp-adapters bucket)                            = 29
fill_ratio   = (18+1+10)/29 = 29/29 = 1.0000
honest_ratio = (18+10)/29   = 28/29 = 0.9655   ✓ ≥ 0.95
```

Final honest_ratio is recomputed against actual mapped/partial/skipped after the cycles land (Phase 4). **No `docs/parity/parity-index.json` hand-edit** — the post-commit hook regenerates it from `parity.manifest.toml`.

## Phase 4 — final parity (actual)

```
mapped=18  partial=1  skipped=10  unmapped=0  total=29
fill_ratio   = 1.0000   (29/29)
honest_ratio = 0.9655   (28/29)   ✓ ≥ 0.95   (was 0.9259)
```

Self-audit: **9/9 gates PASS** (`cargo test -p cave-local-llm --test parity_self_audit`). Crate tests: **lib 99 → 127 (+28)**, all green, 0 build warnings, new modules clippy-clean. `cave-local-llm` is a leaf crate (no reverse dependents), so the type additions are isolated.

### TDD cycle ledger (test → FAIL → impl → PASS; test+impl never in one commit)

| Cycle | Component | RED (FAIL) | GREEN (PASS) | Δ tests |
|---|---|---|---|---|
| 1 | Multimodal images | `f43458a0` | `e00f7d64` | +5 |
| 2 | Tool / function calling | `61def66d` | `4a0331ee` | +4 |
| 3 | OpenAI `/v1` SSE streaming | `cf021890` | `92c6ae4a` | +4 unit +1 integ |
| 4 | GGUF reader | `39e0f6c7` | `6fda5732` | +7 |
| 5 | Quantization helpers | `c6a6904d` | `473a85e3` | +8 |

Gap analysis doc: `7b57b175`. Each RED commit fails to compile (missing fields/types/fns); each GREEN commit implements and turns the suite green.

### honest_ratio movement

`honest_ratio = (mapped + skipped) / total`. The lift came from **resolving the OpenAI-compat streaming partial** (2 partials → 1). Reclassifying multimodal/tool-calling (skipped → mapped) and adding the GGUF/quant mapped pieces did not change the honest numerator (skipped already counted as honest) but materially increased real coverage and shrank the skip list. The daemon graceful-shutdown partial was **kept honest, not inflated** — SIGTERM + stop-file shutdown already works in-crate; SIGINT/full-drain remains a cave-runtime host concern.

### LOC delta (cave-local-llm src)

| | LOC |
|---|---|
| Pre-uplift src (15 files) | 5 217 |
| New `gguf.rs` | ~290 |
| New `quant.rs` | ~250 |
| Additions to `ollama.rs` / `openai_compat.rs` (types, helpers, streaming) | ~230 |
| **Post-uplift (approx.)** | **~6 000** |

### Remaining work (honest, out of this scope)

- Daemon SIGINT + in-flight drain (resolves the last partial → 1.0).
- Concrete llama.cpp/vLLM inference adapters (still delegated to `cave-llm-gateway`).
- `/api/pull|push|create|blobs` model-authoring flow, KV-cache eviction, JSON-mode validation — all remain honest scope-cuts.
- GGUF tensor-info block (offsets/dims) reading — current reader stops after the metadata block; the tensor directory is parseable next if the registry needs it.
- Version pin bump `v0.3.0 → latest` is deliberately deferred (gate_2 + functional re-audit), tracked separately.
