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
