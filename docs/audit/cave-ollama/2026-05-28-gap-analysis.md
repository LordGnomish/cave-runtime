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
