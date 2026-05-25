// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Ollama API methods beyond the four-method core (tags + generate + chat).
//!
//! Cite: ollama/ollama `api/types.go` v0.3.0 — `ShowRequest` / `ShowResponse`
//! (`/api/show`), `PullRequest` (`/api/pull`), `CopyRequest` (`/api/copy`),
//! `DeleteRequest` (`/api/delete`), `EmbedRequest` (`/api/embed`),
//! `ListRunningResponse` (`/api/ps`).
//!
//! These extend `OllamaClient` without depending on the daemon — useful for
//! model lifecycle workflows and embedding-driven recall.

use crate::ollama::{OllamaClient, OllamaError, OllamaResult};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::instrument;

/// Cite: `api/types.go::ShowRequest` (`POST /api/show`).
#[derive(Debug, Clone, Serialize)]
pub struct ShowRequest {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verbose: Option<bool>,
}

/// Cite: `api/types.go::ShowResponse` — narrowed to the fields the daemon
/// uses (parameter count, model family, template).
#[derive(Debug, Clone, Deserialize)]
pub struct ShowResponse {
    #[serde(default)]
    pub modelfile: String,
    #[serde(default)]
    pub parameters: String,
    #[serde(default)]
    pub template: String,
    #[serde(default)]
    pub details: ModelDetails,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ModelDetails {
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub family: String,
    #[serde(default)]
    pub parameter_size: String,
    #[serde(default)]
    pub quantization_level: String,
}

/// Cite: `api/types.go::PullRequest` (`POST /api/pull`).
#[derive(Debug, Clone, Serialize)]
pub struct PullRequest {
    pub model: String,
    /// When true, the request blocks until the model is fully pulled.
    /// When false (default), the response is the first status frame and the
    /// caller is expected to poll `/api/ps`. cave defaults to true so a
    /// pull is atomic from the caller's perspective.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insecure: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PullStatus {
    pub status: String,
    #[serde(default)]
    pub digest: Option<String>,
    #[serde(default)]
    pub total: Option<u64>,
    #[serde(default)]
    pub completed: Option<u64>,
}

/// Cite: `api/types.go::CopyRequest` (`POST /api/copy`).
#[derive(Debug, Clone, Serialize)]
pub struct CopyRequest {
    pub source: String,
    pub destination: String,
}

/// Cite: `api/types.go::DeleteRequest` (`DELETE /api/delete`).
#[derive(Debug, Clone, Serialize)]
pub struct DeleteRequest {
    pub model: String,
}

/// Cite: `api/types.go::EmbedRequest` (`POST /api/embed`).
#[derive(Debug, Clone, Serialize)]
pub struct EmbedRequest {
    pub model: String,
    pub input: EmbedInput,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<serde_json::Value>,
}

/// Cite: `api/types.go::EmbedRequest.Input` — the input is a JSON value that
/// may be a single string or an array of strings.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum EmbedInput {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Clone, Deserialize)]
pub struct EmbedResponse {
    pub model: String,
    pub embeddings: Vec<Vec<f64>>,
    #[serde(default)]
    pub total_duration: Option<u64>,
    #[serde(default)]
    pub load_duration: Option<u64>,
    #[serde(default)]
    pub prompt_eval_count: Option<u32>,
}

/// Cite: `api/types.go::ProcessModelResponse` — one entry returned by
/// `GET /api/ps` (the "loaded model" listing).
#[derive(Debug, Clone, Deserialize)]
pub struct RunningModel {
    pub name: String,
    pub model: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub size_vram: u64,
    #[serde(default)]
    pub digest: String,
    #[serde(default)]
    pub expires_at: String,
}

#[derive(Debug, Clone, Deserialize)]
struct PsResponse {
    models: Vec<RunningModel>,
}

/// Cite: ollama/ollama `api/types.go` lifecycle methods. cave's wrapper
/// reuses the daemon's [`OllamaClient`] HTTP client so timeouts + headers
/// remain consistent.
pub struct OllamaLifecycle<'a> {
    base_url: &'a str,
    client: Client,
}

impl<'a> OllamaLifecycle<'a> {
    pub fn from(client: &'a OllamaClient) -> Self {
        Self {
            base_url: client.base_url(),
            client: client.http_client().clone(),
        }
    }

