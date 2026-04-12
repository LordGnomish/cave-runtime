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
pub mod cost;
pub mod error;
pub mod guardrails;
pub mod logging;
pub mod openai;
pub mod provider;
pub mod rate_limit;
pub mod router;
pub mod routes;
pub mod streaming;

use axum::Router;
use std::sync::Arc;

pub use error::{GatewayError, GatewayResult};
pub use router::GatewayRouter;

pub const MODULE_NAME: &str = "llm-gateway";

/// Shared state for the LLM gateway.
pub struct GatewayState {
    pub router: Arc<GatewayRouter>,
}

impl GatewayState {
    pub fn new(router: GatewayRouter) -> Self {
        Self {
            router: Arc::new(router),
        }
    }
}

impl Default for GatewayState {
    fn default() -> Self {
        todo!("GatewayState requires provider configuration — use GatewayState::new(router)")
    }
}

/// Build Axum router exposing the OpenAI-compatible API + admin endpoints.
pub fn router(state: Arc<GatewayState>) -> Router {
    routes::create_router(state)
}
