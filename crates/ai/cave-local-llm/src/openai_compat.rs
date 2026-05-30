// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! OpenAI-compatible HTTP layer.
//!
//! Cite: ollama/ollama `docs/openai.md` v0.3.0 — the OpenAI-compatible
//! endpoints exposed at `/v1/chat/completions`, `/v1/completions`,
//! `/v1/embeddings`, `/v1/models`. Ollama transparently rewrites these
//! against its native `/api/*` surface; cave's `OpenAiCompatClient`
//! talks to the same `/v1/*` URLs so any OpenAI-Python-style caller
//! (e.g. `langchain`, `openai` SDK pointed at the local URL) works
//! against a local model.
//!
//! This module is a *client* — it talks to either Ollama's /v1 surface or
//! any other OpenAI-API-compatible server (vLLM, llama-cpp-python, LiteLLM
//! gateway). It does not host the surface; cave's HTTP-server portal lives
//! in cave-portal-api.

use crate::ollama::{OllamaClient, OllamaError, OllamaResult};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::instrument;

/// Cite: OpenAI `POST /v1/chat/completions` — `messages` is the canonical
/// chat history; `role` ∈ {system, user, assistant, tool}.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiChatMessage {
    pub role: String,
    pub content: String,
}

/// Cite: OpenAI `POST /v1/chat/completions` request body — narrowed to the
/// fields Ollama's /v1 endpoint actually honours.
#[derive(Debug, Clone, Serialize)]
pub struct OpenAiChatRequest {
    pub model: String,
    pub messages: Vec<OpenAiChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    /// `seed` is honored by Ollama for deterministic sampling — keeps the
    /// "amele draft" path reproducible.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenAiChatResponseChoice {
    pub index: u32,
    pub message: OpenAiChatMessage,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct OpenAiUsage {
    #[serde(default)]
    pub prompt_tokens: u32,
    #[serde(default)]
    pub completion_tokens: u32,
    #[serde(default)]
    pub total_tokens: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenAiChatResponse {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<OpenAiChatResponseChoice>,
    #[serde(default)]
    pub usage: OpenAiUsage,
}

/// Cite: OpenAI `POST /v1/completions` (legacy text-completion surface;
/// some clients still use it).
#[derive(Debug, Clone, Serialize)]
pub struct OpenAiCompletionRequest {
    pub model: String,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenAiCompletionChoice {
    pub index: u32,
    pub text: String,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenAiCompletionResponse {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<OpenAiCompletionChoice>,
    #[serde(default)]
    pub usage: OpenAiUsage,
}

/// Cite: OpenAI `POST /v1/embeddings`.
#[derive(Debug, Clone, Serialize)]
pub struct OpenAiEmbeddingRequest {
    pub model: String,
    pub input: OpenAiEmbeddingInput,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum OpenAiEmbeddingInput {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenAiEmbeddingData {
    pub index: u32,
    pub embedding: Vec<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenAiEmbeddingResponse {
    pub object: String,
    pub model: String,
    pub data: Vec<OpenAiEmbeddingData>,
    #[serde(default)]
    pub usage: OpenAiUsage,
}

/// Cite: OpenAI `GET /v1/models`.
#[derive(Debug, Clone, Deserialize)]
pub struct OpenAiModelEntry {
    pub id: String,
    pub object: String,
    #[serde(default)]
    pub created: u64,
    #[serde(default)]
    pub owned_by: String,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAiModelList {
    #[allow(dead_code)]
    object: String,
    data: Vec<OpenAiModelEntry>,
}

pub struct OpenAiCompatClient {
    base_url: String,
    client: Client,
    api_key: Option<String>,
}

impl OpenAiCompatClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .connect_timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("reqwest client build");
        Self {
            base_url: base_url.into(),
            client,
            api_key: None,
        }
    }

    /// Wraps an existing [`OllamaClient`] — useful when the daemon already
    /// owns one for `/api/*` and we want a sibling /v1 client without a
    /// second connection pool.
    pub fn from_ollama(client: &OllamaClient) -> Self {
        Self {
            base_url: client.base_url().to_string(),
            client: client.http_client().clone(),
            api_key: None,
        }
    }

    /// Configure a bearer token for upstream OpenAI-compat servers that
    /// require auth (LiteLLM, vLLM with `--api-key`). Local Ollama does not
    /// require this.
    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    fn auth_header(&self) -> Option<(&'static str, String)> {
        self.api_key
            .as_ref()
            .map(|k| ("Authorization", format!("Bearer {k}")))
    }

    /// Cite: OpenAI `POST /v1/chat/completions`.
    #[instrument(skip(self, req), fields(model = %req.model))]
    pub async fn chat_completions(
        &self,
        req: OpenAiChatRequest,
    ) -> OllamaResult<OpenAiChatResponse> {
        let mut rb = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .json(&req);
        if let Some((k, v)) = self.auth_header() {
            rb = rb.header(k, v);
        }
        let response = rb.send().await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(OllamaError::Api { status, body });
        }
        Ok(response.json().await?)
    }

    /// Cite: OpenAI `POST /v1/completions`.
    #[instrument(skip(self, req), fields(model = %req.model))]
    pub async fn completions(
        &self,
        req: OpenAiCompletionRequest,
    ) -> OllamaResult<OpenAiCompletionResponse> {
        let mut rb = self
            .client
            .post(format!("{}/v1/completions", self.base_url))
            .json(&req);
        if let Some((k, v)) = self.auth_header() {
            rb = rb.header(k, v);
        }
        let response = rb.send().await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(OllamaError::Api { status, body });
        }
        Ok(response.json().await?)
    }

    #[instrument(skip(self, req), fields(model = %req.model))]
    pub async fn embeddings(
        &self,
        req: OpenAiEmbeddingRequest,
    ) -> OllamaResult<OpenAiEmbeddingResponse> {
        let mut rb = self
            .client
            .post(format!("{}/v1/embeddings", self.base_url))
            .json(&req);
        if let Some((k, v)) = self.auth_header() {
            rb = rb.header(k, v);
        }
        let response = rb.send().await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(OllamaError::Api { status, body });
        }
        Ok(response.json().await?)
    }

    #[instrument(skip(self))]
    pub async fn models(&self) -> OllamaResult<Vec<OpenAiModelEntry>> {
        let mut rb = self.client.get(format!("{}/v1/models", self.base_url));
        if let Some((k, v)) = self.auth_header() {
            rb = rb.header(k, v);
        }
        let response = rb.send().await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(OllamaError::Api { status, body });
        }
        let body: OpenAiModelList = response.json().await?;
        Ok(body.data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_request_serializes_with_min_fields() {
        let req = OpenAiChatRequest {
            model: "qwen3".into(),
            messages: vec![OpenAiChatMessage {
                role: "user".into(),
                content: "hi".into(),
            }],
            temperature: None,
            max_tokens: None,
            top_p: None,
            stream: None,
            seed: None,
            stop: None,
        };
        let s = serde_json::to_string(&req).unwrap();
        assert!(s.contains("\"model\":\"qwen3\""));
        assert!(!s.contains("temperature"));
    }

    #[test]
    fn chat_request_includes_seed_when_set() {
        let req = OpenAiChatRequest {
            model: "qwen3".into(),
            messages: vec![],
            temperature: Some(0.0),
            max_tokens: Some(128),
            top_p: None,
            stream: Some(false),
            seed: Some(42),
            stop: Some(vec!["</s>".to_string()]),
        };
        let s = serde_json::to_string(&req).unwrap();
        assert!(s.contains("\"seed\":42"));
        assert!(s.contains("\"stop\":[\"</s>\"]"));
        assert!(s.contains("\"max_tokens\":128"));
    }

    #[test]
    fn embedding_input_one_serializes_as_string() {
        let req = OpenAiEmbeddingRequest {
            model: "qwen".into(),
            input: OpenAiEmbeddingInput::One("hello".into()),
        };
        let s = serde_json::to_string(&req).unwrap();
        assert!(s.contains("\"input\":\"hello\""));
    }

    #[test]
    fn embedding_input_many_serializes_as_array() {
        let req = OpenAiEmbeddingRequest {
            model: "qwen".into(),
            input: OpenAiEmbeddingInput::Many(vec!["a".into(), "b".into()]),
        };
        let s = serde_json::to_string(&req).unwrap();
        assert!(s.contains("[\"a\",\"b\"]"));
    }

    #[test]
    fn chat_response_deserializes_canonical_shape() {
        let raw = r#"{
            "id": "chatcmpl-1",
            "object": "chat.completion",
            "created": 1700000000,
            "model": "qwen3",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 5, "completion_tokens": 1, "total_tokens": 6}
        }"#;
        let r: OpenAiChatResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(r.choices.len(), 1);
        assert_eq!(r.choices[0].message.content, "hi");
        assert_eq!(r.usage.total_tokens, 6);
    }

    #[test]
    fn completion_response_deserializes() {
        let raw = r#"{
            "id":"cmpl-1","object":"text_completion","created":1700000000,"model":"qwen3",
            "choices":[{"index":0,"text":"world","finish_reason":"stop"}]
        }"#;
        let r: OpenAiCompletionResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(r.choices[0].text, "world");
    }

    #[test]
    fn embedding_response_deserializes() {
        let raw = r#"{
            "object":"list","model":"qwen",
            "data":[{"index":0,"embedding":[0.1,0.2,0.3]}]
        }"#;
        let r: OpenAiEmbeddingResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(r.data[0].embedding.len(), 3);
    }

