// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Replicate provider (async predictions API).
//!
//! Maps `litellm/llms/replicate/`. Replicate is not request/response in one
//! shot: a `POST /v1/predictions` creates a prediction, then the client polls
//! `urls.get` until `status` reaches a terminal value. The LLM `output` is an
//! array of token fragments that must be concatenated. We port the prompt
//! flattening, request building, status classification and output decoding as
//! pure functions, and drive the poll loop in `complete`.

use crate::error::{GatewayError, GatewayResult};
use crate::openai::{ChatCompletionRequest, ChatCompletionResponse, Usage};
use crate::provider::{LlmProvider, ProviderConfig};
use async_trait::async_trait;

pub const DEFAULT_BASE_URL: &str = "https://api.replicate.com";

pub struct ReplicateProvider {
    config: ProviderConfig,
    client: reqwest::Client,
}

/// Flatten an OpenAI chat transcript into a single role-tagged prompt — the
/// shape Replicate language models expect on the `prompt` input field.
pub fn flatten_prompt(req: &ChatCompletionRequest) -> String {
    let mut out = String::new();
    for m in &req.messages {
        let tag = match m.role {
            crate::openai::Role::System => "System",
            crate::openai::Role::Assistant => "Assistant",
            crate::openai::Role::Tool | crate::openai::Role::Function => "Tool",
            crate::openai::Role::User => "User",
        };
        out.push_str(tag);
        out.push_str(": ");
        out.push_str(m.content.as_text().unwrap_or(""));
        out.push('\n');
    }
    out.push_str("Assistant: ");
    out
}

/// Terminal prediction states — once reached, polling stops.
pub fn is_terminal(status: &str) -> bool {
    matches!(status, "succeeded" | "failed" | "canceled")
}

/// Concatenate the Replicate `output` field, which is either an array of token
/// fragments or a single string.
pub fn decode_output(val: &serde_json::Value) -> String {
    match &val["output"] {
        serde_json::Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p.as_str())
            .collect::<String>(),
        serde_json::Value::String(s) => s.clone(),
        _ => String::new(),
    }
}

impl ReplicateProvider {
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

    /// Build the `POST /v1/predictions` body. A `model` containing a colon is
    /// treated as a pinned `version`; otherwise it is sent as a model slug via
    /// the model-scoped predictions endpoint (handled in `complete`).
    pub fn to_replicate_request(&self, req: &ChatCompletionRequest) -> serde_json::Value {
        let mut input = serde_json::json!({ "prompt": flatten_prompt(req) });
        if let Some(mt) = req.max_tokens {
            input["max_new_tokens"] = serde_json::json!(mt);
        }
        if let Some(t) = req.temperature {
            input["temperature"] = serde_json::json!(t);
        }
        if let Some(p) = req.top_p {
            input["top_p"] = serde_json::json!(p);
        }

        if let Some((_, version)) = req.model.split_once(':') {
            serde_json::json!({ "version": version, "input": input })
        } else {
            serde_json::json!({ "input": input })
        }
    }

    fn create_url(&self, model: &str) -> String {
        let base = self.base().trim_end_matches('/');
        if model.contains(':') {
            format!("{base}/v1/predictions")
        } else {
            format!("{base}/v1/models/{model}/predictions")
        }
    }

