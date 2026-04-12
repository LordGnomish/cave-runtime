//! GatewayRouter — multi-provider routing with load balancing and fallback chains.

use crate::alias::AliasRegistry;
use crate::cache::{CacheConfig, PromptCache};
use crate::cost::CostTracker;
use crate::error::{GatewayError, GatewayResult};
use crate::guardrails::GuardrailEngine;
use crate::logging::{LogStatus, RequestLogger};
use crate::openai::{ChatCompletionRequest, ChatCompletionResponse};
use crate::provider::{LlmProvider, ProviderRegistry};
use crate::rate_limit::{RateLimit, RateLimiter};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;

// ── Routing policy ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingStrategy {
    /// Always use a specific provider
    Fixed { provider: String },
    /// Round-robin across providers
    RoundRobin { providers: Vec<String> },
    /// Weighted random selection
    Weighted { providers: Vec<(String, u32)> },
    /// Try providers in order until one succeeds
    Fallback { providers: Vec<String> },
}

// ── Gateway router ────────────────────────────────────────────────────────────

pub struct GatewayRouter {
    pub providers: Arc<ProviderRegistry>,
    pub aliases: Arc<AliasRegistry>,
    pub rate_limiter: Arc<RateLimiter>,
    pub cache: Arc<PromptCache>,
    pub cost_tracker: Arc<CostTracker>,
    pub logger: Arc<RequestLogger>,
    pub guardrails: Arc<GuardrailEngine>,
    strategy: RoutingStrategy,
    /// Rotating counter for round-robin
    rr_counter: std::sync::atomic::AtomicUsize,
}

