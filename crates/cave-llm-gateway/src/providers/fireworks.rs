// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Fireworks AI SaaS provider.
//!
//! Fireworks AI exposes an OpenAI-compatible surface at
//! `https://api.fireworks.ai/inference/v1/chat/completions` — identical
//! request and response JSON to OpenAI, differing only in base host and the
//! `accounts/fireworks/models/*` model namespace.

use crate::error::{GatewayError, GatewayResult};
use crate::openai::{ChatCompletionRequest, ChatCompletionResponse};
use crate::provider::{LlmProvider, ProviderConfig};
use async_trait::async_trait;

/// OpenAI-compatible base (the gateway appends `/v1/chat/completions`).
pub const DEFAULT_BASE_URL: &str = "https://api.fireworks.ai/inference";

pub struct FireworksProvider {
    config: ProviderConfig,
    client: reqwest::Client,
}

impl FireworksProvider {
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
impl LlmProvider for FireworksProvider {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn supported_models(&self) -> Vec<String> {
        vec![
            "accounts/fireworks/models/llama-v3p3-70b-instruct".into(),
            "accounts/fireworks/models/llama-v3p1-8b-instruct".into(),
            "accounts/fireworks/models/mixtral-8x22b-instruct".into(),
            "accounts/fireworks/models/qwen2p5-72b-instruct".into(),
            "accounts/fireworks/models/deepseek-v3".into(),
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
            name: "fireworks-test".into(),
            provider_type: ProviderType::Fireworks,
            base_url: "".into(),
            api_key: Some("fake".into()),
            timeout_secs: 5,
            max_retries: 0,
            weight: 1,
            enabled: true,
        }
    }

    #[test]
    fn fireworks_default_base_url_is_inference_host() {
        assert_eq!(DEFAULT_BASE_URL, "https://api.fireworks.ai/inference");
    }

    #[test]
    fn fireworks_supported_models_use_accounts_namespace() {
        let p = FireworksProvider::new(cfg());
        assert!(p
            .supported_models()
            .iter()
            .all(|m| m.starts_with("accounts/fireworks/models/")));
    }

    #[tokio::test]
    async fn fireworks_health_check_with_dead_endpoint_fails() {
        let p = FireworksProvider::new(ProviderConfig {
            base_url: "http://127.0.0.1:1".into(),
            ..cfg()
        });
        assert!(!p.health_check().await);
    }
}
