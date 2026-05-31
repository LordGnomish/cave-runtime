// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Groq Cloud SaaS provider.
//!
//! Groq exposes an OpenAI-compatible surface at
//! `https://api.groq.com/openai/v1/chat/completions` — the same request and
//! response JSON as OpenAI, only the base host and model catalog differ.

use crate::error::{GatewayError, GatewayResult};
use crate::openai::{ChatCompletionRequest, ChatCompletionResponse};
use crate::provider::{LlmProvider, ProviderConfig};
use async_trait::async_trait;

/// OpenAI-compatible base (the gateway appends `/v1/chat/completions`).
pub const DEFAULT_BASE_URL: &str = "https://api.groq.com/openai";

pub struct GroqProvider {
    config: ProviderConfig,
    client: reqwest::Client,
}

impl GroqProvider {
    pub fn new(config: ProviderConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .expect("reqwest client");
        Self { config, client }
    }

    fn base(&self) -> &str {
        if self.config.base_url.is_empty() {
            DEFAULT_BASE_URL
        } else {
            self.config.base_url.as_str()
        }
    }
}

#[async_trait]
impl LlmProvider for GroqProvider {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn supported_models(&self) -> Vec<String> {
        vec![
            "llama-3.3-70b-versatile".into(),
            "llama-3.1-8b-instant".into(),
            "llama3-70b-8192".into(),
            "mixtral-8x7b-32768".into(),
            "gemma2-9b-it".into(),
        ]
    }

    async fn complete(&self, req: &ChatCompletionRequest) -> GatewayResult<ChatCompletionResponse> {
        let url = format!("{}/v1/chat/completions", self.base().trim_end_matches('/'));
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
        let url = format!("{}/v1/models", self.base().trim_end_matches('/'));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ProviderType;

    fn cfg() -> ProviderConfig {
        ProviderConfig {
            name: "groq-test".into(),
            provider_type: ProviderType::Groq,
            base_url: "".into(),
            api_key: Some("fake".into()),
            timeout_secs: 5,
            max_retries: 0,
            weight: 1,
            enabled: true,
        }
    }

    #[test]
    fn groq_default_base_url_is_openai_compat_host() {
        assert_eq!(DEFAULT_BASE_URL, "https://api.groq.com/openai");
    }

    #[test]
    fn groq_supported_models_include_llama_33() {
        let p = GroqProvider::new(cfg());
        assert!(p
            .supported_models()
            .iter()
            .any(|m| m == "llama-3.3-70b-versatile"));
    }

    #[tokio::test]
    async fn groq_health_check_with_dead_endpoint_fails() {
        let p = GroqProvider::new(ProviderConfig {
            base_url: "http://127.0.0.1:1".into(),
            ..cfg()
        });
        assert!(!p.health_check().await);
    }
}
