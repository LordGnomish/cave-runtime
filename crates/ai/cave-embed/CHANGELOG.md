# Changelog — cave-embed

## Unreleased — 2026-05-30 (fresh strict-TDD port)

Fresh pure-Rust port of the serving surface of michaelfeil/infinity (MIT),
built with 8 RED→GREEN cycles:

- `pooling` — mask-aware mean/max, CLS, last-token, L2, cosine
- `registry` — 11-card catalogue (ST/BGE/E5/Mistral/nomic/jina/CLIP) + prefixes
- `tokenize` + `batch` — word tokenizer + dynamic token-budget batcher
- `backend` — `EmbeddingBackend` trait + deterministic reference `HashEmbedder`
- `api` + `service` — OpenAI `/v1/embeddings` (float/base64, Matryoshka dims, usage)
- `cache` — LRU result memoization
- `quant` — fp16 + per-vector int8
- `rerank` — cross-encoder + `/v1/rerank`

Charter v2 8-gate self-audit: 8/8 PASS. honest_ratio 0.9412 (multimodal CLIP
held as an honest partial). 64 tests green.
