// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Inference backend abstraction.
//!
//! infinity dispatches to a concrete engine (sentence-transformers/torch,
//! optimum/ONNX, CTranslate2) that turns text into per-token hidden states.
//! We expose that as the [`EmbeddingBackend`] trait and ship a deterministic,
//! dependency-free **reference embedder** (feature-hashing) so the server is
//! fully exercisable in tests and offline demos. Concrete neural runtimes are
//! a documented scope-cut (delegated to a future ONNX/candle adapter).

use crate::error::{EmbedError, EmbedResult};
use crate::pooling::{self, PoolingStrategy};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

/// A backend turns a single text into per-token hidden states + attention mask.
#[async_trait]
pub trait EmbeddingBackend: Send + Sync {
    /// Backend / model id.
    fn id(&self) -> &str;
    /// Native hidden dimensionality.
    fn dims(&self) -> usize;
    /// Encode one text into `(token_embeddings[seq][hidden], mask[seq])`.
    async fn encode_tokens(&self, text: &str) -> EmbedResult<(Vec<Vec<f32>>, Vec<u32>)>;
}

/// Deterministic, dependency-free reference embedder.
///
/// Each whitespace token is hashed to seed a small SplitMix64 PRNG that emits
/// a `dims`-length vector in `[-1, 1]`. The mapping is stable across runs and
/// process restarts, and is permutation-invariant under mean pooling — making
/// it a faithful stand-in for property tests without shipping model weights.
pub struct HashingEmbedder {
    id: String,
    dims: usize,
}

impl HashingEmbedder {
    /// Create a reference embedder with the given id and output dimensionality.
    pub fn new(id: impl Into<String>, dims: usize) -> Self {
        Self {
            id: id.into(),
            dims,
        }
    }

    /// Stable FNV-1a 64-bit hash of a token.
    fn fnv1a(token: &str) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in token.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }

    /// One SplitMix64 step.
    fn splitmix(state: &mut u64) -> u64 {
        *state = state.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }
}

#[async_trait]
impl EmbeddingBackend for HashingEmbedder {
    fn id(&self) -> &str {
        &self.id
    }
    fn dims(&self) -> usize {
        self.dims
    }
    async fn encode_tokens(&self, text: &str) -> EmbedResult<(Vec<Vec<f32>>, Vec<u32>)> {
        let words: Vec<&str> = text.split_whitespace().collect();
        if words.is_empty() {
            return Err(EmbedError::EmptyInput);
        }
        let mut tokens = Vec::with_capacity(words.len());
        for w in &words {
            let mut state = Self::fnv1a(&w.to_ascii_lowercase());
            let mut row = Vec::with_capacity(self.dims);
            for _ in 0..self.dims {
                // Map a u64 into a deterministic f32 in [-1, 1).
                let r = Self::splitmix(&mut state);
                let unit = (r >> 11) as f32 / (1u64 << 53) as f32; // [0,1)
                row.push(unit * 2.0 - 1.0);
            }
            tokens.push(row);
        }
        let mask = vec![1u32; tokens.len()];
        Ok((tokens, mask))
    }
}

/// Registry of named backends.
#[derive(Default, Clone)]
pub struct BackendRegistry {
    backends: HashMap<String, Arc<dyn EmbeddingBackend>>,
}

impl BackendRegistry {
    /// Empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a backend under its id.
    pub fn register(&mut self, backend: Arc<dyn EmbeddingBackend>) {
        self.backends.insert(backend.id().to_string(), backend);
    }

    /// Look up a backend by id.
    pub fn get(&self, id: &str) -> Option<Arc<dyn EmbeddingBackend>> {
        self.backends.get(id).cloned()
    }

    /// Number of registered backends.
    pub fn len(&self) -> usize {
        self.backends.len()
    }

    /// Whether empty.
    pub fn is_empty(&self) -> bool {
        self.backends.is_empty()
    }

    /// Build a registry pre-populated with a reference [`HashingEmbedder`] for
    /// every model in the built-in catalog (matching its native dims).
    pub fn seeded() -> Self {
        let mut reg = Self::new();
        let catalog = crate::registry::ModelCatalog::builtin();
        for id in catalog.ids() {
            let dims = catalog.get(id).map(|c| c.dims).unwrap_or(0);
            reg.register(Arc::new(HashingEmbedder::new(id, dims)));
        }
        reg
    }
}

