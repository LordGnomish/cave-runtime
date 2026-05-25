// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Ollama provider — talks the native `/api/chat` protocol.
//!
//! Ollama also exposes an OpenAI-compatible shim on `/v1/chat/completions`,
//! but the native API exposes additional fields (`eval_count`,
//! `prompt_eval_count`, model digest) that we use for accurate token
//! accounting in the cost ledger.

use crate::error::{GatewayError, GatewayResult};
use crate::openai::{ChatCompletionRequest, ChatCompletionResponse, Role, Usage};
use crate::provider::{LlmProvider, ProviderConfig};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub struct OllamaProvider {
    config: ProviderConfig,
    client: reqwest::Client,
}

impl OllamaProvider {
    pub fn new(config: ProviderConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .expect("reqwest client");
        Self { config, client }
    }

    pub fn base_url(&self) -> &str {
        &self.config.base_url
    }
}

#[derive(Debug, Serialize)]
struct OllamaChatRequest<'a> {
    model: &'a str,
    messages: Vec<OllamaMessage<'a>>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
}

#[derive(Debug, Serialize)]
struct OllamaMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Serialize, Default)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_predict: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct OllamaChatResponse {
    model: String,
    message: OllamaResponseMessage,
    #[serde(default)]
    prompt_eval_count: u64,
    #[serde(default)]
    eval_count: u64,
}

#[derive(Debug, Deserialize)]
struct OllamaResponseMessage {
    #[allow(dead_code)]
    role: String,
    content: String,
    /// "Thinking" models (qwen3-*, deepseek-r1-*, etc.) place their
    /// reasoning trace here. If `content` is empty we fall back to
    /// this string so the gateway never silently returns empty bodies.
    #[serde(default)]
    thinking: Option<String>,
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn supported_models(&self) -> Vec<String> {
        // Static seed; the live `/api/tags` endpoint can extend at runtime.
        vec![
            "llama3.1".into(),
            "llama3".into(),
            "mistral".into(),
            "qwen2.5".into(),
            "phi3".into(),
            "gemma2".into(),
        ]
    }

    async fn complete(&self, req: &ChatCompletionRequest) -> GatewayResult<ChatCompletionResponse> {
        let url = format!("{}/api/chat", self.config.base_url.trim_end_matches('/'));
        let mut messages: Vec<OllamaMessage<'_>> = Vec::with_capacity(req.messages.len());
        for m in &req.messages {
            let role = match m.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool | Role::Function => "tool",
            };
            messages.push(OllamaMessage {
                role,
                content: m.content.as_text().unwrap_or(""),
            });
        }
        let body = OllamaChatRequest {
            model: &req.model,
            messages,
            stream: false,
            options: Some(OllamaOptions {
                temperature: req.temperature,
                top_p: req.top_p,
                num_predict: req.max_tokens,
                seed: req.seed,
            }),
        };

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| GatewayError::ProviderUnavailable {
                provider: self.config.name.clone(),
                reason: e.to_string(),
            })?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(GatewayError::UpstreamError { status, body });
        }
        let parsed: OllamaChatResponse =
            resp.json()
                .await
                .map_err(|e| GatewayError::ProviderUnavailable {
                    provider: self.config.name.clone(),
                    reason: format!("deserialize: {e}"),
                })?;

        let text = if !parsed.message.content.is_empty() {
            parsed.message.content
        } else {
            parsed.message.thinking.unwrap_or_default()
        };
        Ok(ChatCompletionResponse::simple(
            &parsed.model,
            text,
            Usage::new(parsed.prompt_eval_count as u32, parsed.eval_count as u32),
        ))
    }

    async fn health_check(&self) -> bool {
        let url = format!("{}/api/tags", self.config.base_url.trim_end_matches('/'));
        self.client
            .get(&url)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ProviderType;

    fn cfg() -> ProviderConfig {
        ProviderConfig {
            name: "ollama-test".into(),
            provider_type: ProviderType::Local,
            base_url: "http://127.0.0.1:11434".into(),
            api_key: None,
            timeout_secs: 5,
            max_retries: 0,
            weight: 1,
            enabled: true,
        }
    }

    #[test]
    fn ollama_supported_models_includes_llama3() {
        let p = OllamaProvider::new(cfg());
        let m = p.supported_models();
        assert!(m.iter().any(|s| s.starts_with("llama3")));
    }

    #[test]
    fn ollama_base_url_is_what_we_configured() {
        let p = OllamaProvider::new(cfg());
        assert_eq!(p.base_url(), "http://127.0.0.1:11434");
    }

    #[tokio::test]
    async fn ollama_health_check_returns_false_when_no_server() {
        let p = OllamaProvider::new(ProviderConfig {
            base_url: "http://127.0.0.1:1".into(),
            ..cfg()
        });
        assert!(!p.health_check().await);
    }
}
