<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# cave-embed

OpenAI-compatible **text-embedding + rerank** inference server — a clean-room,
pure-safe-Rust port of [`michaelfeil/infinity`](https://github.com/michaelfeil/infinity)
plus the [`sentence-transformers`](https://github.com/UKPLab/sentence-transformers)
pooling / normalize / quantize patterns. No native ML runtime; a deterministic
feature-hashing reference embedder makes the whole API exercisable offline.

## API

| method | path | purpose |
|--------|------|---------|
| POST | `/v1/embeddings` | OpenAI embeddings (`float`/`base64`, `dimensions`) |
| GET  | `/v1/models` | list registered models |
| POST | `/rerank` | Cohere/infinity rerank (`top_n`, `return_documents`) |
| GET  | `/admin/embed` | server-rendered model catalog page |
| GET  | `/api/embed/models` | model cards (JSON) |
| GET  | `/api/embed/health` | health probe |

## Run

```sh
cargo run -p cave-embed --bin cave-embed -- --addr 127.0.0.1:7997
```

## Features

- 6 model families: sentence-transformers, BGE, E5, nomic (Matryoshka), Jina, Mistral
- Pooling: mean / cls / max / last-token / mean-sqrt-len + L2 normalize
- Quantization: int8 / uint8 / binary / ubinary
- Length-sorted dynamic batching, LRU cache, CLIP modality routing

See [`PARITY_REPORT.md`](./PARITY_REPORT.md) for the parity breakdown.

Licensed under AGPL-3.0-or-later.
