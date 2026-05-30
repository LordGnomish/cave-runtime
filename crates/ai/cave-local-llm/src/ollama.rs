// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Ollama HTTP client — GET /api/tags, POST /api/generate, POST /api/chat (streaming + non-streaming).

/// Default model for all Qwen amele operations.
/// Overridden at runtime by OLLAMA_MODEL env var or CLI --model flag.
pub const DEFAULT_MODEL: &str = "qwen3-coder-next:Q4_K_M";

use base64::Engine as _;
use futures::{Stream, StreamExt, TryStreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use thiserror::Error;
use tokio_util::codec::{FramedRead, LinesCodec, LinesCodecError};
use tokio_util::io::StreamReader;
use tracing::instrument;

#[derive(Debug, Error)]
pub enum OllamaError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Ollama API error {status}: {body}")]
    Api { status: u16, body: String },
    #[error("Stream decode error: {0}")]
    StreamDecode(String),
}

pub type OllamaResult<T> = Result<T, OllamaError>;

/// Boxed, pinned stream of `T` items from an Ollama streaming endpoint.
pub type OllamaStream<T> = Pin<Box<dyn Stream<Item = OllamaResult<T>> + Send>>;

/// Base64-encode raw image bytes for the `images` field of a generate/chat
/// request. Cite api/types.go `ImageData []byte` — Ollama serialises raw image
/// bytes as standard base64 strings on the wire for multimodal models (llava,
/// llama3.2-vision, …).
pub fn encode_image(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

// ── Request / response types ──────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ModelInfo {
    pub name: String,
    pub modified_at: String,
    pub size: u64,
    pub digest: String,
}

#[derive(Debug, Clone, Deserialize)]
struct TagsResponse {
    models: Vec<ModelInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GenerateRequest {
    pub model: String,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<serde_json::Value>,
    /// Keep-alive duration for the loaded model. Defaults to "24h" if unset so
    /// the model stays resident across daemon idle periods. Accepts Ollama
    /// duration strings ("5m", "1h", "24h") or "-1" for indefinite.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_alive: Option<String>,
    /// Base64-encoded images for multimodal models. Cite api/types.go
    /// `GenerateRequest.Images []ImageData`. Use [`encode_image`] to build
    /// entries from raw bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GenerateResponse {
    pub model: String,
    pub created_at: String,
    pub response: String,
    pub done: bool,
    #[serde(default)]
    pub total_duration: Option<u64>,
    #[serde(default)]
    pub load_duration: Option<u64>,
    #[serde(default)]
    pub prompt_eval_count: Option<u32>,
    #[serde(default)]
    pub eval_count: Option<u32>,
}

/// One NDJSON chunk from a streaming /api/generate response.
#[derive(Debug, Clone, Deserialize)]
pub struct GenerateChunk {
    pub model: String,
    pub created_at: String,
    pub response: String,
    pub done: bool,
    #[serde(default)]
    pub total_duration: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    /// Base64-encoded images for multimodal chat. Cite api/types.go
    /// `Message.Images []ImageData`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<String>>,
    /// Tool calls emitted by the model in an assistant turn. Cite
    /// api/types.go `Message.ToolCalls []ToolCall`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// Name of the tool this message is a result for. Cite api/types.go
    /// `Message.ToolName`. Set on `role: "tool"` messages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// ID of the originating tool call. Cite api/types.go `Message.ToolCallID`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

// ── Tool / function calling types — cite api/types.go ─────────────────────────

/// A tool the model may call. Cite api/types.go `Tool { Type, Function }`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    /// Tool kind; currently always `"function"`.
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: ToolFunction,
}

impl Tool {
    /// Build a `function`-type tool from a name, description, and a JSON-Schema
    /// `parameters` object (cite api/types.go `ToolFunctionParameters`, modelled
    /// here as a `serde_json::Value` schema for fidelity without over-typing).
    pub fn function(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: name.into(),
                description: Some(description.into()),
                parameters,
            },
        }
    }
}

/// Cite api/types.go `ToolFunction { Name, Description, Parameters }`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFunction {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// JSON-Schema parameter spec.
    pub parameters: serde_json::Value,
}

/// A tool invocation produced by the model. Cite api/types.go
/// `ToolCall { ID, Function ToolCallFunction }`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub function: ToolCallFunction,
}

/// Cite api/types.go `ToolCallFunction { Index, Name, Arguments }`. Arguments
/// is a free-form JSON object (upstream `ToolCallFunctionArguments`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFunction {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<i32>,
    pub name: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<serde_json::Value>,
    /// Tools the model may call. Cite api/types.go `ChatRequest.Tools`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatResponse {
    pub model: String,
    pub created_at: String,
    pub message: ChatMessage,
    pub done: bool,
    #[serde(default)]
    pub total_duration: Option<u64>,
}

/// One NDJSON chunk from a streaming /api/chat response.
#[derive(Debug, Clone, Deserialize)]
pub struct ChatChunk {
    pub model: String,
    pub created_at: String,
    pub message: ChatMessage,
    pub done: bool,
}

// ── Client ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct OllamaClient {
    base_url: String,
    client: Client,
}

