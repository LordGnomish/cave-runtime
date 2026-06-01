// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-embed — OpenAI-compatible text-embedding + rerank inference server.
//!
//! Clean-room control-plane port of `michaelfeil/infinity` (MIT) plus the
//! `sentence-transformers` pooling / normalization / quantization patterns,
//! re-implemented in pure safe Rust with no native ML runtime dependency.
//!
//! Scope (mapped):
//! - `openai`   — OpenAI `/v1/embeddings` request/response contract, the
//!   `float`/`base64` encoding formats, and the `dimensions` (Matryoshka)
//!   truncation parameter.
//! - `registry` — embedding-model catalog covering the sentence-transformers,
//!   BGE, E5, nomic, Jina and Mistral families with their native dimensions,
//!   default pooling and asymmetric query/passage instruction prefixes.
//! - `pooling`  — sentence-transformers `Pooling` module (mean / cls / max /
//!   last-token / mean-sqrt-len) + L2 normalization.
//! - `backend`  — `EmbeddingBackend` trait + a deterministic pure-Rust
//!   feature-hashing reference embedder + a backend registry. Concrete
//!   ONNX/torch/CTranslate2 runtimes are a documented scope-cut.
//! - `batch`    — infinity-style dynamic batcher (length-sorted micro-batches,
//!   max-batch and token-budget caps, padding accounting).
//! - `quantize` — sentence-transformers `quantize_embeddings`
//!   (int8 / uint8 / binary / ubinary scalar quantization).
//! - `cache`    — LRU embedding cache keyed by (model, content hash).
//! - `rerank`   — Cohere/infinity `/rerank` cross-encoder contract
//!   (top_n, return_documents, score + sort).
//! - `clip`     — multimodal (text/image) modality routing.

/// Shared error type.
pub mod error;

pub mod pooling;

pub mod registry;

pub mod backend;

pub mod openai;

pub mod batch;

pub mod quantize;

pub use error::{EmbedError, EmbedResult};

/// Crate identity. The self-audit (`gate_*`) asserts this stays in lockstep
/// with `parity.manifest.toml`'s `[upstream] version`.
pub const UPSTREAM_VERSION: &str = "0.0.75";
