// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cohere Command provider (v2 chat API).
//!
//! Maps `litellm/llms/cohere/chat/transformation.py`. Cohere's v2 endpoint
//! `https://api.cohere.com/v2/chat` accepts OpenAI-style `messages` but
//! returns a divergent response shape: the assistant turn is
//! `message.content[]` content blocks and token usage lives under
//! `usage.tokens.{input,output}_tokens`. We translate both directions so the
//! gateway exposes a uniform OpenAI `ChatCompletionResponse`.

use crate::error::{GatewayError, GatewayResult};
use crate::openai::{ChatCompletionRequest, ChatCompletionResponse, Usage};
use crate::provider::{LlmProvider, ProviderConfig};
use async_trait::async_trait;

pub const DEFAULT_BASE_URL: &str = "https://api.cohere.com";

pub struct CohereProvider {
    config: ProviderConfig,
    client: reqwest::Client,
}

impl CohereProvider {
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

    /// Build the Cohere v2 `/v2/chat` request body. Cohere v2 accepts the
    /// OpenAI `messages` array verbatim (system/user/assistant roles), so we
    /// pass text content straight through and translate only the generation
    /// knobs that have different names.
    pub fn to_cohere_request(&self, req: &ChatCompletionRequest) -> serde_json::Value {
        let messages: Vec<serde_json::Value> = req
            .messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    crate::openai::Role::System => "system",
                    crate::openai::Role::Assistant => "assistant",
                    crate::openai::Role::Tool | crate::openai::Role::Function => "tool",
                    crate::openai::Role::User => "user",
                };
                serde_json::json!({
                    "role": role,
                    "content": m.content.as_text().unwrap_or(""),
                })
            })
            .collect();

        let mut body = serde_json::json!({
            "model": req.model,
            "messages": messages,
        });
        if let Some(t) = req.temperature {
            body["temperature"] = serde_json::json!(t);
        }
        if let Some(mt) = req.max_tokens {
            body["max_tokens"] = serde_json::json!(mt);
        }
        if let Some(p) = req.top_p {
            body["p"] = serde_json::json!(p);
        }
        body
    }

    /// Translate the Cohere v2 chat response into an OpenAI completion. Cohere
    /// returns the assistant turn as `message.content[]` blocks and token
    /// counts under `usage.tokens.{input,output}_tokens`; `finish_reason` uses
    /// SCREAMING-CASE values (`COMPLETE`, `MAX_TOKENS`, `ERROR`).
    pub fn from_cohere_response(
        &self,
        val: serde_json::Value,
        model: &str,
    ) -> GatewayResult<ChatCompletionResponse> {
        let text = val["message"]["content"]
            .as_array()
            .map(|blocks| {
                blocks
                    .iter()
                    .filter(|b| b["type"] == "text")
                    .filter_map(|b| b["text"].as_str())
                    .collect::<String>()
            })
            .unwrap_or_default();

        let input_tokens = val["usage"]["tokens"]["input_tokens"].as_u64().unwrap_or(0) as u32;
        let output_tokens = val["usage"]["tokens"]["output_tokens"]
            .as_u64()
            .unwrap_or(0) as u32;

        let mut resp =
            ChatCompletionResponse::simple(model, text, Usage::new(input_tokens, output_tokens));
        resp.choices[0].finish_reason = Some(map_finish_reason(
            val["finish_reason"].as_str().unwrap_or("COMPLETE"),
        ));
        Ok(resp)
    }
}

/// Map Cohere's SCREAMING-CASE finish reasons onto OpenAI's lowercase set.
fn map_finish_reason(cohere: &str) -> String {
    match cohere {
        "MAX_TOKENS" => "length",
        "TOOL_CALL" => "tool_calls",
        // COMPLETE / STOP_SEQUENCE / ERROR / anything else
        _ => "stop",
    }
    .to_string()
}

#[async_trait]
impl LlmProvider for CohereProvider {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn supported_models(&self) -> Vec<String> {
        vec![
            "command-r-plus".into(),
            "command-r".into(),
            "command-r7b".into(),
            "command".into(),
            "command-light".into(),
        ]
    }

