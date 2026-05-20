// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-llm-gateway wire — a `MultiGateway` that holds one concrete
//! [`LlmGateway`] per [`ProviderKind`] and routes incoming
//! [`CompletionRequest`]s to the right backend.
//!
//! In production the underlying gateways come from cave-llm-gateway (one
//! HTTP transport per provider, configured from cave-vault secrets). For
//! this crate's MVP we register the gateways cave-hermes already ships
//! (`OllamaGateway`, `AnthropicStubGateway`) plus an injectable
//! [`InMemoryGateway`] for tests / dry-runs.

use crate::error::HermesError;
use crate::gateway::{CompletionRequest, CompletionResponse, LlmGateway};
use crate::prompt::ProviderKind;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

/// One gateway per provider. `complete` looks up the gateway by
/// `req.model` prefix (e.g. `claude-` → Anthropic, `gpt-` → OpenAI,
/// `llama` / `mistral` → Ollama) and dispatches.
pub struct MultiGateway {
    backends: HashMap<ProviderKind, Arc<dyn LlmGateway>>,
    routing: ModelRouting,
}

#[derive(Debug, Clone)]
pub struct ModelRouting {
    rules: Vec<ModelRule>,
    fallback: Option<ProviderKind>,
}

#[derive(Debug, Clone)]
struct ModelRule {
    prefix: String,
    provider: ProviderKind,
}

impl Default for ModelRouting {
    fn default() -> Self {
        Self {
            rules: vec![
                ModelRule {
                    prefix: "claude".into(),
                    provider: ProviderKind::Anthropic,
                },
                ModelRule {
                    prefix: "gpt".into(),
                    provider: ProviderKind::OpenAi,
                },
                ModelRule {
                    prefix: "o1".into(),
                    provider: ProviderKind::OpenAi,
                },
                ModelRule {
                    prefix: "llama".into(),
                    provider: ProviderKind::Ollama,
                },
                ModelRule {
                    prefix: "mistral".into(),
                    provider: ProviderKind::Ollama,
                },
                ModelRule {
                    prefix: "qwen".into(),
                    provider: ProviderKind::Ollama,
                },
                ModelRule {
                    prefix: "openrouter/".into(),
                    provider: ProviderKind::OpenRouter,
                },
            ],
            fallback: Some(ProviderKind::Ollama),
        }
    }
}

impl ModelRouting {
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            fallback: None,
        }
    }

    pub fn with_rule(mut self, prefix: impl Into<String>, provider: ProviderKind) -> Self {
        self.rules.push(ModelRule {
            prefix: prefix.into(),
            provider,
        });
        self
    }

    pub fn with_fallback(mut self, provider: ProviderKind) -> Self {
        self.fallback = Some(provider);
        self
    }

    /// Resolve a model name to a provider. Prefix match (longest wins) then
    /// fallback. Returns `None` if no rule matches and no fallback set.
    pub fn route(&self, model: &str) -> Option<ProviderKind> {
        let lowered = model.to_ascii_lowercase();
        let mut best: Option<(&ModelRule, usize)> = None;
        for rule in &self.rules {
            if lowered.starts_with(&rule.prefix.to_ascii_lowercase()) {
                let len = rule.prefix.len();
                if best.map(|(_, l)| len > l).unwrap_or(true) {
                    best = Some((rule, len));
                }
            }
        }
        best.map(|(r, _)| r.provider).or(self.fallback)
    }
}

impl MultiGateway {
    pub fn new(routing: ModelRouting) -> Self {
        Self {
            backends: HashMap::new(),
            routing,
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(ModelRouting::default())
    }

    /// Register (or replace) the gateway for `provider`.
    pub fn register(&mut self, provider: ProviderKind, gw: Arc<dyn LlmGateway>) {
        self.backends.insert(provider, gw);
    }

    pub fn registered_providers(&self) -> Vec<ProviderKind> {
        let mut v: Vec<_> = self.backends.keys().copied().collect();
        v.sort_by_key(|p| p.as_str().to_string());
        v
    }

    pub fn route(&self, model: &str) -> Option<ProviderKind> {
        self.routing.route(model)
    }

    fn gateway_for(&self, provider: ProviderKind) -> Result<&Arc<dyn LlmGateway>, HermesError> {
        self.backends.get(&provider).ok_or_else(|| {
            HermesError::PlannerRejected(format!(
                "no gateway registered for provider {:?}",
                provider
            ))
        })
    }
}

#[async_trait]
impl LlmGateway for MultiGateway {
    fn kind(&self) -> ProviderKind {
        // The "kind" of a multi-gateway is meaningless. Pick the first
        // registered provider so callers checking `.kind()` see something
        // stable.
        let mut providers: Vec<_> = self.backends.keys().copied().collect();
        providers.sort_by_key(|p| p.as_str().to_string());
        providers.into_iter().next().unwrap_or(ProviderKind::Ollama)
    }

