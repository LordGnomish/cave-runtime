// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! OpenAI-compatible `/v1/embeddings` contract + the embedding service that
//! resolves a model, runs the backend, pools/normalizes, and packages the
//! response in either `float` or `base64` encoding.
//!
//! Upstream: infinity's `infinity_emb/fastapi_schemas/pymodels.py` +
//! `transformer/embedder` dispatch, mirroring the OpenAI embeddings API
//! (`POST /v1/embeddings`).

use crate::backend::{self, BackendRegistry};
use crate::error::{EmbedError, EmbedResult};
use crate::registry::ModelCatalog;
use base64::Engine;
use serde::{Deserialize, Serialize};

/// `input` accepts either a single string or an array of strings.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum EmbeddingInput {
    /// A single document.
    Single(String),
    /// A batch of documents.
    Batch(Vec<String>),
}

impl EmbeddingInput {
    /// Normalize to a vector of inputs.
    pub fn into_vec(self) -> Vec<String> {
        match self {
            EmbeddingInput::Single(s) => vec![s],
            EmbeddingInput::Batch(v) => v,
        }
    }
}

/// Output encoding for the embedding vectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum EncodingFormat {
    /// JSON array of float32 (default).
    #[default]
    Float,
    /// base64-encoded little-endian float32 bytes.
    Base64,
}

/// `POST /v1/embeddings` request body.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EmbeddingRequest {
    /// Text(s) to embed.
    pub input: EmbeddingInput,
    /// Model id.
    pub model: String,
    /// Output encoding.
    #[serde(default)]
    pub encoding_format: EncodingFormat,
    /// Optional Matryoshka output dimensionality.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<usize>,
    /// Optional end-user id (echoed nowhere; OpenAI-compat field).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

/// One embedding, either a float array or a base64 string.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EmbeddingValue {
    /// Float array.
    Float(Vec<f32>),
    /// base64-encoded float32-LE bytes.
    Base64(String),
}

/// One element of the response `data` array.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingData {
    /// Always `"embedding"`.
    pub object: String,
    /// Index in the input batch.
    pub index: usize,
    /// The vector.
    pub embedding: EmbeddingValue,
}

/// Token-usage accounting.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Usage {
    /// Prompt tokens consumed.
    pub prompt_tokens: usize,
    /// Total tokens (== prompt_tokens for embeddings).
    pub total_tokens: usize,
}

/// `POST /v1/embeddings` response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingResponse {
    /// Always `"list"`.
    pub object: String,
    /// One entry per input.
    pub data: Vec<EmbeddingData>,
    /// Model id.
    pub model: String,
    /// Usage accounting.
    pub usage: Usage,
}

/// Encode a float32 slice as OpenAI-style base64 (little-endian bytes).
pub fn encode_f32_base64(v: &[f32]) -> String {
    // PLACEHOLDER (RED).
    let _ = v;
    String::new()
}

/// Heuristic token count (whitespace tokenization). The real server uses the
/// model tokenizer; this is a documented approximation for usage accounting.
pub fn approx_tokens(text: &str) -> usize {
    text.split_whitespace().count().max(1)
}

/// Service that resolves models and produces embedding responses.
pub struct EmbeddingService {
    catalog: ModelCatalog,
    backends: BackendRegistry,
}

impl Default for EmbeddingService {
    fn default() -> Self {
        Self {
            catalog: ModelCatalog::builtin(),
            backends: BackendRegistry::seeded(),
        }
    }
}

impl EmbeddingService {
    /// Build from an explicit catalog + backend registry.
    pub fn new(catalog: ModelCatalog, backends: BackendRegistry) -> Self {
        Self { catalog, backends }
    }

    /// Borrow the catalog (used by `/v1/models`).
    pub fn catalog(&self) -> &ModelCatalog {
        &self.catalog
    }