/// Core embed path: encode → pool → optional Matryoshka truncation → optional
/// L2 normalize. This is what the OpenAI `/v1/embeddings` route calls.
pub async fn embed_with(
    backend: &dyn EmbeddingBackend,
    pooling_strategy: PoolingStrategy,
    normalize: bool,
    truncate_dims: Option<usize>,
    text: &str,
) -> EmbedResult<Vec<f32>> {
    let (tokens, mask) = backend.encode_tokens(text).await?;
    let mut v = pooling::pool(pooling_strategy, &tokens, &mask)?;
    if let Some(d) = truncate_dims {
        if d == 0 || d > v.len() {
            return Err(EmbedError::InvalidDimensions {
                requested: d,
                native: v.len(),
            });
        }
        v.truncate(d);
    }
    if normalize {
        v = pooling::l2_normalize(&v)?;
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        dot / (na * nb)
    }

    #[tokio::test]
    async fn encode_emits_one_token_per_word_with_right_dims() {
        let e = HashingEmbedder::new("ref", 16);
        let (tok, mask) = e.encode_tokens("hello world foo").await.unwrap();
        assert_eq!(tok.len(), 3, "one row per whitespace token");
        assert_eq!(mask.len(), 3);
        assert!(tok.iter().all(|r| r.len() == 16));
        assert!(mask.iter().all(|&m| m == 1));
    }

    #[tokio::test]
    async fn deterministic_across_calls() {
        let e = HashingEmbedder::new("ref", 32);
        let a = embed_with(&e, PoolingStrategy::Mean, true, None, "the cat sat").await.unwrap();
        let b = embed_with(&e, PoolingStrategy::Mean, true, None, "the cat sat").await.unwrap();
        assert_eq!(a, b);
    }

    #[tokio::test]
    async fn mean_pooling_is_permutation_invariant() {
        let e = HashingEmbedder::new("ref", 32);
        let a = embed_with(&e, PoolingStrategy::Mean, false, None, "cat dog bird").await.unwrap();
        let b = embed_with(&e, PoolingStrategy::Mean, false, None, "bird cat dog").await.unwrap();
        for (x, y) in a.iter().zip(&b) {
            assert!((x - y).abs() < 1e-5);
        }
    }

    #[tokio::test]
    async fn different_text_differs() {
        let e = HashingEmbedder::new("ref", 32);
        let a = embed_with(&e, PoolingStrategy::Mean, true, None, "alpha beta").await.unwrap();
        let b = embed_with(&e, PoolingStrategy::Mean, true, None, "gamma delta").await.unwrap();
        assert!(cosine(&a, &b) < 0.999);
    }

    #[tokio::test]
    async fn normalized_is_unit_length() {
        let e = HashingEmbedder::new("ref", 64);
        let v = embed_with(&e, PoolingStrategy::Mean, true, None, "normalize me").await.unwrap();
        let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((n - 1.0).abs() < 1e-5);
    }

    #[tokio::test]
    async fn matryoshka_truncation_then_renormalize() {
        let e = HashingEmbedder::new("ref", 64);
        let v = embed_with(&e, PoolingStrategy::Mean, true, Some(16), "truncate me").await.unwrap();
        assert_eq!(v.len(), 16);
        let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((n - 1.0).abs() < 1e-5);
    }

    #[tokio::test]
    async fn truncate_beyond_native_errors() {
        let e = HashingEmbedder::new("ref", 8);
        let r = embed_with(&e, PoolingStrategy::Mean, false, Some(99), "x").await;
        assert!(matches!(r, Err(EmbedError::InvalidDimensions { .. })));
    }

    #[test]
    fn seeded_registry_has_all_catalog_models() {
        let reg = BackendRegistry::seeded();
        assert_eq!(reg.len(), crate::registry::ModelCatalog::builtin().len());
        let b = reg.get("sentence-transformers/all-MiniLM-L6-v2").unwrap();
        assert_eq!(b.dims(), 384);
    }
}
