// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! llama.cpp HTTP server provider.
//!
//! Talks to `llama-server` which speaks an OpenAI-compatible
//! `/v1/chat/completions` *and* a native `/completion` route. We use the
//! OpenAI route so the wire format matches the rest of the gateway, and
//! probe `/health` for readiness.

use crate::error::{GatewayError, GatewayResult};
use crate::openai::{ChatCompletionRequest, ChatCompletionResponse};
use crate::provider::{LlmProvider, ProviderConfig};
use async_trait::async_trait;

pub struct LlamaCppProvider {
    config: ProviderConfig,
    client: reqwest::Client,
}

impl LlamaCppProvider {
    pub fn new(config: ProviderConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .expect("reqwest client");
        Self { config, client }
    }
}

#[async_trait]
impl LlmProvider for LlamaCppProvider {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn supported_models(&self) -> Vec<String> {
        // llama.cpp serves whatever GGUF was loaded at startup; we report
        // generic IDs the capability router can match on.
        vec!["llamacpp".into(), "gguf".into()]
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
        // llama-server: `/health` returns 200 OK with `{"status":"ok"}`.
        let url = format!("{}/health", self.config.base_url.trim_end_matches('/'));
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
            name: "llamacpp-test".into(),
            provider_type: ProviderType::Local,
            base_url: "http://127.0.0.1:8080".into(),
            api_key: None,
            timeout_secs: 5,
            max_retries: 0,
            weight: 1,
            enabled: true,
        }
    }

    #[test]
    fn llama_cpp_supported_models_reports_gguf() {
        let p = LlamaCppProvider::new(cfg());
        assert!(p.supported_models().iter().any(|s| s == "gguf"));
    }

    #[tokio::test]
    async fn llama_cpp_health_check_returns_false_when_no_server() {
        let p = LlamaCppProvider::new(ProviderConfig {
            base_url: "http://127.0.0.1:1".into(),
            ..cfg()
        });
        assert!(!p.health_check().await);
    }
}
