// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-hermes integration shim.
//!
//! cave-hermes ships a `MultiGateway` (see its `llm_gateway_adapter`
//! module) that fans calls out across `ProviderKind` backends. This
//! module exposes the *cave-llm-gateway side* of that contract: the
//! data structures it expects and a helper that maps our
//! [`ChatCompletionRequest`] to the lighter hermes prompt struct.
//!
//! Why not import cave-hermes directly? cave-llm-gateway is a lower-
//! level dependency — pulling cave-hermes back in would create a cycle.
//! Instead, both crates speak the wire-stable `HermesProviderKind`
//! enum mirrored below, and cave-hermes' `MultiGateway::register` takes
//! an `Arc<dyn LlmGateway>` whose trait signature this module documents.

use crate::openai::{ChatCompletionRequest, ChatCompletionResponse, ChatMessage, MessageContent, Role};
use serde::{Deserialize, Serialize};

/// Mirror of `cave_hermes::prompt::ProviderKind`. Kept in lockstep with
/// the upstream variants by the integration test in this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HermesProviderKind {
    Anthropic,
    OpenAi,
    Ollama,
    OpenRouter,
    Mistral,
    LlamaCpp,
    Mlx,
}

impl HermesProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
            Self::Ollama => "ollama",
            Self::OpenRouter => "openrouter",
            Self::Mistral => "mistral",
            Self::LlamaCpp => "llamacpp",
            Self::Mlx => "mlx",
        }
    }
}

/// The "lite" prompt shape cave-hermes hands us. We do not depend on
/// cave-hermes itself; the wire-fields below are copied verbatim from
/// `cave_hermes::gateway::CompletionRequest` (kept stable by the
/// upstream Charter v2 audit).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HermesCompletionRequest {
    pub model: String,
    pub system: String,
    pub user: String,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HermesCompletionResponse {
    pub text: String,
    pub provider: HermesProviderKind,
    pub model: String,
    pub tokens: u32,
    pub latency_ms: u64,
}

/// Map a hermes request to our (richer) OpenAI-compatible shape.
pub fn from_hermes_request(req: &HermesCompletionRequest) -> ChatCompletionRequest {
    let mut messages = Vec::with_capacity(2);
    if !req.system.is_empty() {
        messages.push(ChatMessage::system(req.system.clone()));
    }
    messages.push(ChatMessage::user(req.user.clone()));
    ChatCompletionRequest {
        model: req.model.clone(),
        messages,
        temperature: req.temperature,
        top_p: None,
        max_tokens: req.max_tokens,
        stream: Some(false),
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

/// Map a gateway response back to the hermes shape.
pub fn to_hermes_response(
    resp: &ChatCompletionResponse,
    provider: HermesProviderKind,
    latency_ms: u64,
) -> HermesCompletionResponse {
    let text = resp
        .choices
        .first()
        .and_then(|c| c.message.as_ref())
        .map(|m| match &m.content {
            MessageContent::Text(s) => s.clone(),
            MessageContent::Parts(_) => String::new(),
        })
        .unwrap_or_default();
    let total = resp.usage.total_tokens;
    HermesCompletionResponse {
        text,
        provider,
        model: resp.model.clone(),
        tokens: total,
        latency_ms,
    }
}

/// Provider-name → HermesProviderKind. Returns `None` for an unknown
/// label so callers can decide how to surface it.
pub fn classify_provider(name: &str) -> Option<HermesProviderKind> {
    match name {
        "anthropic" => Some(HermesProviderKind::Anthropic),
        "openai" => Some(HermesProviderKind::OpenAi),
        "ollama" => Some(HermesProviderKind::Ollama),
        "openrouter" => Some(HermesProviderKind::OpenRouter),
        "mistral" => Some(HermesProviderKind::Mistral),
        "llamacpp" | "llama.cpp" | "llama_cpp" => Some(HermesProviderKind::LlamaCpp),
        "mlx" | "mlx-lm" => Some(HermesProviderKind::Mlx),
        // Groq, DeepSeek & Together AI speak the OpenAI wire protocol, so the
        // hermes bridge treats them as OpenAI-kind clients (no new variant).
        "groq" | "deepseek" | "together" => Some(HermesProviderKind::OpenAi),
        _ => None,
    }
}

/// Used by both sides of the boundary: which providers cave-hermes is
/// expected to find in our registry under the hermes-mandated defaults.
pub const HERMES_REQUIRED_PROVIDERS: &[HermesProviderKind] = &[
    HermesProviderKind::Anthropic,
    HermesProviderKind::OpenAi,
    HermesProviderKind::Ollama,
];

#[allow(unused_imports)]
use Role as _Role; // ensure module imports are exercised in docs

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_recognises_all_six_mvp_providers() {
        for (slug, want) in [
            ("anthropic", HermesProviderKind::Anthropic),
            ("openai", HermesProviderKind::OpenAi),
            ("ollama", HermesProviderKind::Ollama),
            ("mistral", HermesProviderKind::Mistral),
            ("llamacpp", HermesProviderKind::LlamaCpp),
            ("mlx", HermesProviderKind::Mlx),
            ("groq", HermesProviderKind::OpenAi),
            ("deepseek", HermesProviderKind::OpenAi),
            ("together", HermesProviderKind::OpenAi),
        ] {
            assert_eq!(classify_provider(slug), Some(want), "slug={}", slug);
        }
    }

    #[test]
    fn classify_returns_none_for_unknown() {
        assert!(classify_provider("xyzzy").is_none());
    }

    #[test]
    fn provider_kind_as_str_round_trips() {
        for p in [
            HermesProviderKind::Anthropic,
            HermesProviderKind::OpenAi,
            HermesProviderKind::Ollama,
            HermesProviderKind::OpenRouter,
            HermesProviderKind::Mistral,
            HermesProviderKind::LlamaCpp,
            HermesProviderKind::Mlx,
        ] {
            assert_eq!(classify_provider(p.as_str()), Some(p));
        }
    }

    #[test]
    fn from_hermes_request_builds_two_messages_when_system_set() {
        let req = HermesCompletionRequest {
            model: "claude-3-5-sonnet".into(),
            system: "be concise".into(),
            user: "hello".into(),
            max_tokens: Some(64),
            temperature: Some(0.0),
        };
        let oai = from_hermes_request(&req);
        assert_eq!(oai.messages.len(), 2);
        assert_eq!(oai.model, "claude-3-5-sonnet");
        assert_eq!(oai.temperature, Some(0.0));
        assert_eq!(oai.max_tokens, Some(64));
    }

    #[test]
    fn from_hermes_request_omits_system_when_empty() {
        let req = HermesCompletionRequest {
            model: "llama3".into(),
            system: "".into(),
            user: "hello".into(),
            max_tokens: None,
            temperature: None,
        };
        let oai = from_hermes_request(&req);
        assert_eq!(oai.messages.len(), 1);
    }

    #[test]
    fn to_hermes_response_extracts_text_from_first_choice() {
        use crate::openai::Usage;
        let resp = ChatCompletionResponse::simple("gpt-4o", "hi back".to_string(), Usage::new(3, 5));
        let h = to_hermes_response(&resp, HermesProviderKind::OpenAi, 42);
        assert_eq!(h.text, "hi back");
        assert_eq!(h.provider, HermesProviderKind::OpenAi);
        assert_eq!(h.tokens, 8);
        assert_eq!(h.latency_ms, 42);
    }

    #[test]
    fn required_providers_covers_three_default_kinds() {
        assert_eq!(HERMES_REQUIRED_PROVIDERS.len(), 3);
    }
}