    async fn complete(&self, req: &ChatCompletionRequest) -> GatewayResult<ChatCompletionResponse> {
        let url = format!("{}/v2/chat", self.base().trim_end_matches('/'));
        let api_key = self.config.api_key.as_deref().unwrap_or("");
        let body = self.to_cohere_request(req);
        let model = req.model.clone();

        let resp = self
            .client
            .post(&url)
            .bearer_auth(api_key)
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
        self.from_cohere_response(val, &model)
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
    use crate::openai::{ChatCompletionRequest, ChatMessage};
    use crate::provider::{ProviderConfig, ProviderType};

    fn cfg() -> ProviderConfig {
        ProviderConfig {
            name: "cohere-test".into(),
            provider_type: ProviderType::Cohere,
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
            model: "command-r-plus".into(),
            messages: vec![
                ChatMessage::system("You are helpful"),
                ChatMessage::user("Hi"),
            ],
            temperature: Some(0.4),
            top_p: None,
            max_tokens: Some(128),
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
    fn cohere_default_base_url_is_official_host() {
        assert_eq!(DEFAULT_BASE_URL, "https://api.cohere.com");
    }

    #[test]
    fn cohere_supported_models_includes_command_r_plus() {
        let p = CohereProvider::new(cfg());
        assert!(p
            .supported_models()
            .iter()
            .any(|m| m == "command-r-plus"));
    }

    #[test]
    fn cohere_request_carries_messages_temperature_and_max_tokens() {
        let p = CohereProvider::new(cfg());
        let body = p.to_cohere_request(&req());
        assert_eq!(body["model"], "command-r-plus");
        assert_eq!(body["messages"].as_array().unwrap().len(), 2);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["content"], "Hi");
        assert!((body["temperature"].as_f64().unwrap() - 0.4).abs() < 1e-6);
        assert_eq!(body["max_tokens"], 128);
    }

    #[test]
    fn cohere_response_extracts_text_and_token_usage() {
        let p = CohereProvider::new(cfg());
        let raw = serde_json::json!({
            "id": "c-123",
            "message": {
                "role": "assistant",
                "content": [{"type": "text", "text": "Hello there!"}]
            },
            "finish_reason": "COMPLETE",
            "usage": {"tokens": {"input_tokens": 5, "output_tokens": 2}}
        });
        let resp = p.from_cohere_response(raw, "command-r-plus").unwrap();
        assert_eq!(
            resp.choices[0]
                .message
                .as_ref()
                .unwrap()
                .content
                .as_text()
                .unwrap(),
            "Hello there!"
        );
        assert_eq!(resp.usage.prompt_tokens, 5);
        assert_eq!(resp.usage.completion_tokens, 2);
        assert_eq!(resp.usage.total_tokens, 7);
    }

    #[test]
    fn cohere_finish_reason_complete_maps_to_stop() {
        let p = CohereProvider::new(cfg());
        let raw = serde_json::json!({
            "message": {"role": "assistant", "content": [{"type": "text", "text": "x"}]},
            "finish_reason": "COMPLETE",
            "usage": {"tokens": {"input_tokens": 1, "output_tokens": 1}}
        });
        let resp = p.from_cohere_response(raw, "command-r").unwrap();
        assert_eq!(resp.choices[0].finish_reason.as_deref(), Some("stop"));
    }

    #[test]
    fn cohere_response_concatenates_multiple_text_blocks() {
        let p = CohereProvider::new(cfg());
        let raw = serde_json::json!({
            "message": {"role": "assistant", "content": [
                {"type": "text", "text": "foo "},
                {"type": "text", "text": "bar"}
            ]},
            "finish_reason": "COMPLETE",
            "usage": {"tokens": {"input_tokens": 1, "output_tokens": 1}}
        });
        let resp = p.from_cohere_response(raw, "command-r").unwrap();
        assert_eq!(
            resp.choices[0]
                .message
                .as_ref()
                .unwrap()
                .content
                .as_text()
                .unwrap(),
            "foo bar"
        );
    }
}
