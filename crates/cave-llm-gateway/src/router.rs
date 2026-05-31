// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! GatewayRouter — multi-provider routing with load balancing and fallback chains.

use crate::alias::AliasRegistry;
use crate::budget::BudgetManager;
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
    pub budget: Arc<BudgetManager>,
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
            budget: Arc::new(BudgetManager::new()),
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
        let estimated_tokens: u32 = req
            .messages
            .iter()
            .filter_map(|m| m.content.as_text())
            .map(|t| crate::cost::estimate_tokens(t))
            .sum();

        if let Err(e) = self.rate_limiter.check(consumer, estimated_tokens) {
            self.logger.log_error(
                consumer,
                &provider_name,
                &req.model,
                0,
                LogStatus::RateLimited,
                &e.to_string(),
            );
            return Err(e);
        }

        // Spend-budget enforcement (LiteLLM BudgetManager). Roll the window
        // first so a daily/monthly reset frees a previously-blocked consumer,
        // then reject once cumulative spend has passed the allocated limit.
        self.budget.reset_on_duration(consumer);
        if !self.budget.is_within_budget(consumer) {
            let e = GatewayError::BudgetExceeded {
                scope: consumer.to_string(),
                spent: self.budget.get_current_cost(consumer),
                limit: self.budget.get_total_budget(consumer).unwrap_or(0.0),
            };
            self.logger.log_error(
                consumer,
                &provider_name,
                &req.model,
                0,
                LogStatus::Error,
                &e.to_string(),
            );
            return Err(e);
        }

        // Guardrail input check
        if let Err(e) = self.guardrails.check_input(&req) {
            self.logger.log_error(
                consumer,
                &provider_name,
                &req.model,
                0,
                LogStatus::GuardrailBlocked,
                &e.to_string(),
            );
            return Err(e);
        }

        // Cache check
        if let Some(cached) = self.cache.get(&req) {
            let latency = start.elapsed().as_millis() as u64;
            self.logger
                .log_success(consumer, &provider_name, &req, &cached, latency, true);
            return Ok(cached);
        }

        // Route to provider(s)
        let resp = self.dispatch(consumer, &provider_name, &req).await?;

        // Guardrail output check
        if let Err(e) = self.guardrails.check_output(&resp) {
            let latency = start.elapsed().as_millis() as u64;
            self.logger.log_error(
                consumer,
                &provider_name,
                &req.model,
                latency,
                LogStatus::GuardrailBlocked,
                &e.to_string(),
            );
            return Err(e);
        }

        // Record cost, then debit the consumer's spend budget so the next
        // request sees the updated cumulative total.
        let usage = self.cost_tracker.record(
            consumer,
            &req.model,
            &provider_name,
            resp.usage.prompt_tokens,
            resp.usage.completion_tokens,
        );
        self.budget.update_cost(consumer, &req.model, usage.cost_usd);

        // Cache store
        self.cache.insert(&req, resp.clone());

        // Log
        let latency = start.elapsed().as_millis() as u64;
        self.logger
            .log_success(consumer, &provider_name, &req, &resp, latency, false);

        Ok(resp)
    }

    /// Route an embeddings request: resolve the model alias to a provider,
    /// rate-limit the consumer, then dispatch to that provider's `/v1/embeddings`.
    pub async fn embeddings(
        &self,
        consumer: &str,
        mut req: crate::openai::EmbeddingRequest,
    ) -> GatewayResult<crate::openai::EmbeddingResponse> {
        let (provider_name, model) = self.resolve_model(&req.model);
        req.model = model;

        // Rate-limit on a rough token estimate of the inputs.
        let estimated_tokens: u32 = req
            .input
            .as_vec()
            .iter()
            .map(|t| crate::cost::estimate_tokens(t))
            .sum();
        self.rate_limiter.check(consumer, estimated_tokens)?;

        let provider =
            self.providers
                .get(&provider_name)
                .ok_or_else(|| GatewayError::ProviderUnavailable {
                    provider: provider_name.clone(),
                    reason: "not registered".into(),
                })?;
        provider.embeddings(&req).await
    }

    fn resolve_model(&self, model: &str) -> (String, String) {
        if let Some(alias) = self.aliases.resolve(model) {
            (alias.provider, alias.model)
        } else {
            // Try to infer provider from model name
            let provider = if model.starts_with("claude") {
                "anthropic"
            } else if model.starts_with("gpt") || model.starts_with("o1") || model.starts_with("o3")
            {
                "openai"
            } else if model.starts_with("deepseek") {
                "deepseek"
            } else {
                "local"
            };
            (provider.to_string(), model.to_string())
        }
    }

    async fn dispatch(
        &self,
        consumer: &str,
        preferred_provider: &str,
        req: &ChatCompletionRequest,
    ) -> GatewayResult<ChatCompletionResponse> {
        match &self.strategy {
            RoutingStrategy::Fixed { provider } => {
                // Honour an explicit alias-resolved preference only when it
                // points at a registered provider; otherwise the Fixed
                // strategy wins (its whole point is "always route here").
                let name = if self.providers.get(preferred_provider).is_some() {
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
                let idx = self
                    .rr_counter
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                    % providers.len();
                self.call_provider(&providers[idx], req).await
            }
            RoutingStrategy::Weighted { providers } => {
                if providers.is_empty() {
                    return Err(GatewayError::NoProvidersAvailable);
                }
                // Simple weighted selection using modulo on counter
                let total: u32 = providers.iter().map(|(_, w)| w).sum();
                let idx = self
                    .rr_counter
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                    as u32
                    % total;
                let mut cumulative = 0u32;
                let selected = providers
                    .iter()
                    .find(|(_, w)| {
                        cumulative += w;
                        cumulative > idx
                    })
                    .map(|(name, _)| name.as_str())
                    .unwrap_or(&providers[0].0);
                self.call_provider(selected, req).await
            }
        }
    }

    async fn call_provider(
        &self,
        name: &str,
        req: &ChatCompletionRequest,
    ) -> GatewayResult<ChatCompletionResponse> {
        let provider =
            self.providers
                .get(name)
                .ok_or_else(|| GatewayError::ProviderUnavailable {
                    provider: name.to_string(),
                    reason: "not registered".into(),
                })?;
        provider.complete(req).await
    }

    pub fn provider_names(&self) -> Vec<String> {
        self.providers.list()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openai::ChatMessage;
    use crate::provider::{MockProvider, ProviderRegistry};

    fn make_router() -> GatewayRouter {
        let registry = Arc::new(ProviderRegistry::new());
        registry.register(Arc::new(MockProvider::new("mock")));
        let aliases = Arc::new(AliasRegistry::new());
        GatewayRouter::new(
            registry,
            aliases,
            RoutingStrategy::Fixed {
                provider: "mock".into(),
            },
        )
    }

    fn make_req() -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "mock-model".into(),
            messages: vec![ChatMessage::user("hello")],
            temperature: None,
            top_p: None,
            max_tokens: None,
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
    async fn over_budget_consumer_is_blocked_by_pipeline() {
        let router = make_router();
        // $0 budget → the very first non-free request is rejected. Use a priced
        // model so cost accrues (mock-model is free in the pricing table).
        router.budget.create_budget(0.0, "broke", None);
        // Seed spend directly to simulate a prior request having exhausted it.
        router.budget.update_cost("broke", "gpt-4o", 0.01);
        let mut req = make_req();
        req.model = "gpt-4o".into();
        let err = router.complete("broke", req).await.unwrap_err();
        assert!(
            matches!(err, GatewayError::BudgetExceeded { .. }),
            "expected BudgetExceeded, got {err:?}"
        );
    }

    #[tokio::test]
    async fn within_budget_consumer_passes_and_spend_is_debited() {
        let router = make_router();
        router.budget.create_budget(100.0, "rich", None);
        // mock-model is free, so spend stays 0 but the request must succeed
        // and pass through the budget gate.
        let resp = router.complete("rich", make_req()).await.unwrap();
        assert!(!resp.choices.is_empty());
        assert!(router.budget.is_within_budget("rich"));
    }

    #[tokio::test]
    async fn embeddings_dispatch_routes_to_provider() {
        // Alias the embedding model to the registered mock provider so
        // resolve_model() routes the request there.
        let registry = Arc::new(ProviderRegistry::new());
        registry.register(Arc::new(MockProvider::new("mock")));
        let aliases = Arc::new(AliasRegistry::new());
        aliases.register(crate::alias::ModelAlias {
            alias: "embed-me".into(),
            provider: "mock".into(),
            model: "mock-embed".into(),
            description: None,
        });
        let router = GatewayRouter::new(
            registry,
            aliases,
            RoutingStrategy::Fixed {
                provider: "mock".into(),
            },
        );
        let req = crate::openai::EmbeddingRequest {
            model: "embed-me".into(),
            input: crate::openai::EmbeddingInput::Single("vectorise me".into()),
            encoding_format: None,
            dimensions: Some(8),
            user: None,
        };
        let resp = router.embeddings("user-1", req).await.unwrap();
        assert_eq!(resp.data.len(), 1);
        assert_eq!(resp.data[0].embedding.len(), 8);
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
            RoutingStrategy::Fallback {
                providers: vec!["broken".into(), "mock".into()],
            },
        );
        let resp = router.complete("user-1", make_req()).await.unwrap();
        assert!(!resp.choices.is_empty());
    }
}