    /// Turn a terminal prediction into an OpenAI completion. `usage` is
    /// estimated from word counts because Replicate does not report tokens.
    pub fn from_prediction(
        &self,
        val: &serde_json::Value,
        req: &ChatCompletionRequest,
    ) -> GatewayResult<ChatCompletionResponse> {
        if val["status"] == "failed" {
            return Err(GatewayError::UpstreamError {
                status: 502,
                body: val["error"].as_str().unwrap_or("prediction failed").into(),
            });
        }
        let text = decode_output(val);
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
impl LlmProvider for ReplicateProvider {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn supported_models(&self) -> Vec<String> {
        vec![
            "meta/meta-llama-3.1-405b-instruct".into(),
            "meta/meta-llama-3-70b-instruct".into(),
            "mistralai/mistral-7b-instruct-v0.2".into(),
        ]
    }

    async fn complete(&self, req: &ChatCompletionRequest) -> GatewayResult<ChatCompletionResponse> {
        let url = self.create_url(&req.model);
        let token = self.config.api_key.as_deref().unwrap_or("");
        let body = self.to_replicate_request(req);

        let resp = self
            .client
            .post(&url)
            .bearer_auth(token)
            .header("Prefer", "wait")
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
        let mut val: serde_json::Value =
            resp.json()
                .await
                .map_err(|e| GatewayError::ProviderUnavailable {
                    provider: self.config.name.clone(),
                    reason: format!("deserialize: {e}"),
                })?;

        // Poll until terminal (the `Prefer: wait` header usually returns a
        // completed prediction first try, but fall back to polling urls.get).
        let mut polls = 0;
        while !is_terminal(val["status"].as_str().unwrap_or("starting")) && polls < 60 {
            let get_url = match val["urls"]["get"].as_str() {
                Some(u) => u.to_string(),
                None => break,
            };
            let poll = self
                .client
                .get(&get_url)
                .bearer_auth(token)
                .send()
                .await
                .map_err(|e| GatewayError::ProviderUnavailable {
                    provider: self.config.name.clone(),
                    reason: e.to_string(),
                })?;
            val = poll
                .json()
                .await
                .map_err(|e| GatewayError::ProviderUnavailable {
                    provider: self.config.name.clone(),
                    reason: format!("poll deserialize: {e}"),
                })?;
            polls += 1;
        }
        self.from_prediction(&val, req)
    }

    async fn health_check(&self) -> bool {
        let url = format!("{}/v1/models", self.base().trim_end_matches('/'));
        let token = self.config.api_key.as_deref().unwrap_or("");
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
            name: "replicate-test".into(),
            provider_type: ProviderType::Replicate,
            base_url: "".into(),
            api_key: Some("fake".into()),
            timeout_secs: 5,
            max_retries: 0,
            weight: 1,
            enabled: true,
        }
    }

    fn req(model: &str) -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: model.into(),
            messages: vec![
                ChatMessage::system("Be brief"),
                ChatMessage::user("hello world"),
            ],
            temperature: Some(0.7),
            top_p: None,
            max_tokens: Some(64),
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
    fn replicate_default_base_url() {
        assert_eq!(DEFAULT_BASE_URL, "https://api.replicate.com");
    }

    #[test]
    fn flatten_prompt_tags_roles_and_ends_with_assistant_cue() {
        let p = flatten_prompt(&req("meta/meta-llama-3-70b-instruct"));
        assert!(p.contains("System: Be brief"));
        assert!(p.contains("User: hello world"));
        assert!(p.trim_end().ends_with("Assistant:"));
    }

    #[test]
    fn request_with_version_model_sends_version_field() {
        let pr = ReplicateProvider::new(cfg());
        let body = pr.to_replicate_request(&req("owner/model:abc123"));
        assert_eq!(body["version"], "abc123");
        assert_eq!(body["input"]["max_new_tokens"], 64);
        assert!(body["input"]["prompt"].as_str().unwrap().contains("hello world"));
    }

    #[test]
    fn request_with_slug_model_omits_version() {
        let pr = ReplicateProvider::new(cfg());
        let body = pr.to_replicate_request(&req("meta/meta-llama-3-70b-instruct"));
        assert!(body.get("version").is_none());
        assert!(body["input"].is_object());
    }

    #[test]
    fn create_url_picks_model_scoped_for_slug() {
        let pr = ReplicateProvider::new(cfg());
        assert_eq!(
            pr.create_url("meta/llama"),
            "https://api.replicate.com/v1/models/meta/llama/predictions"
        );
        assert_eq!(
            pr.create_url("owner/m:ver"),
            "https://api.replicate.com/v1/predictions"
        );
    }

    #[test]
    fn is_terminal_classifies_states() {
        assert!(is_terminal("succeeded"));
        assert!(is_terminal("failed"));
        assert!(is_terminal("canceled"));
        assert!(!is_terminal("starting"));
        assert!(!is_terminal("processing"));
    }

    #[test]
    fn decode_output_joins_array_fragments() {
        let v = serde_json::json!({"output": ["Hel", "lo ", "wld"]});
        assert_eq!(decode_output(&v), "Hello wld");
    }

    #[test]
    fn decode_output_passes_through_string() {
        let v = serde_json::json!({"output": "single string"});
        assert_eq!(decode_output(&v), "single string");
    }

    #[test]
    fn from_prediction_succeeded_builds_completion() {
        let pr = ReplicateProvider::new(cfg());
        let v = serde_json::json!({
            "status": "succeeded",
            "output": ["four ", "words ", "here ", "now"]
        });
        let resp = pr.from_prediction(&v, &req("meta/llama")).unwrap();
        assert_eq!(
            resp.choices[0].message.as_ref().unwrap().content.as_text().unwrap(),
            "four words here now"
        );
        assert_eq!(resp.usage.completion_tokens, 4);
    }

    #[test]
    fn from_prediction_failed_is_error() {
        let pr = ReplicateProvider::new(cfg());
        let v = serde_json::json!({"status": "failed", "error": "OOM"});
        assert!(pr.from_prediction(&v, &req("meta/llama")).is_err());
    }
}