    async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse, HermesError> {
        let provider = self.routing.route(&req.model).ok_or_else(|| {
            HermesError::PlannerRejected(format!("no route for model `{}`", req.model))
        })?;
        let gw = self.gateway_for(provider)?;
        gw.complete(req).await
    }
}

/// Test-only gateway that returns canned responses keyed by model name.
/// Useful for wiring tests without spinning up a real HTTP backend.
pub struct InMemoryGateway {
    responses: HashMap<String, String>,
    kind: ProviderKind,
}

impl InMemoryGateway {
    pub fn new(kind: ProviderKind) -> Self {
        Self {
            responses: HashMap::new(),
            kind,
        }
    }

    pub fn with_response(mut self, model: impl Into<String>, text: impl Into<String>) -> Self {
        self.responses.insert(model.into(), text.into());
        self
    }
}

#[async_trait]
impl LlmGateway for InMemoryGateway {
    fn kind(&self) -> ProviderKind {
        self.kind
    }
    async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse, HermesError> {
        let text = self
            .responses
            .get(&req.model)
            .cloned()
            .unwrap_or_else(|| format!("[{}] {}", req.model, req.user));
        Ok(CompletionResponse {
            text: text.clone(),
            provider: self.kind,
            model: req.model.clone(),
            tokens: (text.chars().count() / 4) as u32,
            latency_ms: 1,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn routing_matches_claude_prefix() {
        let r = ModelRouting::default();
        assert_eq!(r.route("claude-3-7-sonnet"), Some(ProviderKind::Anthropic));
    }

    #[tokio::test]
    async fn routing_matches_gpt_prefix() {
        let r = ModelRouting::default();
        assert_eq!(r.route("gpt-4o-mini"), Some(ProviderKind::OpenAi));
    }

    #[tokio::test]
    async fn routing_matches_ollama_models() {
        let r = ModelRouting::default();
        assert_eq!(r.route("llama3.1"), Some(ProviderKind::Ollama));
        assert_eq!(r.route("mistral-7b"), Some(ProviderKind::Ollama));
        assert_eq!(r.route("qwen2.5"), Some(ProviderKind::Ollama));
    }

    #[tokio::test]
    async fn routing_falls_back_when_no_rule_matches() {
        let r = ModelRouting::default();
        assert_eq!(r.route("unknown-model"), Some(ProviderKind::Ollama));
    }

    #[tokio::test]
    async fn routing_returns_none_when_no_match_and_no_fallback() {
        let r = ModelRouting::new();
        assert!(r.route("anything").is_none());
    }

    #[tokio::test]
    async fn routing_custom_rule_priority() {
        let r = ModelRouting::new()
            .with_rule("custom-", ProviderKind::OpenAi)
            .with_fallback(ProviderKind::Ollama);
        assert_eq!(r.route("custom-x"), Some(ProviderKind::OpenAi));
        assert_eq!(r.route("other"), Some(ProviderKind::Ollama));
    }

    #[tokio::test]
    async fn multi_gateway_dispatches_by_route() {
        let mut g = MultiGateway::with_defaults();
        let ollama = Arc::new(InMemoryGateway::new(ProviderKind::Ollama).with_response("llama3", "ok-llama"));
        let openai = Arc::new(InMemoryGateway::new(ProviderKind::OpenAi).with_response("gpt-4", "ok-openai"));
        g.register(ProviderKind::Ollama, ollama);
        g.register(ProviderKind::OpenAi, openai);
        let r1 = g
            .complete(&CompletionRequest::new("llama3", "s", "u"))
            .await
            .unwrap();
        let r2 = g
            .complete(&CompletionRequest::new("gpt-4", "s", "u"))
            .await
            .unwrap();
        assert_eq!(r1.text, "ok-llama");
        assert_eq!(r2.text, "ok-openai");
    }

    #[tokio::test]
    async fn multi_gateway_errors_for_unregistered_provider() {
        let g = MultiGateway::with_defaults();
        let err = g
            .complete(&CompletionRequest::new("gpt-4", "s", "u"))
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("no gateway"));
    }

    #[tokio::test]
    async fn multi_gateway_errors_for_unroutable_model() {
        let routing = ModelRouting::new();
        let mut g = MultiGateway::new(routing);
        let ollama = Arc::new(InMemoryGateway::new(ProviderKind::Ollama));
        g.register(ProviderKind::Ollama, ollama);
        let err = g
            .complete(&CompletionRequest::new("xyz", "s", "u"))
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("no route"));
    }

    #[tokio::test]
    async fn registered_providers_listed_sorted() {
        let mut g = MultiGateway::with_defaults();
        g.register(
            ProviderKind::Ollama,
            Arc::new(InMemoryGateway::new(ProviderKind::Ollama)),
        );
        g.register(
            ProviderKind::Anthropic,
            Arc::new(InMemoryGateway::new(ProviderKind::Anthropic)),
        );
        let list = g.registered_providers();
        assert_eq!(list, vec![ProviderKind::Anthropic, ProviderKind::Ollama]);
    }

    #[tokio::test]
    async fn in_memory_gateway_default_echoes() {
        let g = InMemoryGateway::new(ProviderKind::OpenAi);
        let resp = g
            .complete(&CompletionRequest::new("gpt-x", "sys", "hello"))
            .await
            .unwrap();
        assert!(resp.text.contains("hello"));
        assert_eq!(resp.provider, ProviderKind::OpenAi);
    }
}
