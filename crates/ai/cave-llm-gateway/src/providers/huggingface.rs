// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HuggingFace Inference API provider (text-generation task).
//!
//! Maps `litellm/llms/huggingface/`. The serverless Inference API at
//! `https://api-inference.huggingface.co/models/{model}` takes a single
//! `inputs` string plus a `parameters` object and returns
//! `[{"generated_text": "..."}]`. We port the request build and response
//! decode as pure functions. HF does not report token usage, so usage is
//! estimated from word counts.

use crate::error::{GatewayError, GatewayResult};
use crate::openai::{ChatCompletionRequest, ChatCompletionResponse, Usage};
use crate::provider::{LlmProvider, ProviderConfig};
use crate::providers::replicate::flatten_prompt;
use async_trait::async_trait;

pub const DEFAULT_BASE_URL: &str = "https://api-inference.huggingface.co";

pub struct HuggingFaceProvider {
    config: ProviderConfig,
    client: reqwest::Client,
}

impl HuggingFaceProvider {
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

    /// Build the HF text-generation body: a flattened `inputs` prompt plus a
    /// `parameters` object. `return_full_text=false` so HF echoes only the
    /// completion, matching OpenAI semantics.
    pub fn to_hf_request(&self, req: &ChatCompletionRequest) -> serde_json::Value {
        let mut params = serde_json::Map::new();
        params.insert("return_full_text".into(), serde_json::json!(false));
        if let Some(mt) = req.max_tokens {
            params.insert("max_new_tokens".into(), serde_json::json!(mt));
        }
        if let Some(t) = req.temperature {
            params.insert("temperature".into(), serde_json::json!(t));
        }
        if let Some(p) = req.top_p {
            params.insert("top_p".into(), serde_json::json!(p));
        }
        serde_json::json!({
            "inputs": flatten_prompt(req),
            "parameters": serde_json::Value::Object(params),
        })
    }

    fn generate_url(&self, model: &str) -> String {
        format!("{}/models/{}", self.base().trim_end_matches('/'), model)
    }

    /// Decode the HF text-generation response. The success shape is an array of
    /// objects each carrying `generated_text`; a single object is also
    /// tolerated.
    pub fn from_hf_response(
        &self,
        val: &serde_json::Value,
        req: &ChatCompletionRequest,
    ) -> GatewayResult<ChatCompletionResponse> {
        let text = if let Some(arr) = val.as_array() {
            arr.first()
                .and_then(|o| o["generated_text"].as_str())
                .unwrap_or("")
                .to_string()
        } else {
            val["generated_text"].as_str().unwrap_or("").to_string()
        };
        if text.is_empty() {
            if let Some(err) = val["error"].as_str() {
                return Err(GatewayError::UpstreamError {
                    status: 502,
                    body: err.to_string(),
                });
            }
        }
        let prompt_tokens = flatten_prompt(req).split_whitespace().count() as u32;
        let completion_tokens = text.split_whitespace().count() as u32;
        Ok(ChatCompletionResponse::simple(
            &req.model,
            text,
            Usage::new(prompt_tokens, completion_tokens),
        ))
    }
}

#[async_trait]
impl LlmProvider for HuggingFaceProvider {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn supported_models(&self) -> Vec<String> {
        vec![
            "meta-llama/Meta-Llama-3-8B-Instruct".into(),
            "mistralai/Mistral-7B-Instruct-v0.3".into(),
            "HuggingFaceH4/zephyr-7b-beta".into(),
        ]
    }

    async fn complete(&self, req: &ChatCompletionRequest) -> GatewayResult<ChatCompletionResponse> {
        let url = self.generate_url(&req.model);
        let token = self.config.api_key.as_deref().unwrap_or("");
        let body = self.to_hf_request(req);

        let resp = self
            .client
            .post(&url)
            .bearer_auth(token)
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
        self.from_hf_response(&val, req)
    }

    async fn health_check(&self) -> bool {
        let token = self.config.api_key.as_deref().unwrap_or("");
        let url = format!("{}/models", self.base().trim_end_matches('/'));
        self.client
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openai::{ChatCompletionRequest, ChatMessage};
    use crate::provider::{ProviderConfig, ProviderType};

    fn cfg() -> ProviderConfig {
        ProviderConfig {
            name: "hf-test".into(),
            provider_type: ProviderType::HuggingFace,
            base_url: "".into(),
            api_key: Some("fake".into()),
            timeout_secs: 5,
            max_retries: 0,
            weight: 1,
            enabled: true,
        }
    }

    fn req() -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "meta-llama/Meta-Llama-3-8B-Instruct".into(),
            messages: vec![ChatMessage::user("ping")],
            temperature: Some(0.5),
            top_p: None,
            max_tokens: Some(32),
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
        }
    }

    #[test]
    fn hf_default_base_url() {
        assert_eq!(DEFAULT_BASE_URL, "https://api-inference.huggingface.co");
    }

    #[test]
    fn hf_generate_url_targets_model_path() {
        let p = HuggingFaceProvider::new(cfg());
        assert_eq!(
            p.generate_url("meta-llama/Meta-Llama-3-8B-Instruct"),
            "https://api-inference.huggingface.co/models/meta-llama/Meta-Llama-3-8B-Instruct"
        );
    }

    #[test]
    fn hf_request_wraps_inputs_and_parameters() {
        let p = HuggingFaceProvider::new(cfg());
        let body = p.to_hf_request(&req());
        assert!(body["inputs"].as_str().unwrap().contains("ping"));
        assert_eq!(body["parameters"]["max_new_tokens"], 32);
        assert_eq!(body["parameters"]["return_full_text"], false);
    }

    #[test]
    fn hf_response_array_extracts_generated_text() {
        let p = HuggingFaceProvider::new(cfg());
        let v = serde_json::json!([{"generated_text": "pong reply"}]);
        let resp = p.from_hf_response(&v, &req()).unwrap();
        assert_eq!(
            resp.choices[0].message.as_ref().unwrap().content.as_text().unwrap(),
            "pong reply"
        );
        assert_eq!(resp.usage.completion_tokens, 2);
    }

    #[test]
    fn hf_response_error_object_is_err() {
        let p = HuggingFaceProvider::new(cfg());
        let v = serde_json::json!({"error": "model is loading"});
        assert!(p.from_hf_response(&v, &req()).is_err());
    }
}