    /// Run an embedding request.
    pub async fn embed(&self, req: &EmbeddingRequest) -> EmbedResult<EmbeddingResponse> {
        // PLACEHOLDER (RED): empty response.
        let _ = (&self.catalog, &self.backends, req);
        Ok(EmbeddingResponse {
            object: "list".into(),
            data: Vec::new(),
            model: req.model.clone(),
            usage: Usage {
                prompt_tokens: 0,
                total_tokens: 0,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64_decode_len(s: &str) -> usize {
        base64::engine::general_purpose::STANDARD
            .decode(s)
            .unwrap()
            .len()
    }

    #[test]
    fn input_deserializes_single_and_batch() {
        let s: EmbeddingRequest =
            serde_json::from_str(r#"{"input":"hi","model":"m"}"#).unwrap();
        assert_eq!(s.input.into_vec(), vec!["hi".to_string()]);
        let b: EmbeddingRequest =
            serde_json::from_str(r#"{"input":["a","b"],"model":"m"}"#).unwrap();
        assert_eq!(b.input.into_vec().len(), 2);
        assert_eq!(b.encoding_format, EncodingFormat::Float);
    }

    #[test]
    fn base64_roundtrip_is_little_endian_f32() {
        let s = encode_f32_base64(&[1.0, 2.0]);
        let bytes = base64::engine::general_purpose::STANDARD.decode(&s).unwrap();
        assert_eq!(bytes.len(), 8);
        assert_eq!(&bytes[0..4], &1.0f32.to_le_bytes());
        assert_eq!(&bytes[4..8], &2.0f32.to_le_bytes());
    }

    fn req(model: &str, input: EmbeddingInput) -> EmbeddingRequest {
        EmbeddingRequest {
            input,
            model: model.into(),
            encoding_format: EncodingFormat::Float,
            dimensions: None,
            user: None,
        }
    }

    #[tokio::test]
    async fn single_string_yields_minilm_384() {
        let svc = EmbeddingService::default();
        let r = svc
            .embed(&req(
                "sentence-transformers/all-MiniLM-L6-v2",
                EmbeddingInput::Single("hello world".into()),
            ))
            .await
            .unwrap();
        assert_eq!(r.object, "list");
        assert_eq!(r.data.len(), 1);
        assert_eq!(r.data[0].index, 0);
        match &r.data[0].embedding {
            EmbeddingValue::Float(v) => {
                assert_eq!(v.len(), 384);
                let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                assert!((n - 1.0).abs() < 1e-4, "MiniLM normalizes");
            }
            _ => panic!("expected float"),
        }
        assert!(r.usage.prompt_tokens > 0);
        assert_eq!(r.usage.prompt_tokens, r.usage.total_tokens);
    }

    #[tokio::test]
    async fn batch_indices_are_sequential() {
        let svc = EmbeddingService::default();
        let r = svc
            .embed(&req(
                "sentence-transformers/all-MiniLM-L6-v2",
                EmbeddingInput::Batch(vec!["a".into(), "b c".into(), "d".into()]),
            ))
            .await
            .unwrap();
        assert_eq!(r.data.len(), 3);
        assert_eq!(r.data.iter().map(|d| d.index).collect::<Vec<_>>(), vec![0, 1, 2]);
    }

    #[tokio::test]
    async fn base64_format_returns_decodable_bytes() {
        let mut request = req(
            "sentence-transformers/all-MiniLM-L6-v2",
            EmbeddingInput::Single("base64 please".into()),
        );
        request.encoding_format = EncodingFormat::Base64;
        let svc = EmbeddingService::default();
        let r = svc.embed(&request).await.unwrap();
        match &r.data[0].embedding {
            EmbeddingValue::Base64(s) => assert_eq!(b64_decode_len(s), 384 * 4),
            _ => panic!("expected base64"),
        }
    }

    #[tokio::test]
    async fn dimensions_matryoshka_on_nomic() {
        let mut request = req(
            "nomic-ai/nomic-embed-text-v1.5",
            EmbeddingInput::Single("truncate me".into()),
        );
        request.dimensions = Some(256);
        let svc = EmbeddingService::default();
        let r = svc.embed(&request).await.unwrap();
        match &r.data[0].embedding {
            EmbeddingValue::Float(v) => assert_eq!(v.len(), 256),
            _ => panic!(),
        }
    }

    #[tokio::test]
    async fn invalid_dimensions_rejected() {
        let mut request = req(
            "sentence-transformers/all-MiniLM-L6-v2",
            EmbeddingInput::Single("x".into()),
        );
        request.dimensions = Some(256); // MiniLM is not Matryoshka
        let svc = EmbeddingService::default();
        assert!(matches!(
            svc.embed(&request).await,
            Err(EmbedError::InvalidDimensions { .. })
        ));
    }

    #[tokio::test]
    async fn unknown_model_errors() {
        let svc = EmbeddingService::default();
        assert!(matches!(
            svc.embed(&req("nope/x", EmbeddingInput::Single("y".into())))
                .await,
            Err(EmbedError::UnknownModel(_))
        ));
    }

    #[tokio::test]
    async fn empty_input_errors() {
        let svc = EmbeddingService::default();
        assert!(matches!(
            svc.embed(&req(
                "sentence-transformers/all-MiniLM-L6-v2",
                EmbeddingInput::Batch(vec![])
            ))
            .await,
            Err(EmbedError::EmptyInput)
        ));
    }
}
