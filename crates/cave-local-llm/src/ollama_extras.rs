//! Ollama extras — `/api/show`, `/api/copy`, `/api/delete`, `/api/embed`, `/api/ps`.
//!
//! These endpoints extend the core API (tags/generate/chat) with model lifecycle
//! management, embedding generation, and running-model introspection.

use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::instrument;

/// Lightweight errors from extras endpoints.
#[derive(Debug, Error)]
pub enum OllamaExtrasError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Ollama API error {status}: {body}")]
    Api { status: u16, body: String },
}

pub type OllamaExtrasResult<T> = Result<T, OllamaExtrasError>;

// ── Request / response types ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ShowRequest {
    pub model: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ShowDetails {
    #[serde(default)]
    pub parent_model: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub families: Option<Vec<String>>,
    #[serde(default)]
    pub parameter_size: Option<String>,
    #[serde(default)]
    pub quantization_level: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ShowResponse {
    #[serde(default)]
    pub parent_model: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub families: Option<Vec<String>>,
    #[serde(default)]
    pub parameter_size: Option<String>,
    #[serde(default)]
    pub quantization_level: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub modified_at: Option<String>,
    #[serde(default)]
    pub details: Option<ShowDetails>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CopyRequest {
    pub source: String,
    pub destination: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CopyResponse {
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeleteRequest {
    pub model: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeleteResponse {
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EmbedRequest {
    pub model: String,
    pub input: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncate: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EmbedResponse {
    #[serde(default)]
    pub model: Option<String>,
    pub embeddings: Vec<Vec<f32>>,
    #[serde(default)]
    pub total_duration: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RunningModelInfo {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub digest: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub ttl: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PsResponse {
    pub models: Vec<RunningModelInfo>,
}

// ── Client ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct OllamaExtrasClient {
    base_url: String,
    client: Client,
}

impl OllamaExtrasClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .connect_timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("reqwest client build");
        Self { base_url: base_url.into(), client }
    }

    /// Show model details (POST /api/show).
    #[instrument(skip(self))]
    pub async fn show(&self, req: ShowRequest) -> OllamaExtrasResult<ShowResponse> {
        // Placeholder — stubs for RED → GREEN TDD transition.
        Err(OllamaExtrasError::Api {
            status: 501,
            body: "not yet implemented".into(),
        })
    }

    /// Copy a model (POST /api/copy).
    #[instrument(skip(self))]
    pub async fn copy(&self, req: CopyRequest) -> OllamaExtrasResult<CopyResponse> {
        // Placeholder — stubs for RED → GREEN TDD transition.
        Err(OllamaExtrasError::Api {
            status: 501,
            body: "not yet implemented".into(),
        })
    }

    /// Delete a model (POST /api/delete).
    #[instrument(skip(self))]
    pub async fn delete(&self, req: DeleteRequest) -> OllamaExtrasResult<DeleteResponse> {
        // Placeholder — stubs for RED → GREEN TDD transition.
        Err(OllamaExtrasError::Api {
            status: 501,
            body: "not yet implemented".into(),
        })
    }

    /// Generate embeddings (POST /api/embed).
    #[instrument(skip(self))]
    pub async fn embed(&self, req: EmbedRequest) -> OllamaExtrasResult<EmbedResponse> {
        // Placeholder — stubs for RED → GREEN TDD transition.
        Err(OllamaExtrasError::Api {
            status: 501,
            body: "not yet implemented".into(),
        })
    }

    /// List running models (GET /api/ps).
    #[instrument(skip(self))]
    pub async fn list_running(&self) -> OllamaExtrasResult<PsResponse> {
        // Placeholder — stubs for RED → GREEN TDD transition.
        Err(OllamaExtrasError::Api {
            status: 501,
            body: "not yet implemented".into(),
        })
    }
}
