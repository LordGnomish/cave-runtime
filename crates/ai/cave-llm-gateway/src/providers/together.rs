// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Together AI + Fireworks AI providers (OpenAI-compatible).
//!
//! Maps `litellm/llms/together_ai/` and `litellm/llms/fireworks_ai/`. Both
//! vendors expose an OpenAI-compatible `/v1/chat/completions` surface, so a
//! single adapter — parameterised by vendor (host + catalogue) — covers both.
//! Request/response bodies pass through unchanged; only the base URL and the
//! bearer key differ.

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
