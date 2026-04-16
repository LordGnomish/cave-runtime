//! YAML config schema for the gateway. Consumed by `GatewayState::from_config`.
//!
//! Example:
//! ```yaml
//! llm_gateway:
//!   enabled: true
//!   default_model: embedded
//!   strategy: fallback
//!   providers:
//!     - name: embedded
//!       type: embedded
//!       embedded:
//!         name: embedded
//!         model_path: ~/.cave/models/qwen2.5-coder-7b-instruct-q4_k_m.gguf
//!         model_id: qwen2.5-coder-7b
//!         context_size: 16384
//!         gpu_layers: -1
//!         chat_template: qwen
//!   aliases:
//!     - alias: triage-fast
//!       provider: embedded
//!       model: qwen2.5-coder-7b
//! ```

use crate::alias::{AliasRegistry, ModelAlias};
use crate::provider::{ProviderConfig, ProviderRegistry};
use crate::router::{GatewayRouter, RoutingStrategy};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Model id used when clients send a request without naming one explicitly.
    #[serde(default)]
    pub default_model: Option<String>,
    /// Routing strategy across providers.
    #[serde(default = "default_strategy")]
    pub strategy: RoutingStrategy,
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    #[serde(default)]
    pub aliases: Vec<ModelAlias>,
}

fn default_enabled() -> bool { true }

fn default_strategy() -> RoutingStrategy {
    RoutingStrategy::Fallback { providers: vec![] }
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            default_model: None,
            strategy: default_strategy(),
            providers: vec![],
            aliases: vec![],
        }
    }
}

impl GatewayConfig {
    /// Build a fully-initialised router from this config.
    pub fn build_router(&self) -> GatewayRouter {
        let providers = Arc::new(ProviderRegistry::from_config(self.providers.clone()));
        let aliases = Arc::new(AliasRegistry::new());
        for a in &self.aliases {
            aliases.register(a.clone());
        }
        GatewayRouter::new(providers, aliases, self.strategy.clone())
    }
}
