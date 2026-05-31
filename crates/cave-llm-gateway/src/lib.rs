// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CAVE LLM Gateway — Multi-provider LLM router and proxy.
//!
//! Compatible with: LiteLLM, custom AI gateway, OpenAI proxy.
//!
//! Features:
//! - Provider trait + concrete backends: Ollama (native API), llama.cpp,
//!   MLX-LM, Anthropic, OpenAI, Mistral La Plateforme.
//! - Keychain-resolved SaaS keys (macOS `security`, env-var fallback).
//! - Capability-based router (context window, tools, vision, locality, cost).
//! - Cost ledger + response cache + exponential-backoff retry.
//! - Prometheus exposition compatible with cave-metrics scrape config.
//! - Aggregate health check across all registered providers.
//! - cave-hermes bridge (MultiGateway adapter) + cave-llm-tracker bench wire.

pub mod alias;
pub mod api_keys;
pub mod bench_wire;
pub mod budget;
pub mod cache;
pub mod capability;
pub mod cost;
pub mod error;
pub mod guardrails;
pub mod health;
pub mod hermes_bridge;
pub mod keychain;
pub mod logging;
pub mod metrics;
pub mod openai;
pub mod provider;
pub mod providers;
pub mod rate_limit;
pub mod retry;
pub mod router;
pub mod routes;
pub mod streaming;

use axum::Router;
use std::sync::Arc;

pub use bench_wire::{BenchOutcome, BenchPrompt, BenchSummary, BenchTarget, BENCH_PROMPT_IDS};
pub use capability::{
    seed_catalogue, seeded_router, CapabilityRequest, CapabilityRouter, Locality, ModelCapability,
};
pub use error::{GatewayError, GatewayResult};
pub use health::{check_all as check_health, HealthReport, HealthState, ProviderHealth};
pub use hermes_bridge::{
    classify_provider as classify_hermes_provider, from_hermes_request, to_hermes_response,
    HermesCompletionRequest, HermesCompletionResponse, HermesProviderKind,
    HERMES_REQUIRED_PROVIDERS,
};
pub use keychain::{resolve as resolve_api_key, KeySource, KeychainProvider};
pub use metrics::{global as global_metrics, GatewayMetrics, ProviderStats};
pub use retry::{with_retry, RetryPolicy};
pub use router::GatewayRouter;

/// Crate identity. Charter v2 gate (`gate_1_upstream_version_pinned`)
/// asserts this string and `parity.manifest.toml`'s `[upstream] version`
/// stay in lockstep.
pub const MODULE_NAME: &str = "llm-gateway";
pub const UPSTREAM_VERSION: &str = "v1.85.1";

/// Shared state for the LLM gateway.
pub struct GatewayState {
    pub router: Arc<GatewayRouter>,
    pub metrics: Arc<GatewayMetrics>,
}

impl GatewayState {
    pub fn new(router: GatewayRouter) -> Self {
        Self {
            router: Arc::new(router),
            metrics: Arc::new(GatewayMetrics::new()),
        }
    }
}

impl Default for GatewayState {
    fn default() -> Self {
        let providers = Arc::new(provider::ProviderRegistry::new());
        let aliases = Arc::new(alias::AliasRegistry::new());
        let strategy = router::RoutingStrategy::Fallback { providers: vec![] };
        Self {
            router: Arc::new(GatewayRouter::new(providers, aliases, strategy)),
            metrics: Arc::new(GatewayMetrics::new()),
        }
    }
}

/// Build Axum router exposing the OpenAI-compatible API + admin endpoints.
pub fn router(state: Arc<GatewayState>) -> Router {
    routes::create_router(state)
}
