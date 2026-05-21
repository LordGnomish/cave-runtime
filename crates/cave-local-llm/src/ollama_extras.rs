//! Ollama extras — `/api/show`, `/api/copy`, `/api/delete`, `/api/embed`, `/api/ps`.
//!
//! These endpoints extend the core API (tags/generate/chat) with model lifecycle
//! management, embedding generation, and running-model introspection.
//!
//! API reference: <https://github.com/ollama/ollama/blob/main/docs/api.md>

use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::instrument;

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
        Self {
            base_url: base_url.into(),
            client,
        }
    }

    /// Show model information by model name (POST /api/show).
    ///
    /// Accepts a JSON body with `model` (required) and optional `stream` / `options` fields.
    #[instrument(skip(self, req))]
    pub async fn show(&self, req: ShowRequest) -> OllamaExtrasResult<ShowResponse> {
        let response = self
            .client
            .post(format!("{}/api/show", self.base_url))
            .json(&serde_json::json!({ "model": req.model }))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(OllamaExtrasError::Api { status, body });
        }

        Ok(response.json().await?)
    }

    /// Copy a model from one name to another (POST /api/copy).
    #[instrument(skip(self, req))]
    pub async fn copy(&self, req: CopyRequest) -> OllamaExtrasResult<CopyResponse> {
        let response = self
            .client
            .post(format!("{}/api/copy", self.base_url))
            .json(&serde_json::json!({
                "source": req.source,
                "destination": req.destination,
            }))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(OllamaExtrasError::Api { status, body });
        }

        Ok(response.json().await?)
    }

    /// Delete a model (POST /api/delete).
    #[instrument(skip(self, req))]
    pub async fn delete(&self, req: DeleteRequest) -> OllamaExtrasResult<DeleteResponse> {
        let response = self
            .client
            .delete(format!("{}/api/delete", self.base_url))
            .json(&serde_json::json!({ "model": req.model }))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(OllamaExtrasError::Api { status, body });
        }

        Ok(response.json().await?)
    }

    /// Generate embeddings for input texts (POST /api/embed).
    #[instrument(skip(self, req))]
    pub async fn embed(&self, req: EmbedRequest) -> OllamaExtrasResult<EmbedResponse> {
        let body = serde_json::json!({
            "model": req.model,
            "input": req.input,
        });

        // Only include truncate if Some
        let body = if req.truncate.is_some() {
            serde_json::json!({
                "model": body["model"],
                "input": body["input"],
                "truncate": req.truncate,
            })
        } else {
            body
        };

        let response = self
            .client
            .post(format!("{}/api/embed", self.base_url))
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(OllamaExtrasError::Api { status, body });
        }

        Ok(response.json().await?)
    }

    /// List running models (GET /api/ps).
    #[instrument(skip(self))]
    pub async fn list_running(&self) -> OllamaExtrasResult<PsResponse> {
        let response = self
             .client
             .get(format!("{}/api/ps", self.base_url))
             .send()
             .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(OllamaExtrasError::Api { status, body });
         }

        Ok(response.json().await?)
     }
}

