// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Mistral La Plateforme SaaS provider.
//!
//! Mistral's hosted API is OpenAI-compatible at
//! `https://api.mistral.ai/v1/chat/completions`. The only divergence is
//! the `/v1/models` endpoint, which returns the same JSON shape with the
//! Mistral catalog.

use crate::error::{GatewayError, GatewayResult};
use crate::openai::{ChatCompletionRequest, ChatCompletionResponse};
use crate::provider::{LlmProvider, ProviderConfig};
use async_trait::async_trait;

pub const DEFAULT_BASE_URL: &str = "https://api.mistral.ai";

pub struct MistralProvider {
    config: ProviderConfig,
    client: reqwest::Client,
}

impl MistralProvider {
    pub fn new(config: ProviderConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .expect("reqwest client");
        Self { config, client }
    }
}

#[async_trait]
impl LlmProvider for MistralProvider {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn supported_models(&self) -> Vec<String> {
        vec![
            "mistral-large-latest".into(),
            "mistral-medium-latest".into(),
            "mistral-small-latest".into(),
            "open-mistral-nemo".into(),
            "codestral-latest".into(),
        ]
    }

    async fn complete(&self, req: &ChatCompletionRequest) -> GatewayResult<ChatCompletionResponse> {
        let base = if self.config.base_url.is_empty() {
            DEFAULT_BASE_URL
        } else {
            self.config.base_url.as_str()
        };
        let url = format!("{}/v1/chat/completions", base.trim_end_matches('/'));
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
        let base = if self.config.base_url.is_empty() {
            DEFAULT_BASE_URL
        } else {
            self.config.base_url.as_str()
        };
        let url = format!("{}/v1/models", base.trim_end_matches('/'));
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
            name: "mistral-test".into(),
            provider_type: ProviderType::OpenAi,
            base_url: "".into(),
            api_key: Some("fake".into()),
            timeout_secs: 5,
            max_retries: 0,
            weight: 1,
            enabled: true,
        }
    }

    #[test]
    fn mistral_default_base_url_constant_is_official_host() {
        assert_eq!(DEFAULT_BASE_URL, "https://api.mistral.ai");
    }

    #[test]
    fn mistral_supported_models_includes_codestral() {
        let p = MistralProvider::new(cfg());
        assert!(p.supported_models().iter().any(|s| s.contains("codestral")));
    }

    #[tokio::test]
    async fn mistral_health_check_with_invalid_endpoint_fails_quickly() {
        let p = MistralProvider::new(ProviderConfig {
            base_url: "http://127.0.0.1:1".into(),
            ..cfg()
        });
        assert!(!p.health_check().await);
    }
}
