// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Embedding service — orchestrates registry + backend + tokenizer.
//!
//! [`EmbeddingService::embed`] is the serving core behind `/v1/embeddings`:
//! resolve the model card, truncate each input to the context window, embed the
//! batch through the backend, optionally truncate the output dimensionality
//! (Matryoshka) and renormalize, encode `float`/`base64`, and report token
//! usage — the pipeline infinity runs per request.

use crate::api::{
    EmbeddingData, EmbeddingObject, EmbeddingRequest, EmbeddingResponse, Usage,
};
use crate::backend::{EmbeddingBackend, HashEmbedder};
use crate::pooling::l2_normalize;
use crate::registry::ModelRegistry;
use crate::tokenize::{count_tokens, truncate};
use base64::Engine;

/// Errors surfaced by the embedding service.
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    /// The requested model is not registered.
    #[error("unknown model: {0}")]
    UnknownModel(String),
    /// The input list was empty.
    #[error("input must not be empty")]
    EmptyInput,
    /// Requested `dimensions` exceeds the model's native dimensionality.
    #[error("requested {requested} dimensions but model has {available}")]
    InvalidDimensions {
        /// Dimensions the caller asked for.
        requested: usize,
        /// Dimensions the model actually produces.
        available: usize,
    },
    /// The backend failed.
    #[error(transparent)]
    Backend(#[from] crate::backend::BackendError),
}

/// The serving entry point: a model registry plus a default backend.
pub struct EmbeddingService {
    registry: ModelRegistry,
    backend: Box<dyn EmbeddingBackend>,
}

impl EmbeddingService {
    /// Build a service with the given registry and backend.
    pub fn new(registry: ModelRegistry, backend: Box<dyn EmbeddingBackend>) -> Self {
        EmbeddingService { registry, backend }
    }

    /// Build a service with the built-in model catalogue and reference backend.
    pub fn with_builtins() -> Self {
        Self::new(
            ModelRegistry::with_builtins(),
            Box::new(HashEmbedder::new()),
        )
    }

    /// Borrow the model registry (for `/v1/models`).
    pub fn registry(&self) -> &ModelRegistry {
        &self.registry
    }

    /// Serve an `/v1/embeddings` request.
    pub fn embed(&self, req: &EmbeddingRequest) -> Result<EmbeddingResponse, ServiceError> {
        let card = self
            .registry
            .get(&req.model)
            .ok_or_else(|| ServiceError::UnknownModel(req.model.clone()))?;

        let inputs = req.input.as_vec();
        if inputs.is_empty() {
            return Err(ServiceError::EmptyInput);
        }

        if let Some(d) = req.dimensions {
            if d == 0 || d > card.dimensions {
                return Err(ServiceError::InvalidDimensions {
                    requested: d,
                    available: card.dimensions,
                });
            }
        }

        // Truncate each input to the context window and tally tokens.
        let mut prepared = Vec::with_capacity(inputs.len());
        let mut total_tokens = 0usize;
        for text in &inputs {
            let t = truncate(text, card.max_seq_len);
            total_tokens += count_tokens(&t);
            prepared.push(t);
        }

        let raw = self.backend.embed(&prepared, card)?;

        let base64 = req.encoding_format.as_deref() == Some("base64");
        let mut data = Vec::with_capacity(raw.len());
        for (index, mut vec) in raw.into_iter().enumerate() {
            // Matryoshka dimension truncation + renormalize.
            if let Some(d) = req.dimensions {
                vec.truncate(d);
                if card.normalize {
                    vec = l2_normalize(&vec);
                }
            }
            let embedding = if base64 {
                EmbeddingData::Base64(encode_f32_base64(&vec))
            } else {
                EmbeddingData::Float(vec)
            };
            data.push(EmbeddingObject {
                object: "embedding".to_string(),
                index,
                embedding,
            });
        }

        Ok(EmbeddingResponse {
            object: "list".to_string(),
            data,
            model: card.id.clone(),
            usage: Usage {
                prompt_tokens: total_tokens,
                total_tokens,
            },
        })
    }
}

/// Encode an f32 slice as base64 of its little-endian byte representation —
/// the layout the OpenAI client decodes when `encoding_format=base64`.
fn encode_f32_base64(v: &[f32]) -> String {
    let mut bytes = Vec::with_capacity(v.len() * 4);
    for f in v {
        bytes.extend_from_slice(&f.to_le_bytes());
    }
    base64::engine::general_purpose::STANDARD.encode(&bytes)
}