impl GatewayRouter {
    pub fn new(
        providers: Arc<ProviderRegistry>,
        aliases: Arc<AliasRegistry>,
        strategy: RoutingStrategy,
    ) -> Self {
        Self {
            providers,
            aliases,
            strategy,
            rate_limiter: Arc::new(RateLimiter::new(RateLimit::default())),
            cache: Arc::new(PromptCache::new(CacheConfig::default())),
            cost_tracker: Arc::new(CostTracker::new()),
            logger: Arc::new(RequestLogger::new(10_000)),
            guardrails: Arc::new(GuardrailEngine::new()),
            rr_counter: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Route a request through the full pipeline:
    /// rate-limit → guardrail input check → cache → provider → guardrail output → log
    pub async fn complete(
        &self,
        consumer: &str,
        mut req: ChatCompletionRequest,
    ) -> GatewayResult<ChatCompletionResponse> {
        let start = Instant::now();

        // Resolve alias
        let (provider_name, model) = self.resolve_model(&req.model);
        req.model = model;

        // Rate limit (estimate token cost from messages length)
        let estimated_tokens: u32 = req.messages.iter()
            .filter_map(|m| m.content.as_text())
            .map(|t| crate::cost::estimate_tokens(t))
            .sum();

        if let Err(e) = self.rate_limiter.check(consumer, estimated_tokens) {
            self.logger.log_error(consumer, &provider_name, &req.model, 0, LogStatus::RateLimited, &e.to_string());
            return Err(e);
        }

        // Guardrail input check
        if let Err(e) = self.guardrails.check_input(&req) {
            self.logger.log_error(consumer, &provider_name, &req.model, 0, LogStatus::GuardrailBlocked, &e.to_string());
            return Err(e);
        }

        // Cache check
        if let Some(cached) = self.cache.get(&req) {
            let latency = start.elapsed().as_millis() as u64;
            self.logger.log_success(consumer, &provider_name, &req, &cached, latency, true);
            return Ok(cached);
        }

        // Route to provider(s)
        let resp = self.dispatch(consumer, &provider_name, &req).await?;

        // Guardrail output check
        if let Err(e) = self.guardrails.check_output(&resp) {
            let latency = start.elapsed().as_millis() as u64;
            self.logger.log_error(consumer, &provider_name, &req.model, latency, LogStatus::GuardrailBlocked, &e.to_string());
            return Err(e);
        }

        // Record cost
        self.cost_tracker.record(consumer, &req.model, &provider_name, resp.usage.prompt_tokens, resp.usage.completion_tokens);

        // Cache store
        self.cache.insert(&req, resp.clone());

        // Log
        let latency = start.elapsed().as_millis() as u64;
        self.logger.log_success(consumer, &provider_name, &req, &resp, latency, false);

        Ok(resp)
    }

    fn resolve_model(&self, model: &str) -> (String, String) {
        if let Some(alias) = self.aliases.resolve(model) {
            (alias.provider, alias.model)
        } else {
            // Try to infer provider from model name
            let provider = if model.starts_with("claude") {
                "anthropic"
            } else if model.starts_with("gpt") || model.starts_with("o1") || model.starts_with("o3") {
                "openai"
            } else {
                "local"
            };
            (provider.to_string(), model.to_string())
        }
    }

    async fn dispatch(&self, consumer: &str, preferred_provider: &str, req: &ChatCompletionRequest) -> GatewayResult<ChatCompletionResponse> {
        match &self.strategy {
            RoutingStrategy::Fixed { provider } => {
                let name = if preferred_provider == "local" || preferred_provider == "openai" || preferred_provider == "anthropic" {
                    preferred_provider
                } else {
                    provider.as_str()
                };
                self.call_provider(name, req).await
            }
            RoutingStrategy::Fallback { providers } => {
                let mut last_err = GatewayError::NoProvidersAvailable;
                for name in providers {
                    match self.call_provider(name, req).await {
                        Ok(resp) => return Ok(resp),
                        Err(e) => {
                            tracing::warn!("provider {} failed: {}", name, e);
                            last_err = e;
                        }
                    }
                }
                Err(last_err)
            }
            RoutingStrategy::RoundRobin { providers } => {
                if providers.is_empty() {
                    return Err(GatewayError::NoProvidersAvailable);
                }
                let idx = self.rr_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % providers.len();
                self.call_provider(&providers[idx], req).await
            }
            RoutingStrategy::Weighted { providers } => {
                if providers.is_empty() {
                    return Err(GatewayError::NoProvidersAvailable);
                }
                // Simple weighted selection using modulo on counter
                let total: u32 = providers.iter().map(|(_, w)| w).sum();
                let idx = self.rr_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed) as u32 % total;
                let mut cumulative = 0u32;
                let selected = providers.iter().find(|(_, w)| {
                    cumulative += w;
                    cumulative > idx
                }).map(|(name, _)| name.as_str()).unwrap_or(&providers[0].0);
                self.call_provider(selected, req).await
            }
        }
    }

    async fn call_provider(&self, name: &str, req: &ChatCompletionRequest) -> GatewayResult<ChatCompletionResponse> {
        let provider = self.providers.get(name)
            .ok_or_else(|| GatewayError::ProviderUnavailable { provider: name.to_string(), reason: "not registered".into() })?;
        provider.complete(req).await
    }

    pub fn provider_names(&self) -> Vec<String> {
        self.providers.list()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{MockProvider, ProviderRegistry};
    use crate::openai::ChatMessage;

    fn make_router() -> GatewayRouter {
        let registry = Arc::new(ProviderRegistry::new());
        registry.register(Arc::new(MockProvider::new("mock")));
        let aliases = Arc::new(AliasRegistry::new());
        GatewayRouter::new(registry, aliases, RoutingStrategy::Fixed { provider: "mock".into() })
    }

    fn make_req() -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "mock-model".into(),
            messages: vec![ChatMessage::user("hello")],
            temperature: None, top_p: None, max_tokens: None, stream: None,
            stop: None, presence_penalty: None, frequency_penalty: None,
            n: None, user: None, tools: None, tool_choice: None,
            response_format: None, seed: None, logprobs: None,
        }
    }

    #[tokio::test]
    async fn basic_routing_success() {
        let router = make_router();
        let resp = router.complete("user-1", make_req()).await.unwrap();
        assert!(!resp.choices.is_empty());
    }

    #[tokio::test]
    async fn cache_hit_on_second_request() {
        let router = make_router();
        // First call
        router.complete("user-1", make_req()).await.unwrap();
        // Second identical call should be a cache hit
        router.complete("user-1", make_req()).await.unwrap();
        let stats = router.cache.stats();
        // At least one hit
        assert!(stats.hits >= 1);
    }

    #[tokio::test]
    async fn fallback_strategy_tries_next_on_failure() {
        let registry = Arc::new(ProviderRegistry::new());
        // Register only "mock", not "broken"
        registry.register(Arc::new(MockProvider::new("mock")));
        let aliases = Arc::new(AliasRegistry::new());
        let router = GatewayRouter::new(
            registry,
            aliases,
            RoutingStrategy::Fallback { providers: vec!["broken".into(), "mock".into()] },
        );
        let resp = router.complete("user-1", make_req()).await.unwrap();
        assert!(!resp.choices.is_empty());
    }
}
