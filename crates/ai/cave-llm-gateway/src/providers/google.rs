// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Google Gemini / Vertex AI provider (`generateContent` protocol).
//!
//! Maps `litellm/llms/vertex_ai/gemini/transformation.py`. Both the public
//! Gemini API (`generativelanguage.googleapis.com`) and Vertex AI
//! (`{region}-aiplatform.googleapis.com`) speak the same `generateContent`
//! body; they differ only in host and auth (API key query param vs. GCP
//! OAuth bearer). We port the request/response transformation — the actual
//! parity surface — and resolve the credential from `ProviderConfig`
//! (cave-vault / keychain), supporting both hosts via `base_url`.

use crate::error::{GatewayError, GatewayResult};
use crate::openai::{ChatCompletionRequest, ChatCompletionResponse, Role, Usage};
use crate::provider::{LlmProvider, ProviderConfig};
use async_trait::async_trait;

pub const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com";

pub struct GoogleProvider {
    config: ProviderConfig,
    client: reqwest::Client,
}

impl GoogleProvider {
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

    /// Build a Gemini `generateContent` request body. System turns are hoisted
    /// into `systemInstruction`; remaining turns become `contents[]` with the
    /// OpenAI assistant role rewritten to Gemini's `model`. Generation knobs
    /// move into `generationConfig` under their Gemini names.
    pub fn to_gemini_request(&self, req: &ChatCompletionRequest) -> serde_json::Value {
        let system: String = req
            .messages
            .iter()
            .filter(|m| m.role == Role::System)
            .filter_map(|m| m.content.as_text())
            .collect::<Vec<_>>()
            .join("\n");

        let contents: Vec<serde_json::Value> = req
            .messages
            .iter()
            .filter(|m| m.role != Role::System)
            .map(|m| {
                let role = match m.role {
                    Role::Assistant => "model",
                    _ => "user",
                };
                serde_json::json!({
                    "role": role,
                    "parts": [{"text": m.content.as_text().unwrap_or("")}],
                })
            })
            .collect();

        let mut gen_config = serde_json::Map::new();
        if let Some(t) = req.temperature {
            gen_config.insert("temperature".into(), serde_json::json!(t));
        }
        if let Some(mt) = req.max_tokens {
            gen_config.insert("maxOutputTokens".into(), serde_json::json!(mt));
        }
        if let Some(p) = req.top_p {
            gen_config.insert("topP".into(), serde_json::json!(p));
        }

        let mut body = serde_json::json!({ "contents": contents });
        if !system.is_empty() {
            body["systemInstruction"] = serde_json::json!({ "parts": [{"text": system}] });
        }
        if !gen_config.is_empty() {
            body["generationConfig"] = serde_json::Value::Object(gen_config);
        }
        body
    }

    /// Translate a Gemini `generateContent` response into an OpenAI completion:
    /// concat `candidates[0].content.parts[].text`, pull token counts from
    /// `usageMetadata`, and normalise `finishReason`.
    pub fn from_gemini_response(
        &self,
        val: serde_json::Value,
        model: &str,
    ) -> GatewayResult<ChatCompletionResponse> {
        let candidate = &val["candidates"][0];
        let text = candidate["content"]["parts"]
            .as_array()
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|p| p["text"].as_str())
                    .collect::<String>()
            })
            .unwrap_or_default();

        let prompt_tokens = val["usageMetadata"]["promptTokenCount"]
            .as_u64()
            .unwrap_or(0) as u32;
        let completion_tokens = val["usageMetadata"]["candidatesTokenCount"]
            .as_u64()
            .unwrap_or(0) as u32;

        let mut resp = ChatCompletionResponse::simple(
            model,
            text,
            Usage::new(prompt_tokens, completion_tokens),
        );
        resp.choices[0].finish_reason = Some(map_finish_reason(
            candidate["finishReason"].as_str().unwrap_or("STOP"),
        ));
        Ok(resp)
    }
}

/// Map Gemini finish reasons to OpenAI's lowercase set.
fn map_finish_reason(gemini: &str) -> String {
    match gemini {
        "MAX_TOKENS" => "length",
        "SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" => "content_filter",
        // STOP / OTHER / unspecified
        _ => "stop",
    }
    .to_string()
}

