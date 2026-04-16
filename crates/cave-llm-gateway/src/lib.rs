//! CAVE LLM Gateway — Multi-provider LLM router and proxy.
//!
//! Replaces: LiteLLM, custom AI gateway, OpenAI proxy
//!
//! Features:
//! - Multi-provider routing (OpenAI, Anthropic, local models)
//! - OpenAI chat completions API compatibility
//! - Load balancing across providers
//! - Fallback chains (provider A → B → C)
//! - Rate limiting per consumer
//! - Token counting and cost tracking
//! - Prompt caching
//! - Streaming support (SSE)
//! - Model aliasing
//! - Request/response logging
//! - Guardrails (input/output filtering)
//! - API key management

pub mod alias;
pub mod api_keys;
pub mod cache;
pub mod config;
pub mod cost;
pub mod embedded;
pub mod error;
pub mod guardrails;
pub mod insights;
pub mod logging;
pub mod openai;
pub mod provider;
pub mod rate_limit;
pub mod router;
pub mod routes;
pub mod streaming;

use axum::Router;
use std::sync::Arc;

pub use config::GatewayConfig;
pub use embedded::{ChatTemplate, EmbeddedConfig, EmbeddedProvider};
pub use error::{GatewayError, GatewayResult};
pub use insights::InsightsEngine;
pub use router::GatewayRouter;

pub const MODULE_NAME: &str = "llm-gateway";

/// Shared state for the LLM gateway.
pub struct GatewayState {
    pub router: Arc<GatewayRouter>,
}

impl GatewayState {
    pub fn new(router: GatewayRouter) -> Self {
        Self { router: Arc::new(router) }
    }

    /// Build state from a `GatewayConfig` (typically deserialised from YAML).
    pub fn from_config(cfg: &GatewayConfig) -> Self {
        Self::new(cfg.build_router())
    }
}

impl Default for GatewayState {
    /// Mock-only state — useful for tests and as a fallback when no config
    /// is supplied. Real deployments should use `from_config`.
    fn default() -> Self {
        use crate::alias::AliasRegistry;
        use crate::provider::{MockProvider, ProviderRegistry};
        use crate::router::RoutingStrategy;

        let providers = Arc::new(ProviderRegistry::default());
        let aliases = Arc::new(AliasRegistry::new());
        let _ = providers; // already registered "mock" via Default
        let providers = {
            let r = ProviderRegistry::new();
            r.register(Arc::new(MockProvider::new("mock")));
            Arc::new(r)
        };
        Self::new(GatewayRouter::new(
            providers,
            aliases,
            RoutingStrategy::Fixed { provider: "mock".into() },
        ))
    }
}

/// Build Axum router exposing the OpenAI-compatible API + admin endpoints.
pub fn router(state: Arc<GatewayState>) -> Router {
    routes::create_router(state)
}
