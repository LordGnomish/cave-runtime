// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Embedding backend trait + deterministic reference embedder.
//!
//! A production deployment loads model weights through a concrete
//! [`EmbeddingBackend`] (ONNX Runtime, candle, or burn). To keep the serving
//! pipeline testable and the crate dependency-free, [`HashEmbedder`] is a real
//! — if simple — bag-of-words embedding: each token is projected into the
//! model's dimensionality via SHA-256 signed feature hashing, the per-token
//! vectors are pooled with the card's strategy, and the result is L2-normalized
//! when the card requests it. It is deterministic and produces higher cosine
//! similarity for inputs that share vocabulary, so cosine ranking is meaningful.

use crate::pooling::{l2_normalize, pool, Pooling};
use crate::registry::ModelCard;
use crate::tokenize::tokenize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Error raised by a backend during embedding.
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    /// The backend could not produce an embedding for the given input.
    #[error("embedding failed: {0}")]
    Failed(String),
}

/// A pluggable embedding model backend. Implementations turn prepared text into
/// one sentence vector per input, applying the card's pooling + normalization.
pub trait EmbeddingBackend: Send + Sync {
    /// Stable backend identifier (for the registry / telemetry).
    fn id(&self) -> &str;

    /// Embed a batch of prepared texts under `card`. Returns one vector of
    /// `card.dimensions` per input, in input order.
    fn embed(&self, texts: &[String], card: &ModelCard) -> Result<Vec<Vec<f32>>, BackendError>;
}

/// Deterministic dependency-free reference embedder (signed feature hashing).
#[derive(Debug, Default, Clone)]
pub struct HashEmbedder {
    id: String,
}

impl HashEmbedder {
    /// Create the reference embedder.
    pub fn new() -> Self {
        HashEmbedder {
            id: "hash-embedder".to_string(),
        }
    }

    /// Project a single token into a `dim`-wide vector. The SHA-256 of the token
    /// seeds a bucket index and a sign for each of the token's contributions —
    /// the classic signed feature-hashing trick. A token contributes to one
    /// bucket per 4 hash bytes so longer dims spread tokens further apart.
    fn token_vector(token: &str, dim: usize) -> Vec<f32> {
        let mut v = vec![0.0f32; dim];
        // Hash the token; fold the digest into (bucket, sign) draws.
        let digest = Sha256::digest(token.as_bytes());
        // Use up to 8 (bucket,sign) projections from the 32-byte digest.
        for chunk in digest.chunks(4) {
            let word = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            let bucket = (word as usize) % dim;
            let sign = if word & 1 == 0 { 1.0 } else { -1.0 };
            v[bucket] += sign;
        }
        v
    }
}

impl EmbeddingBackend for HashEmbedder {
    fn id(&self) -> &str {
        &self.id
    }

    fn embed(&self, texts: &[String], card: &ModelCard) -> Result<Vec<Vec<f32>>, BackendError> {
        let dim = card.dimensions;
        let mut out = Vec::with_capacity(texts.len());
        for text in texts {
            let tokens = tokenize(text);
            if tokens.is_empty() {
                // No vocabulary → zero vector (cosine 0 against everything).
                out.push(vec![0.0f32; dim]);
                continue;
            }
            // Build the per-token matrix once, caching identical tokens.
            let mut cache: HashMap<&str, Vec<f32>> = HashMap::new();
            let matrix: Vec<Vec<f32>> = tokens
                .iter()
                .map(|t| {
                    cache
                        .entry(t.as_str())
                        .or_insert_with(|| Self::token_vector(t, dim))
                        .clone()
                })
                .collect();
            let mask = vec![1u32; matrix.len()];
            let strategy = match card.pooling {
                // CLS/last-token are meaningless for a bag-of-words reference
                // (no positional [CLS]); fall back to mean so ranking stays sane.
                Pooling::Cls | Pooling::LastToken => Pooling::Mean,
                other => other,
            };
            let pooled = pool(strategy, &matrix, &mask)
                .map_err(|e| BackendError::Failed(e.to_string()))?;
            let vec = if card.normalize {
                l2_normalize(&pooled)
            } else {
                pooled
            };
            out.push(vec);
        }
        Ok(out)
    }
}

/// Registry of named backends with a selectable default.
#[derive(Default)]
pub struct BackendRegistry {
    backends: HashMap<String, Box<dyn EmbeddingBackend>>,
    default_id: Option<String>,
}

impl BackendRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        BackendRegistry {
            backends: HashMap::new(),
            default_id: None,
        }
    }

    /// Register a backend under its `id()`.
    pub fn register(&mut self, backend: Box<dyn EmbeddingBackend>) {
        let id = backend.id().to_string();
        if self.default_id.is_none() {
            self.default_id = Some(id.clone());
        }
        self.backends.insert(id, backend);
    }

    /// Select the default backend by id (must already be registered).
    pub fn set_default(&mut self, id: &str) {
        if self.backends.contains_key(id) {
            self.default_id = Some(id.to_string());
        }
    }

    /// Look up a backend by id.
    pub fn get(&self, id: &str) -> Option<&dyn EmbeddingBackend> {
        self.backends.get(id).map(|b| b.as_ref())
    }

    /// The current default backend, if any.
    pub fn default(&self) -> Option<&dyn EmbeddingBackend> {
        self.default_id.as_ref().and_then(|id| self.get(id))
    }
}
