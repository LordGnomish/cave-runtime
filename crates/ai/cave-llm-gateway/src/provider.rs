// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! LLM provider abstraction — OpenAI, Anthropic, local models.

use crate::error::{GatewayError, GatewayResult};
use crate::openai::{ChatCompletionRequest, ChatCompletionResponse, Usage};
use async_trait::async_trait;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// ── Provider trait ────────────────────────────────────────────────────────────

#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> &str;
    fn supported_models(&self) -> Vec<String>;
    async fn complete(&self, req: &ChatCompletionRequest) -> GatewayResult<ChatCompletionResponse>;
    async fn health_check(&self) -> bool;
}

// ── Provider config ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub name: String,
    pub provider_type: ProviderType,
    pub base_url: String,
    pub api_key: Option<String>,
    pub timeout_secs: u64,
    pub max_retries: u32,
    pub weight: u32,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderType {
    OpenAi,
    Anthropic,
    /// Ollama native `/api/chat` protocol.
    Ollama,
    /// llama.cpp `llama-server` (OpenAI-compatible).
    LlamaCpp,
    /// MLX-LM HTTP server (OpenAI-compatible).
    Mlx,
    /// Mistral La Plateforme SaaS (OpenAI-compatible).
    Mistral,
    /// Cohere Command v2 chat API (divergent response shape).
    Cohere,
    /// Google Gemini / Vertex AI (`generateContent` protocol).
    Google,
    /// Generic OpenAI-compatible local endpoint (Ollama OpenAI shim, vLLM, LM Studio).
    Local,
    Mock,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            name: "default".into(),
            provider_type: ProviderType::Mock,
            base_url: "http://localhost:8080".into(),
            api_key: None,
            timeout_secs: 60,
            max_retries: 3,
            weight: 1,
            enabled: true,
        }
    }
}

// ── OpenAI provider ───────────────────────────────────────────────────────────

pub struct OpenAiProvider {
    config: ProviderConfig,
    client: reqwest::Client,
}