    #[instrument(skip(self), fields(model = %req.model))]
    pub async fn show(&self, req: ShowRequest) -> OllamaResult<ShowResponse> {
        let response = self
            .client
            .post(format!("{}/api/show", self.base_url))
            .json(&req)
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(OllamaError::Api { status, body });
        }
        Ok(response.json().await?)
    }

    /// Cite: `api/types.go::CopyRequest` — atomic name rebind.
    #[instrument(skip(self), fields(src = %req.source, dst = %req.destination))]
    pub async fn copy(&self, req: CopyRequest) -> OllamaResult<()> {
        let response = self
            .client
            .post(format!("{}/api/copy", self.base_url))
            .json(&req)
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(OllamaError::Api { status, body });
        }
        Ok(())
    }

    #[instrument(skip(self), fields(model = %req.model))]
    pub async fn delete(&self, req: DeleteRequest) -> OllamaResult<()> {
        let response = self
            .client
            .delete(format!("{}/api/delete", self.base_url))
            .json(&req)
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(OllamaError::Api { status, body });
        }
        Ok(())
    }

    #[instrument(skip(self), fields(model = %req.model))]
    pub async fn embed(&self, req: EmbedRequest) -> OllamaResult<EmbedResponse> {
        let response = self
            .client
            .post(format!("{}/api/embed", self.base_url))
            .json(&req)
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(OllamaError::Api { status, body });
        }
        Ok(response.json().await?)
    }

    /// Cite: ollama/ollama `api/types.go::ListRunningResponse` (`GET /api/ps`).
    #[instrument(skip(self))]
    pub async fn list_running(&self) -> OllamaResult<Vec<RunningModel>> {
        let response = self
            .client
            .get(format!("{}/api/ps", self.base_url))
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(OllamaError::Api { status, body });
        }
        let body: PsResponse = response.json().await?;
        Ok(body.models)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embed_input_serializes_string_as_bare_value() {
        let req = EmbedRequest {
            model: "qwen".to_string(),
            input: EmbedInput::One("hello".to_string()),
            options: None,
        };
        let s = serde_json::to_string(&req).unwrap();
        assert!(s.contains("\"input\":\"hello\""), "got: {s}");
    }

    #[test]
    fn embed_input_serializes_vec_as_array() {
        let req = EmbedRequest {
            model: "qwen".to_string(),
            input: EmbedInput::Many(vec!["a".to_string(), "b".to_string()]),
            options: None,
        };
        let s = serde_json::to_string(&req).unwrap();
        assert!(s.contains("[\"a\",\"b\"]"), "got: {s}");
    }

    #[test]
    fn show_request_omits_verbose_when_none() {
        let req = ShowRequest {
            model: "qwen3".to_string(),
            verbose: None,
        };
        let s = serde_json::to_string(&req).unwrap();
        assert!(!s.contains("verbose"));
    }

    #[test]
    fn show_request_includes_verbose_when_set() {
        let req = ShowRequest {
            model: "qwen3".to_string(),
            verbose: Some(true),
        };
        let s = serde_json::to_string(&req).unwrap();
        assert!(s.contains("\"verbose\":true"));
    }

    #[test]
    fn copy_request_includes_both_names() {
        let req = CopyRequest {
            source: "a".to_string(),
            destination: "b".to_string(),
        };
        let s = serde_json::to_string(&req).unwrap();
        assert!(s.contains("\"source\":\"a\""));
        assert!(s.contains("\"destination\":\"b\""));
    }

    #[test]
    fn embed_response_deserializes_vector_array() {
        let raw = r#"{
            "model": "qwen",
            "embeddings": [[0.1, 0.2, 0.3], [0.4, 0.5, 0.6]]
        }"#;
        let r: EmbedResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(r.embeddings.len(), 2);
        assert_eq!(r.embeddings[0].len(), 3);
    }

    #[test]
    fn pull_status_deserializes_progress_frame() {
        let raw = r#"{"status":"pulling manifest","total":100,"completed":75}"#;
        let s: PullStatus = serde_json::from_str(raw).unwrap();
        assert_eq!(s.total, Some(100));
        assert_eq!(s.completed, Some(75));
    }

    #[test]
    fn running_model_deserializes() {
        let raw = r#"[{"name":"qwen3","model":"qwen3-coder-next:Q4_K_M","size":1024,"size_vram":512}]"#;
        let v: Vec<RunningModel> = serde_json::from_str(raw).unwrap();
        assert_eq!(v[0].name, "qwen3");
        assert_eq!(v[0].size_vram, 512);
    }
}