    #[test]
    fn models_list_deserializes() {
        let raw = r#"{
            "object":"list",
            "data":[{"id":"qwen3","object":"model","created":1700000000,"owned_by":"ollama"}]
        }"#;
        let r: OpenAiModelList = serde_json::from_str(raw).unwrap();
        assert_eq!(r.data.len(), 1);
        assert_eq!(r.data[0].id, "qwen3");
    }

    #[test]
    fn auth_header_emits_bearer_when_set() {
        let c = OpenAiCompatClient::new("http://x").with_api_key("k");
        let h = c.auth_header().unwrap();
        assert_eq!(h.0, "Authorization");
        assert_eq!(h.1, "Bearer k");
    }

    // ── Streaming — cite docs/openai.md: SSE `chat.completion.chunk` frames
    //    delimited by `data: ` lines, terminated by `data: [DONE]`. ────────────
    #[test]
    fn chat_chunk_deserializes_delta() {
        let raw = r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","created":1700000000,"model":"qwen3","choices":[{"index":0,"delta":{"content":"He"},"finish_reason":null}]}"#;
        let c: OpenAiChatChunk = serde_json::from_str(raw).unwrap();
        assert_eq!(c.choices[0].delta.content.as_deref(), Some("He"));
        assert!(c.choices[0].finish_reason.is_none());
    }

    #[test]
    fn parse_sse_line_extracts_data_chunk() {
        let line = r#"data: {"id":"x","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"content":"hi"}}]}"#;
        match parse_sse_line(line) {
            Some(Ok(chunk)) => {
                assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("hi"))
            }
            other => panic!("expected Some(Ok(chunk)), got {other:?}"),
        }
    }

    #[test]
    fn parse_sse_line_done_terminates() {
        assert!(parse_sse_line("data: [DONE]").is_none());
    }

    #[test]
    fn parse_sse_line_ignores_blank_and_comments() {
        assert!(parse_sse_line("").is_none());
        assert!(parse_sse_line(": keep-alive comment").is_none());
    }
}
