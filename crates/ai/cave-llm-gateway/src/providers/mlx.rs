// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! MLX-LM HTTP server provider (Apple-silicon native inference).
//!
//! `mlx_lm.server` exposes an OpenAI-compatible `/v1/chat/completions`
//! endpoint. Health is detected via `/v1/models`.

use crate::error::{GatewayError, GatewayResult};
use crate::openai::{ChatCompletionRequest, ChatCompletionResponse};
use crate::provider::{LlmProvider, ProviderConfig};
use async_trait::async_trait;

pub struct MlxProvider {
    config: ProviderConfig,
    client: reqwest::Client,
}

impl MlxProvider {
    pub fn new(config: ProviderConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .expect("reqwest client");
        Self { config, client }
    }
}

#[async_trait]
impl LlmProvider for MlxProvider {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn supported_models(&self) -> Vec<String> {
        vec![
            "mlx-community/Llama-3.1-8B-Instruct-4bit".into(),
            "mlx-community/Mistral-7B-Instruct-v0.3-4bit".into(),
            "mlx-community/Qwen2.5-7B-Instruct-4bit".into(),
        ]
    }

    async fn complete(&self, req: &ChatCompletionRequest) -> GatewayResult<ChatCompletionResponse> {
        let url = format!(
            "{}/v1/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        let resp = self
            .client
            .post(&url)
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
        let url = format!("{}/v1/models", self.config.base_url.trim_end_matches('/'));
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
            name: "mlx-test".into(),
            provider_type: ProviderType::Local,
            base_url: "http://127.0.0.1:8081".into(),
            api_key: None,
            timeout_secs: 5,
            max_retries: 0,
            weight: 1,
            enabled: true,
        }
    }

    #[test]
    fn mlx_supported_models_includes_qwen() {
        let p = MlxProvider::new(cfg());
        assert!(p.supported_models().iter().any(|s| s.contains("Qwen")));
    }

    #[tokio::test]
    async fn mlx_health_check_returns_false_when_no_server() {
        let p = MlxProvider::new(ProviderConfig {
            base_url: "http://127.0.0.1:1".into(),
            ..cfg()
        });
        assert!(!p.health_check().await);
    }
}