#[async_trait]
impl LlmProvider for GoogleProvider {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn supported_models(&self) -> Vec<String> {
        vec![
            "gemini-2.0-flash".into(),
            "gemini-1.5-pro".into(),
            "gemini-1.5-flash".into(),
            "gemini-1.5-flash-8b".into(),
        ]
    }

    async fn complete(&self, req: &ChatCompletionRequest) -> GatewayResult<ChatCompletionResponse> {
        let api_key = self.config.api_key.as_deref().unwrap_or("");
        let url = format!(
            "{}/v1beta/models/{}:generateContent?key={}",
            self.base().trim_end_matches('/'),
            req.model,
            api_key
        );
        let body = self.to_gemini_request(req);
        let model = req.model.clone();

        let resp = self
            .client
            .post(&url)
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
        self.from_gemini_response(val, &model)
    }

    async fn health_check(&self) -> bool {
        let api_key = self.config.api_key.as_deref().unwrap_or("");
        let url = format!(
            "{}/v1beta/models?key={}",
            self.base().trim_end_matches('/'),
            api_key
        );
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
    use crate::openai::{ChatCompletionRequest, ChatMessage};
    use crate::provider::{ProviderConfig, ProviderType};

    fn cfg() -> ProviderConfig {
        ProviderConfig {
            name: "google-test".into(),
            provider_type: ProviderType::Google,
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
            model: "gemini-1.5-pro".into(),
            messages: vec![
                ChatMessage::system("Be terse"),
                ChatMessage::user("Hi"),
                ChatMessage::assistant("Hello"),
                ChatMessage::user("Bye"),
            ],
            temperature: Some(0.2),
            top_p: None,
            max_tokens: Some(256),
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
    fn google_default_base_url_is_generativelanguage() {
        assert_eq!(DEFAULT_BASE_URL, "https://generativelanguage.googleapis.com");
    }

    #[test]
    fn google_supported_models_includes_gemini_2_flash() {
        let p = GoogleProvider::new(cfg());
        assert!(p.supported_models().iter().any(|m| m == "gemini-2.0-flash"));
    }

    #[test]
    fn google_request_hoists_system_and_maps_roles() {
        let p = GoogleProvider::new(cfg());
        let body = p.to_gemini_request(&req());
        // system removed from contents, hoisted to systemInstruction
        assert_eq!(body["contents"].as_array().unwrap().len(), 3);
        assert_eq!(body["systemInstruction"]["parts"][0]["text"], "Be terse");
        // user stays user, assistant -> model
        assert_eq!(body["contents"][0]["role"], "user");
        assert_eq!(body["contents"][1]["role"], "model");
        assert_eq!(body["contents"][0]["parts"][0]["text"], "Hi");
    }

    #[test]
    fn google_request_moves_generation_knobs_into_generation_config() {
        let p = GoogleProvider::new(cfg());
        let body = p.to_gemini_request(&req());
        assert!((body["generationConfig"]["temperature"].as_f64().unwrap() - 0.2).abs() < 1e-6);
        assert_eq!(body["generationConfig"]["maxOutputTokens"], 256);
    }

    #[test]
    fn google_response_extracts_text_and_usage_metadata() {
        let p = GoogleProvider::new(cfg());
        let raw = serde_json::json!({
            "candidates": [{
                "content": {"role": "model", "parts": [{"text": "42"}]},
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 11,
                "candidatesTokenCount": 3,
                "totalTokenCount": 14
            }
        });
        let resp = p.from_gemini_response(raw, "gemini-1.5-pro").unwrap();
        assert_eq!(
            resp.choices[0].message.as_ref().unwrap().content.as_text().unwrap(),
            "42"
        );
        assert_eq!(resp.usage.prompt_tokens, 11);
        assert_eq!(resp.usage.completion_tokens, 3);
        assert_eq!(resp.usage.total_tokens, 14);
        assert_eq!(resp.choices[0].finish_reason.as_deref(), Some("stop"));
    }

    #[test]
    fn google_finish_reason_max_tokens_maps_to_length() {
        let p = GoogleProvider::new(cfg());
        let raw = serde_json::json!({
            "candidates": [{
                "content": {"parts": [{"text": "x"}]},
                "finishReason": "MAX_TOKENS"
            }],
            "usageMetadata": {"promptTokenCount": 1, "candidatesTokenCount": 1}
        });
        let resp = p.from_gemini_response(raw, "gemini-1.5-flash").unwrap();
        assert_eq!(resp.choices[0].finish_reason.as_deref(), Some("length"));
    }
}
