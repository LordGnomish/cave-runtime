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
