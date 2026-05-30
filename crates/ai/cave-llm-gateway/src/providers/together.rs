// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Together AI + Fireworks AI providers (OpenAI-compatible).
//!
//! Maps `litellm/llms/together_ai/` and `litellm/llms/fireworks_ai/`. Both
//! vendors expose an OpenAI-compatible `/v1/chat/completions` surface, so a
//! single adapter — parameterised by vendor (host + catalogue) — covers both.
//! Request/response bodies pass through unchanged; only the base URL and the
//! bearer key differ.

use crate::error::{GatewayError, GatewayResult};
use crate::openai::{ChatCompletionRequest, ChatCompletionResponse};
use crate::provider::{LlmProvider, ProviderConfig};
use async_trait::async_trait;

pub const TOGETHER_BASE_URL: &str = "https://api.together.xyz";
pub const FIREWORKS_BASE_URL: &str = "https://api.fireworks.ai/inference";

/// Which OpenAI-compatible SaaS this adapter is fronting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vendor {
    Together,
    Fireworks,
}

impl Vendor {
    pub fn default_base_url(self) -> &'static str {
        match self {
            Vendor::Together => TOGETHER_BASE_URL,
            Vendor::Fireworks => FIREWORKS_BASE_URL,
        }
    }
}

pub struct TogetherProvider {
    config: ProviderConfig,
    vendor: Vendor,
    client: reqwest::Client,
}

impl TogetherProvider {
    pub fn new(config: ProviderConfig) -> Self {
        Self::with_vendor(config, Vendor::Together)
    }

    pub fn fireworks(config: ProviderConfig) -> Self {
        Self::with_vendor(config, Vendor::Fireworks)
    }

    pub fn with_vendor(config: ProviderConfig, vendor: Vendor) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .expect("reqwest client");
        Self {
            config,
            vendor,
            client,
        }
    }

    fn base(&self) -> &str {
        if self.config.base_url.is_empty() {
            self.vendor.default_base_url()
        } else {
            self.config.base_url.as_str()
        }
    }

    /// OpenAI-compatible chat-completions endpoint for the configured vendor.
    pub fn chat_endpoint(&self) -> String {
        format!("{}/v1/chat/completions", self.base().trim_end_matches('/'))
    }
}

#[async_trait]
impl LlmProvider for TogetherProvider {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn supported_models(&self) -> Vec<String> {
        match self.vendor {
            Vendor::Together => vec![
                "meta-llama/Llama-3.3-70B-Instruct-Turbo".into(),
                "meta-llama/Meta-Llama-3.1-8B-Instruct-Turbo".into(),
                "mistralai/Mixtral-8x7B-Instruct-v0.1".into(),
                "Qwen/Qwen2.5-72B-Instruct-Turbo".into(),
            ],
            Vendor::Fireworks => vec![
                "accounts/fireworks/models/llama-v3p1-70b-instruct".into(),
                "accounts/fireworks/models/llama-v3p3-70b-instruct".into(),
                "accounts/fireworks/models/mixtral-8x22b-instruct".into(),
                "accounts/fireworks/models/qwen2p5-72b-instruct".into(),
            ],
        }
    }

    async fn complete(&self, req: &ChatCompletionRequest) -> GatewayResult<ChatCompletionResponse> {
        let url = self.chat_endpoint();
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

    fn cfg(t: ProviderType) -> ProviderConfig {
        ProviderConfig {
            name: "t-test".into(),
            provider_type: t,
            base_url: "".into(),
            api_key: Some("fake".into()),
            timeout_secs: 5,
            max_retries: 0,
            weight: 1,
            enabled: true,
        }
    }

    #[test]
    fn together_default_base_url_is_official_host() {
        assert_eq!(TOGETHER_BASE_URL, "https://api.together.xyz");
        let p = TogetherProvider::new(cfg(ProviderType::Together));
        assert_eq!(p.chat_endpoint(), "https://api.together.xyz/v1/chat/completions");
    }

    #[test]
    fn fireworks_default_base_url_is_inference_host() {
        assert_eq!(FIREWORKS_BASE_URL, "https://api.fireworks.ai/inference");
        let p = TogetherProvider::fireworks(cfg(ProviderType::Fireworks));
        assert_eq!(
            p.chat_endpoint(),
            "https://api.fireworks.ai/inference/v1/chat/completions"
        );
    }

    #[test]
    fn together_supported_models_includes_llama_turbo() {
        let p = TogetherProvider::new(cfg(ProviderType::Together));
        assert!(p
            .supported_models()
            .iter()
            .any(|m| m.contains("Llama-3.3-70B-Instruct-Turbo")));
    }

    #[test]
    fn fireworks_supported_models_use_account_path() {
        let p = TogetherProvider::fireworks(cfg(ProviderType::Fireworks));
        assert!(p
            .supported_models()
            .iter()
            .all(|m| m.starts_with("accounts/fireworks/models/")));
    }

    #[test]
    fn explicit_base_url_overrides_vendor_default() {
        let p = TogetherProvider::with_vendor(
            ProviderConfig {
                base_url: "http://localhost:9000".into(),
                ..cfg(ProviderType::Together)
            },
            Vendor::Together,
        );
        assert_eq!(p.chat_endpoint(), "http://localhost:9000/v1/chat/completions");
    }
}
