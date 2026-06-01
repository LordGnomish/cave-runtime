<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# cave-embed — Parity Report (Charter v2)

**Upstream:** [`michaelfeil/infinity`](https://github.com/michaelfeil/infinity) `0.0.75` (MIT)
**Patterns:** [`UKPLab/sentence-transformers`](https://github.com/UKPLab/sentence-transformers) — pooling / normalize / quantize
**Audited:** 2026-06-01 · **Self-audit:** 10/10 PASS

## Summary

cave-embed is a clean-room, pure-safe-Rust re-implementation of infinity's
embedding/rerank inference-server **control plane** — no native ML runtime.
A deterministic FNV-1a/SplitMix64 feature-hashing reference embedder stands in
for model weights, so the entire OpenAI-compatible surface is exercisable
offline and in CI. Concrete neural runtimes are ADR-justified scope-cuts; the
`EmbeddingBackend` / `Reranker` traits stay open for a future ONNX/candle adapter.

| metric | value |
|--------|-------|
| mapped | 12 |
| partial | 2 |
| skipped | 5 |
| unmapped | 0 |
| total | 19 |
| fill_ratio | 1.0000 |
| honest_ratio | 0.8947 |

`honest_ratio = (mapped + skipped) / total = 17/19`.

## Mapped (12)

1. **OpenAI `/v1/embeddings`** — string|array input, `float`|`base64` (LE-f32)
   encoding, `dimensions` (Matryoshka) param, usage accounting.
2. **`/v1/models`** — OpenAI model-list shape.
3. **Model catalog** — 6 families (sentence-transformers, BGE, E5, nomic, Jina,
   Mistral) with native dims, default pooling, normalize flag, and asymmetric
   query/passage instruction prefixes.
4. **Pooling** — mean / cls / max / last-token / mean-sqrt-len.
5. **L2 normalization**.
6. **`EmbeddingBackend` trait** + reference embedder + backend registry.
7. **Matryoshka truncation** + renormalize.
8. **Dynamic batcher** — length-sorted micro-batches, max-batch + padded-token
   budget, padding-waste accounting.
9. **Quantization** — int8/uint8 scalar (calibration ranges) + binary/ubinary
   big-endian packbits.
10. **LRU cache** — keyed by `sha256(model, text)`, hit/miss stats.
11. **Rerank `/rerank`** — `Reranker` trait, score+sort+`top_n`+`return_documents`.
12. **CLIP** — multimodal text/image modality routing into a shared space.

## Partial (2)

- **Tokenizer** — whitespace heuristic for usage/length; real HF BPE/WordPiece
  + exact `max_seq_len` truncation not implemented.
- **CLIP image branch** — routing + shared-space contract mapped; real image
  decode/preprocess + vision tower deferred (byte-hash stand-in).

## Skipped (5) — ADR-justified

| subsystem | label |
|-----------|-------|
| ONNX (optimum/onnxruntime) backend | vendor-adapter |
| torch / sentence-transformers / CTranslate2 backends | vendor-adapter |
| GPU/device + fp16/bettertransformer acceleration | out-of-scope (host) |
| Prometheus `/metrics` hosting | parallel-track (cave-metrics) |
| `/classify` sequence-classification head | out-of-scope-subsystem |

## Gate results (10/10)

1. SPDX coverage 100% · 2. source_sha pinned · 3. last_audit 2026 ·
4. parity_ratio_source=manifest · 5. fill_ratio≥0.95 · 6. count invariants ·
7. no stub macros · 8. this report exists · 9. `UPSTREAM_VERSION` matches
manifest · 10. Charter v2 composite.
