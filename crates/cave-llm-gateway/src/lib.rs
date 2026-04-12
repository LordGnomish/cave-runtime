//! CAVE LLM Gateway — Unified LLM API gateway with multi-provider routing.
//!
//! Replaces: LiteLLM, OpenRouter
//! OpenAI-compatible API with multi-provider routing, spend tracking, guardrails, and caching.

pub mod guardrails;
pub mod models;
pub mod router;
pub mod routes;
pub mod store;

use axum::Router;
use std::sync::Arc;

/// Shared application state for the LLM gateway module.
pub struct LlmGatewayState {
    pub store: Arc<store::LlmGatewayStore>,
    pub router: Arc<router::ProviderRouter>,
    pub guardrails: Arc<guardrails::GuardrailEngine>,
}

impl Default for LlmGatewayState {
    fn default() -> Self {
        Self {
            store: Arc::new(store::LlmGatewayStore::new()),
            router: Arc::new(router::ProviderRouter::new()),
            guardrails: Arc::new(guardrails::GuardrailEngine::new()),
        }
    }
}

/// Build the axum router for this module, mounting all routes onto the provided state.
pub fn router(state: Arc<LlmGatewayState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "llm-gateway";
