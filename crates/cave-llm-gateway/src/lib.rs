//! CAVE LLM Gateway — LiteLLM / AI Gateway replacement.
//!
//! Replaces: LiteLLM, AI Gateway
//! Unified LLM routing proxy with provider failover, semantic caching,
//! guardrails (PII, content policy, budget, rate limiting), and token budget tracking.

pub mod budget;
pub mod cache;
pub mod guardrails;
pub mod models;
pub mod router;
pub mod routes;

use axum::Router;
use models::{ModelMapping, ProviderType, RoutingPolicy, SemanticCacheEntry};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Shared mutable state for the LLM gateway.
///
/// All fields are wrapped in `Mutex` so they can be updated at runtime via the
/// management API without a service restart. Use `Arc<GatewayState>` in handlers.
pub struct GatewayState {
    pub providers: Mutex<Vec<models::LlmProvider>>,
    pub policies: Mutex<Vec<RoutingPolicy>>,
    pub budgets: Mutex<Vec<models::TokenBudget>>,
    /// Prompt-hash → cached response.
    pub cache: Mutex<HashMap<String, SemanticCacheEntry>>,
    pub guardrails: Mutex<Vec<models::Guardrail>>,
    /// Model alias → per-provider model names.
    pub model_mappings: Mutex<HashMap<String, ModelMapping>>,
}

impl Default for GatewayState {
    fn default() -> Self {
        Self {
            providers: Mutex::new(Vec::new()),
            policies: Mutex::new(vec![RoutingPolicy::default()]),
            budgets: Mutex::new(Vec::new()),
            cache: Mutex::new(HashMap::new()),
            guardrails: Mutex::new(Vec::new()),
            model_mappings: Mutex::new(builtin_model_mappings()),
        }
    }
}

/// Well-known model aliases pre-populated for common cross-provider mappings.
fn builtin_model_mappings() -> HashMap<String, ModelMapping> {
    let mut m = HashMap::new();

    let mut gpt4_providers = HashMap::new();
    gpt4_providers.insert(ProviderType::OpenAI, "gpt-4-turbo".to_string());
    gpt4_providers.insert(
        ProviderType::Anthropic,
        "claude-sonnet-4-20250514".to_string(),
    );
    m.insert(
        "gpt-4".to_string(),
        ModelMapping {
            alias: "gpt-4".to_string(),
            provider_models: gpt4_providers,
        },
    );

    let mut claude_providers = HashMap::new();
    claude_providers.insert(
        ProviderType::Anthropic,
        "claude-sonnet-4-20250514".to_string(),
    );
    claude_providers.insert(ProviderType::OpenAI, "gpt-4o".to_string());
    m.insert(
        "claude-sonnet".to_string(),
        ModelMapping {
            alias: "claude-sonnet".to_string(),
            provider_models: claude_providers,
        },
    );

    m
}

/// Create the axum router for the LLM gateway.
pub fn router(state: Arc<GatewayState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "llm-gateway";
