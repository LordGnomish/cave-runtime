// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! OpenAI `/v1/embeddings` request/response types.
//!
//! Wire-compatible with the OpenAI embeddings contract infinity serves: a
//! `model` + `input` (single string or array), an optional `encoding_format`
//! (`float` or `base64`), an optional `dimensions` (Matryoshka truncation), and
//! an optional `user`. Responses carry one embedding object per input with a
//! `usage` token tally.

use serde::{Deserialize, Serialize};

/// `input` accepts either a single string or an array of strings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Input {
    /// A single text input.
    Single(String),
    /// A batch of text inputs.
    Batch(Vec<String>),
}

impl Input {
    /// Normalize to a vector of inputs.
    pub fn into_vec(self) -> Vec<String> {
        match self {
            Input::Single(s) => vec![s],
            Input::Batch(v) => v,
        }
    }

    /// Borrow as a slice-producing vector without consuming.
    pub fn as_vec(&self) -> Vec<String> {
        match self {
            Input::Single(s) => vec![s.clone()],
            Input::Batch(v) => v.clone(),
        }
    }
}

/// An `/v1/embeddings` request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingRequest {
    /// Model id or alias.
    pub model: String,
    /// One or many input texts.
    pub input: Input,
    /// `float` (default) or `base64`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding_format: Option<String>,
    /// Truncate output to this many dimensions (Matryoshka), then renormalize.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<usize>,
    /// Opaque end-user identifier (passed through, unused).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

/// One embedding payload — float array or base64-encoded little-endian f32.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EmbeddingData {
    /// Raw float vector.
    Float(Vec<f32>),
    /// Base64-encoded little-endian f32 array.
    Base64(String),
}

/// One element of the response `data` list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingObject {
    /// Always `"embedding"`.
    pub object: String,
    /// Position in the input batch.
    pub index: usize,
    /// The embedding payload.
    pub embedding: EmbeddingData,
}

/// Token usage tally.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Usage {
    /// Tokens consumed by the prompt inputs.
    pub prompt_tokens: usize,
    /// Total tokens (equal to `prompt_tokens` for embeddings).
    pub total_tokens: usize,
}

/// An `/v1/embeddings` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingResponse {
    /// Always `"list"`.
    pub object: String,
    /// One object per input.
    pub data: Vec<EmbeddingObject>,
    /// Canonical model id that served the request.
    pub model: String,
    /// Token usage.
    pub usage: Usage,
}