// ── Unit tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

     #[test]
    fn test_copy_request_serialises() {
        let req = CopyRequest {
            source: "qwen2.5-coder:32b".into(),
            destination: "qwen2.5-coder:small".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"source\""));
        assert!(json.contains("\"destination\""));
        assert!(json.contains("qwen2.5-coder:32b"));
        assert!(json.contains("qwen2.5-coder:small"));
     }

     #[test]
    fn test_delete_request_serialises() {
        let req = DeleteRequest {
            model: "qwen2.5-coder:32b".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"model\""));
        assert!(json.contains("qwen2.5-coder:32b"));
     }

     #[test]
    fn test_embed_request_includes_truncate() {
        let req = EmbedRequest {
            model: "nomic-embed".into(),
            input: vec!["hello".into()],
            truncate: Some(true),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"truncate\":true"));
     }

     #[test]
    fn test_embed_request_skips_truncate_when_none() {
        let req = EmbedRequest {
            model: "nomic-embed".into(),
            input: vec!["hello".into()],
            truncate: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("truncate"));
     }

     #[test]
    fn test_show_response_deserialises_full() {
        let raw = r#"{
            "parent_model": "",
            "format": "f16",
            "family": "qwen2.5-coder",
            "families": ["qwen2.5-coder"],
            "parameter_size": "3.8B",
            "license": "Apache-2.0",
            "modified_at": "2024-01-01T00:00:00Z",
            "details": {
                "parent_model": "",
                "format": "f16",
                "family": "qwen2.5-coder",
                "families": ["qwen2.5-coder"],
                "parameter_size": "3.8B",
                "quantization_level": "Q4_K_M"
            }
        }"#;
        let resp: ShowResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(resp.format, Some("f16".into()));
        assert_eq!(resp.family, Some("qwen2.5-coder".into()));
        assert_eq!(resp.parameter_size, Some("3.8B".into()));
        assert_eq!(resp.license, Some("Apache-2.0".into()));
        assert_eq!(resp.details.unwrap().quantization_level, Some("Q4_K_M".into()));
     }

     #[test]
    fn test_show_response_deserialises_minimal() {
        let raw = r#"{"modified_at": "2024-01-01T00:00:00Z"}"#;
        let resp: ShowResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(resp.modified_at, Some("2024-01-01T00:00:00Z".into()));
        assert!(resp.format.is_none());
        assert!(resp.family.is_none());
     }

     #[test]
    fn test_copy_response_deserialises() {
        let raw = r#"{"status": "success"}"#;
        let resp: CopyResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(resp.status, "success");
     }

     #[test]
    fn test_delete_response_deserialises() {
        let raw = r#"{"status": "success"}"#;
        let resp: DeleteResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(resp.status, "success");
     }

     #[test]
    fn test_embed_response_deserialises() {
        let raw = r#"{
            "model": "nomic-embed-text",
            "embeddings": [[0.1, 0.2], [0.3, 0.4]],
            "total_duration": 1234567
        }"#;
        let resp: EmbedResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(resp.model, Some("nomic-embed-text".into()));
        assert_eq!(resp.embeddings.len(), 2);
        assert_eq!(resp.embeddings[0], vec![0.1, 0.2]);
        assert_eq!(resp.total_duration, Some(1_234_567));
     }

     #[test]
    fn test_embed_response_with_default_model() {
        let raw = r#"{
            "embeddings": [[0.5]],
            "total_duration": null
        }"#;
        let resp: EmbedResponse = serde_json::from_str(raw).unwrap();
        assert!(resp.model.is_none());
        assert_eq!(resp.total_duration, None);
     }

     #[test]
    fn test_ps_response_deserialises_running_models() {
        let raw = r#"{
            "models": [
                {
                    "name": "qwen2.5-coder:32b",
                    "model": "qwen2.5-coder:32b",
                    "size": 20000000000,
                    "digest": "abc123",
                    "expires_at": "2024-01-02T00:00:00Z",
                    "ttl": 86400000000
                },
                {
                    "name": "llama3:8b"
                }
            ]
        }"#;
        let resp: PsResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(resp.models.len(), 2);
        assert_eq!(resp.models[0].name.as_ref().unwrap(), "qwen2.5-coder:32b");
        assert_eq!(resp.models[0].size, Some(20_000_000_000));
        assert_eq!(resp.models[0].ttl, Some(86_400_000_000i64));
        assert_eq!(resp.models[1].name.as_ref().unwrap(), "llama3:8b");
        assert!(resp.models[1].size.is_none());
     }

     #[test]
    fn test_ps_response_empty() {
        let raw = r#"{ "models": [] }"#;
        let resp: PsResponse = serde_json::from_str(raw).unwrap();
        assert!(resp.models.is_empty());
     }

     #[test]
    fn test_running_model_info_fields() {
        let raw = r#"{
            "name": "test:model",
            "model": "test:model",
            "size": 100,
            "digest": "def456",
            "expires_at": "2024-06-01T00:00:00Z",
            "ttl": 7200
        }"#;
        let info: RunningModelInfo = serde_json::from_str(raw).unwrap();
        assert_eq!(info.name, Some("test:model".into()));
        assert_eq!(info.model, Some("test:model".into()));
        assert_eq!(info.size, Some(100));
        assert_eq!(info.digest, Some("def456".into()));
        assert_eq!(info.expires_at, Some("2024-06-01T00:00:00Z".into()));
        assert_eq!(info.ttl, Some(7200i64));
     }
}
