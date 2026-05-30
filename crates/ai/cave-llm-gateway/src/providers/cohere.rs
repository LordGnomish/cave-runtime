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
        assert_eq!(body["temperature"], 0.4);
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