/// Lightweight response from `GET /api/version`. Used only for liveness probes;
/// field is non-exhaustive so Ollama version bumps don't break parsing.
#[derive(Debug, Clone, Deserialize)]
pub struct VersionInfo {
    pub version: String,
}

impl OllamaClient {
    /// Returns the configured base URL (no trailing slash).
    /// Used by lifecycle/extras helpers that need to assemble auxiliary
    /// endpoints without re-wiring a second client.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Borrow the reqwest client so siblings can share its pool +
    /// timeouts. Avoids constructing duplicate clients with mismatched
    /// per-request defaults.
    pub fn http_client(&self) -> &Client {
        &self.client
    }

    pub fn new(base_url: impl Into<String>) -> Self {
        // Short per-request timeout so a hung Ollama doesn't stall a daemon tick
        // for minutes. health_check() is the dedicated liveness probe; real
        // generate calls still need to tolerate long inference — they use
        // their own longer timeout below via `client_generate()` if needed.
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

    /// Cheap liveness probe — `GET /api/version` with a short connect timeout.
    /// Used at daemon startup (and can be used as a periodic readiness check)
    /// to avoid hammering /api/generate when the server is unreachable.
    #[instrument(skip(self), fields(base_url = %self.base_url))]
    pub async fn health_check(&self) -> OllamaResult<VersionInfo> {
        let response = self
            .client
            .get(format!("{}/api/version", self.base_url))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(OllamaError::Api { status, body });
        }

        Ok(response.json().await?)
    }

    /// List available models (GET /api/tags).
    #[instrument(skip(self), fields(base_url = %self.base_url))]
    pub async fn list_models(&self) -> OllamaResult<Vec<ModelInfo>> {
        let response = self
            .client
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(OllamaError::Api { status, body });
        }

        let tags: TagsResponse = response.json().await?;
        Ok(tags.models)
    }

    /// Non-streaming generation (POST /api/generate, stream: false).
    #[instrument(skip(self, req), fields(model = %req.model))]
    pub async fn generate(&self, req: GenerateRequest) -> OllamaResult<GenerateResponse> {
        let mut r = req;
        r.stream = Some(false);
        if r.keep_alive.is_none() {
            r.keep_alive = Some("24h".to_string());
        }

        let response = self
            .client
            .post(format!("{}/api/generate", self.base_url))
            .json(&r)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(OllamaError::Api { status, body });
        }

        Ok(response.json().await?)
    }

    /// Streaming generation (POST /api/generate, stream: true).
    ///
    /// Returns a stream of NDJSON chunks; each chunk carries a partial `response` token.
    /// The final chunk has `done: true` and optional timing fields.
    #[instrument(skip(self, req), fields(model = %req.model))]
    pub async fn generate_stream(
        &self,
        req: GenerateRequest,
    ) -> OllamaResult<OllamaStream<GenerateChunk>> {
        let mut r = req;
        r.stream = Some(true);
        if r.keep_alive.is_none() {
            r.keep_alive = Some("24h".to_string());
        }

        let response = self
            .client
            .post(format!("{}/api/generate", self.base_url))
            .json(&r)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(OllamaError::Api { status, body });
        }

        let stream = build_ndjson_stream::<GenerateChunk>(response);
        Ok(Box::pin(stream))
    }

    /// Non-streaming chat (POST /api/chat, stream: false).
    #[instrument(skip(self, req), fields(model = %req.model))]
    pub async fn chat(&self, req: ChatRequest) -> OllamaResult<ChatResponse> {
        let mut r = req;
        r.stream = Some(false);

        let response = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&r)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(OllamaError::Api { status, body });
        }

        Ok(response.json().await?)
    }

    /// Streaming chat (POST /api/chat, stream: true).
    #[instrument(skip(self, req), fields(model = %req.model))]
    pub async fn chat_stream(&self, req: ChatRequest) -> OllamaResult<OllamaStream<ChatChunk>> {
        let mut r = req;
        r.stream = Some(true);

        let response = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&r)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(OllamaError::Api { status, body });
        }

        let stream = build_ndjson_stream::<ChatChunk>(response);
        Ok(Box::pin(stream))
    }
}

// ── Shared streaming helper ───────────────────────────────────────────────────