impl OpenAiProvider {
    pub fn new(config: ProviderConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .expect("reqwest client");
        Self { config, client }
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn supported_models(&self) -> Vec<String> {
        vec![
            "gpt-4o".into(),
            "gpt-4o-mini".into(),
            "gpt-4-turbo".into(),
            "gpt-4".into(),
            "gpt-3.5-turbo".into(),
        ]
    }

    async fn complete(&self, req: &ChatCompletionRequest) -> GatewayResult<ChatCompletionResponse> {
        let url = format!("{}/v1/chat/completions", self.config.base_url);
        let api_key = self.config.api_key.as_deref().unwrap_or("");

        let resp = self
            .client
            .post(&url)
            .bearer_auth(api_key)
            .json(req)
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

        resp.json::<ChatCompletionResponse>()
            .await
            .map_err(|e| GatewayError::ProviderUnavailable {
                provider: self.config.name.clone(),
                reason: format!("deserialize: {e}"),
            })
    }

    async fn health_check(&self) -> bool {
        let url = format!("{}/v1/models", self.config.base_url);
        let api_key = self.config.api_key.as_deref().unwrap_or("");
        self.client
            .get(&url)
            .bearer_auth(api_key)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}

// ── Anthropic provider ────────────────────────────────────────────────────────

pub struct AnthropicProvider {
    config: ProviderConfig,
    client: reqwest::Client,
}

impl AnthropicProvider {
    pub fn new(config: ProviderConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .expect("reqwest client");
        Self { config, client }
    }

    fn to_anthropic_request(&self, req: &ChatCompletionRequest) -> serde_json::Value {
        let system = req
            .messages
            .iter()
            .find(|m| m.role == crate::openai::Role::System)
            .and_then(|m| m.content.as_text())
            .map(|s| s.to_string());

        let messages: Vec<serde_json::Value> = req
            .messages
            .iter()
            .filter(|m| m.role != crate::openai::Role::System)
            .map(|m| {
                serde_json::json!({
                    "role": match m.role {
                        crate::openai::Role::User => "user",
                        crate::openai::Role::Assistant => "assistant",
                        _ => "user",
                    },
                    "content": m.content.as_text().unwrap_or(""),
                })
            })
            .collect();

        let mut body = serde_json::json!({
            "model": req.model,
            "messages": messages,
            "max_tokens": req.max_tokens.unwrap_or(4096),
        });

        if let Some(sys) = system {
            body["system"] = serde_json::json!(sys);
        }
        if let Some(t) = req.temperature {
            body["temperature"] = serde_json::json!(t);
        }
        body
    }

    fn from_anthropic_response(
        &self,
        val: serde_json::Value,
        model: &str,
    ) -> GatewayResult<ChatCompletionResponse> {
        let content = val["content"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|c| c["text"].as_str())
            .unwrap_or("")
            .to_string();

        let input_tokens = val["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32;
        let output_tokens = val["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32;

        Ok(ChatCompletionResponse::simple(
            model,
            content,
            Usage::new(input_tokens, output_tokens),
        ))
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn supported_models(&self) -> Vec<String> {
        vec![
            "claude-opus-4-6".into(),
            "claude-sonnet-4-6".into(),
            "claude-haiku-4-5-20251001".into(),
            "claude-3-5-sonnet-20241022".into(),
            "claude-3-5-haiku-20241022".into(),
        ]
    }

    async fn complete(&self, req: &ChatCompletionRequest) -> GatewayResult<ChatCompletionResponse> {
        let url = format!("{}/v1/messages", self.config.base_url);
        let api_key = self.config.api_key.as_deref().unwrap_or("");
        let body = self.to_anthropic_request(req);
        let model = req.model.clone();

        let resp = self
            .client
            .post(&url)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
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

        let val: serde_json::Value =
            resp.json()
                .await
                .map_err(|e| GatewayError::ProviderUnavailable {
                    provider: self.config.name.clone(),
                    reason: format!("deserialize: {e}"),
                })?;

        self.from_anthropic_response(val, &model)
    }

    async fn health_check(&self) -> bool {
        // Anthropic doesn't have a cheap ping endpoint; we skip real check
        true
    }
}

// ── Local / mock provider ─────────────────────────────────────────────────────

pub struct LocalProvider {
    config: ProviderConfig,
    client: reqwest::Client,
}

impl LocalProvider {
    pub fn new(config: ProviderConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .expect("reqwest client");
        Self { config, client }
    }
}

#[async_trait]
impl LlmProvider for LocalProvider {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn supported_models(&self) -> Vec<String> {
        vec![
            "llama3".into(),
            "mistral".into(),
            "phi3".into(),
            "gemma".into(),
        ]
    }

    async fn complete(&self, req: &ChatCompletionRequest) -> GatewayResult<ChatCompletionResponse> {
        // Assumes OpenAI-compatible endpoint (e.g., Ollama, vLLM, LM Studio)
        let url = format!("{}/v1/chat/completions", self.config.base_url);

        let resp = self.client.post(&url).json(req).send().await.map_err(|e| {
            GatewayError::ProviderUnavailable {
                provider: self.config.name.clone(),
                reason: e.to_string(),
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(GatewayError::UpstreamError { status, body });
        }

        resp.json::<ChatCompletionResponse>()
            .await
            .map_err(|e| GatewayError::ProviderUnavailable {
                provider: self.config.name.clone(),
                reason: format!("deserialize: {e}"),
            })
    }

    async fn health_check(&self) -> bool {
        let url = format!("{}/v1/models", self.config.base_url);
        self.client
            .get(&url)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}

/// Mock provider for tests — echos back a canned response.
pub struct MockProvider {
    pub name: String,
    pub models: Vec<String>,
}

impl MockProvider {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            models: vec!["mock-model".into()],
        }
    }
}

#[async_trait]
impl LlmProvider for MockProvider {
    fn name(&self) -> &str {
        &self.name
    }
    fn supported_models(&self) -> Vec<String> {
        self.models.clone()
    }
    async fn complete(&self, req: &ChatCompletionRequest) -> GatewayResult<ChatCompletionResponse> {
        let last = req
            .messages
            .last()
            .and_then(|m| m.content.as_text())
            .unwrap_or("")
            .to_string();
        let reply = format!("Mock response to: {last}");
        Ok(ChatCompletionResponse::simple(
            &req.model,
            reply,
            Usage::new(10, 20),
        ))
    }
    async fn health_check(&self) -> bool {
        true
    }
}

// ── Provider registry ─────────────────────────────────────────────────────────

pub struct ProviderRegistry {
    providers: DashMap<String, Arc<dyn LlmProvider>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            providers: DashMap::new(),
        }
    }

    pub fn register(&self, provider: Arc<dyn LlmProvider>) {
        self.providers.insert(provider.name().to_string(), provider);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn LlmProvider>> {
        self.providers.get(name).map(|p| Arc::clone(&*p))
    }

    pub fn list(&self) -> Vec<String> {
        self.providers.iter().map(|e| e.key().clone()).collect()
    }

    pub fn from_config(configs: Vec<ProviderConfig>) -> Self {
        let r = Self::new();
        for cfg in configs {
            let provider: Arc<dyn LlmProvider> = match cfg.provider_type {
                ProviderType::OpenAi => Arc::new(OpenAiProvider::new(cfg)),
                ProviderType::Anthropic => Arc::new(AnthropicProvider::new(cfg)),
                ProviderType::Ollama => Arc::new(crate::providers::ollama::OllamaProvider::new(cfg)),
                ProviderType::LlamaCpp => {
                    Arc::new(crate::providers::llama_cpp::LlamaCppProvider::new(cfg))
                }
                ProviderType::Mlx => Arc::new(crate::providers::mlx::MlxProvider::new(cfg)),
                ProviderType::Mistral => {
                    Arc::new(crate::providers::mistral::MistralProvider::new(cfg))
                }
                ProviderType::Cohere => {
                    Arc::new(crate::providers::cohere::CohereProvider::new(cfg))
                }
                ProviderType::Google => {
                    Arc::new(crate::providers::google::GoogleProvider::new(cfg))
                }
                ProviderType::Local => Arc::new(LocalProvider::new(cfg)),
                ProviderType::Mock => Arc::new(MockProvider::new(cfg.name)),
            };
            r.register(provider);
        }
        r
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        let r = Self::new();
        r.register(Arc::new(MockProvider::new("mock")));
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openai::ChatMessage;

    #[tokio::test]
    async fn mock_provider_responds() {
        let p = MockProvider::new("test");
        let req = ChatCompletionRequest {
            model: "mock-model".into(),
            messages: vec![ChatMessage::user("hello")],
            temperature: None,
            top_p: None,
            max_tokens: None,
            stream: None,
            stop: None,
            presence_penalty: None,
            frequency_penalty: None,
            n: None,
            user: None,
            tools: None,
            tool_choice: None,
            response_format: None,
            seed: None,
            logprobs: None,
        };
        let resp = p.complete(&req).await.unwrap();
        assert!(
            resp.choices[0]
                .message
                .as_ref()
                .unwrap()
                .content
                .as_text()
                .unwrap()
                .contains("Mock response")
        );
    }

    #[test]
    fn registry_register_and_get() {
        let registry = ProviderRegistry::new();
        registry.register(Arc::new(MockProvider::new("my-mock")));
        assert!(registry.get("my-mock").is_some());
        assert!(registry.get("nonexistent").is_none());
    }
}
