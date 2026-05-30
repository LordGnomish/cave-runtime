// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-embed — high-throughput embedding service.
//!
//! A pure-Rust, dependency-light port of the serving surface of
//! [michaelfeil/infinity](https://github.com/michaelfeil/infinity) (MIT): an
//! OpenAI `/v1/embeddings`-compatible API, a sentence-transformers/BGE/E5/
//! Mistral/nomic/jina model registry, dynamic token-budget batching, the
//! standard pooling strategies, embedding quantization, a result cache, and a
//! cross-encoder reranker (`/v1/rerank`).
//!
//! The crate ships a deterministic, dependency-free reference
//! [`backend::HashEmbedder`] so the full pipeline (tokenize → batch → pool →
//! normalize → quantize → cache → serve) is exercisable without bundling model
//! weights or a native ONNX/candle/burn runtime. Concrete weight-loading
//! backends plug in through the [`backend::EmbeddingBackend`] trait.

/// Token embedding pooling strategies (mean / CLS / max / last-token) + L2 norm.
pub mod pooling;

/// Embedding model registry — per-model cards (dims, ctx, pooling, prefixes).
pub mod registry;

/// Lightweight word tokenizer for token accounting and context truncation.
pub mod tokenize;

/// Dynamic token-budget batching (max batch size + max tokens per batch).
pub mod batch;

/// Pluggable embedding backend trait + deterministic reference HashEmbedder.
pub mod backend;