/// Converts a reqwest `Response` body into a stream of deserialized NDJSON items.
fn build_ndjson_stream<T>(response: reqwest::Response) -> impl Stream<Item = OllamaResult<T>> + Send
where
    T: for<'de> Deserialize<'de> + Send,
{
    let byte_stream = response
        .bytes_stream()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e));

    let reader = StreamReader::new(byte_stream);
    let framed = FramedRead::new(reader, LinesCodec::new());

    framed.filter_map(|line_result: Result<String, LinesCodecError>| {
        futures::future::ready(match line_result {
            Err(e) => Some(Err(OllamaError::StreamDecode(e.to_string()))),
            Ok(line) if line.trim().is_empty() => None,
            Ok(line) => Some(serde_json::from_str::<T>(&line).map_err(OllamaError::Json)),
        })
    })
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_request_serialises_stream_false() {
        let req = GenerateRequest {
            model: "qwen2.5-coder:32b".into(),
            prompt: "hello".into(),
            stream: Some(false),
            options: None,
            keep_alive: None,
            images: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"stream\":false"));
        assert!(!json.contains("options"));
    }

    #[test]
    fn test_generate_chunk_deserialises() {
        let raw =
            r#"{"model":"m","created_at":"2024-01-01T00:00:00Z","response":"fn ","done":false}"#;
        let chunk: GenerateChunk = serde_json::from_str(raw).unwrap();
        assert_eq!(chunk.response, "fn ");
        assert!(!chunk.done);
    }

    #[test]
    fn test_chat_chunk_deserialises() {
        let raw = r#"{"model":"m","created_at":"2024-01-01T00:00:00Z","message":{"role":"assistant","content":"hi"},"done":false}"#;
        let chunk: ChatChunk = serde_json::from_str(raw).unwrap();
        assert_eq!(chunk.message.content, "hi");
        assert_eq!(chunk.message.role, "assistant");
    }

    #[test]
    fn test_tags_response_deserialises() {
        let raw = r#"{"models":[{"name":"qwen2.5-coder:32b","modified_at":"2024-01-01T00:00:00Z","size":20000000000,"digest":"abc123"}]}"#;
        let tags: TagsResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(tags.models.len(), 1);
        assert_eq!(tags.models[0].name, "qwen2.5-coder:32b");
    }

    // ── Multimodal (image input) — cite api/types.go GenerateRequest.Images,
    //    Message.Images []ImageData (base64-encoded over the wire). ────────────
    #[test]
    fn test_encode_image_base64() {
        // 0x89 'P' 'N' 'G' -> standard base64
        assert_eq!(encode_image(b"\x89PNG"), "iVBORw==");
    }

    #[test]
    fn test_generate_request_serializes_images_when_set() {
        let req = GenerateRequest {
            model: "llava".into(),
            prompt: "describe this".into(),
            stream: Some(false),
            options: None,
            keep_alive: None,
            images: Some(vec![encode_image(b"\x89PNG")]),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(
            json.contains("\"images\":[\"iVBORw==\"]"),
            "expected base64 image array, got {json}"
        );
    }

    #[test]
    fn test_generate_request_omits_images_when_none() {
        let req = GenerateRequest {
            model: "m".into(),
            prompt: "p".into(),
            stream: None,
            options: None,
            keep_alive: None,
            images: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("images"), "images must be omitted, got {json}");
    }

    #[test]
    fn test_chat_message_with_images_serializes() {
        let m = ChatMessage {
            role: "user".into(),
            content: "what is in this picture".into(),
            images: Some(vec!["YWJj".into()]),
            ..Default::default()
        };
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"images\":[\"YWJj\"]"), "got {json}");
    }

    #[test]
    fn test_chat_message_without_images_omits_field() {
        let m = ChatMessage {
            role: "assistant".into(),
            content: "ok".into(),
            ..Default::default()
        };
        let json = serde_json::to_string(&m).unwrap();
        assert!(!json.contains("images"), "got {json}");
    }

    // ── Tool / function calling — cite api/types.go Tool, ToolFunction,
    //    ToolCall, ToolCallFunction; ChatRequest.Tools, Message.ToolCalls. ─────
    #[test]
    fn test_chat_request_serializes_tools() {
        let tool = Tool::function(
            "get_weather",
            "Get current weather for a city",
            serde_json::json!({
                "type": "object",
                "properties": {"city": {"type": "string"}},
                "required": ["city"]
            }),
        );
        let req = ChatRequest {
            model: "qwen3".into(),
            messages: vec![ChatMessage {
                role: "user".into(),
                content: "weather in Paris".into(),
                ..Default::default()
            }],
            tools: Some(vec![tool]),
            ..Default::default()
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"tools\":["), "got {json}");
        assert!(json.contains("\"type\":\"function\""), "got {json}");
        assert!(json.contains("\"name\":\"get_weather\""), "got {json}");
        assert!(json.contains("\"required\":[\"city\"]"), "got {json}");
    }

    #[test]
    fn test_chat_request_omits_tools_when_none() {
        let req = ChatRequest {
            model: "m".into(),
            messages: vec![],
            ..Default::default()
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("tools"), "got {json}");
    }

    #[test]
    fn test_chat_response_deserializes_tool_calls() {
        let raw = r#"{"model":"qwen3","created_at":"2024-01-01T00:00:00Z","message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"get_weather","arguments":{"city":"Paris"}}}]},"done":true}"#;
        let resp: ChatResponse = serde_json::from_str(raw).unwrap();
        let calls = resp.message.tool_calls.expect("tool_calls present");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "get_weather");
        assert_eq!(calls[0].function.arguments["city"], "Paris");
    }

    #[test]
    fn test_tool_result_message_serializes_tool_name() {
        let m = ChatMessage {
            role: "tool".into(),
            content: "{\"temp\":15}".into(),
            tool_name: Some("get_weather".into()),
            ..Default::default()
        };
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"tool_name\":\"get_weather\""), "got {json}");
    }
}
