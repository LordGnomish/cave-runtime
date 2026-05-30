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
