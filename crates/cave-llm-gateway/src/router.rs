//! Provider routing and load balancing for CAVE LLM Gateway.

use crate::models::{ModelAlias, Provider, ProviderConfig};
use std::collections::HashMap;

/// Routes requests to the appropriate LLM provider.
pub struct ProviderRouter {
    providers: Vec<ProviderConfig>,
    model_aliases: HashMap<String, ModelAlias>,
}

impl ProviderRouter {
    /// Create a new router with default provider configurations and model aliases.
    pub fn new() -> Self {
        let providers = vec![
            ProviderConfig {
                provider: Provider::OpenAi,
                api_base_url: "https://api.openai.com/v1".to_string(),
                api_key_env_var: "OPENAI_API_KEY".to_string(),
                enabled: true,
                weight: 10,
                max_retries: 3,
                timeout_seconds: 30,
                models: vec![
                    "gpt-4o".to_string(),
                    "gpt-4o-mini".to_string(),
                    "gpt-3.5-turbo".to_string(),
                    "text-embedding-3-small".to_string(),
                    "text-embedding-3-large".to_string(),
                ],
            },
            ProviderConfig {
                provider: Provider::Anthropic,
                api_base_url: "https://api.anthropic.com/v1".to_string(),
                api_key_env_var: "ANTHROPIC_API_KEY".to_string(),
                enabled: true,
                weight: 8,
                max_retries: 3,
                timeout_seconds: 60,
                models: vec![
                    "claude-opus-4-6".to_string(),
                    "claude-sonnet-4-6".to_string(),
                    "claude-haiku-4-5".to_string(),
                ],
            },
            ProviderConfig {
                provider: Provider::AzureOpenAi,
                api_base_url: "https://YOUR_RESOURCE.openai.azure.com".to_string(),
                api_key_env_var: "AZURE_OPENAI_API_KEY".to_string(),
                enabled: false,
                weight: 5,
                max_retries: 2,
                timeout_seconds: 30,
                models: vec![
                    "gpt-4o".to_string(),
                    "gpt-4o-mini".to_string(),
                ],
            },
            ProviderConfig {
                provider: Provider::GoogleVertexAi,
                api_base_url: "https://us-central1-aiplatform.googleapis.com".to_string(),
                api_key_env_var: "GOOGLE_APPLICATION_CREDENTIALS".to_string(),
                enabled: false,
                weight: 5,
                max_retries: 2,
                timeout_seconds: 30,
                models: vec![
                    "gemini-1.5-pro".to_string(),
                    "gemini-1.5-flash".to_string(),
                ],
            },
            ProviderConfig {
                provider: Provider::Local,
                api_base_url: "http://localhost:11434/v1".to_string(),
                api_key_env_var: "LOCAL_LLM_API_KEY".to_string(),
                enabled: false,
                weight: 1,
                max_retries: 1,
                timeout_seconds: 120,
                models: vec![
                    "llama3".to_string(),
                    "mistral".to_string(),
                ],
            },
        ];

        let aliases_list = vec![
            ModelAlias {
                alias: "smart".to_string(),
                provider: Provider::OpenAi,
                model_id: "gpt-4o".to_string(),
                max_tokens: Some(4096),
                pinned_version: None,
            },
            ModelAlias {
                alias: "fast".to_string(),
                provider: Provider::OpenAi,
                model_id: "gpt-4o-mini".to_string(),
                max_tokens: Some(4096),
                pinned_version: None,
            },
            ModelAlias {
                alias: "embedding".to_string(),
                provider: Provider::OpenAi,
                model_id: "text-embedding-3-small".to_string(),
                max_tokens: None,
                pinned_version: None,
            },
            ModelAlias {
                alias: "claude-smart".to_string(),
                provider: Provider::Anthropic,
                model_id: "claude-sonnet-4-6".to_string(),
                max_tokens: Some(4096),
                pinned_version: None,
            },
            ModelAlias {
                alias: "claude-fast".to_string(),
                provider: Provider::Anthropic,
                model_id: "claude-haiku-4-5".to_string(),
                max_tokens: Some(4096),
                pinned_version: None,
            },
        ];

        let mut model_aliases = HashMap::new();
        for alias in aliases_list {
            model_aliases.insert(alias.alias.clone(), alias);
        }

        Self { providers, model_aliases }
    }

    /// Resolve a model name (which may be an alias) to (provider, actual model ID).
    pub fn resolve_model(&self, model: &str) -> Option<(Provider, String)> {
        // Check aliases first.
        if let Some(alias) = self.model_aliases.get(model) {
            return Some((alias.provider.clone(), alias.model_id.clone()));
        }
        // Otherwise look through provider model lists.
        for config in &self.providers {
            if config.models.contains(&model.to_string()) {
                return Some((config.provider.clone(), model.to_string()));
            }
        }
        None
    }

    /// Select an enabled provider config for the given model.
    ///
    /// If `force_provider` is specified, that provider is returned if enabled.
    /// Otherwise the highest-weight enabled provider that supports the model is chosen.
    pub fn select_provider(&self, model: &str, force_provider: Option<&str>) -> Option<&ProviderConfig> {
        // Resolve alias to real model id.
        let resolved_model = if let Some(alias) = self.model_aliases.get(model) {
            alias.model_id.clone()
        } else {
            model.to_string()
        };

        if let Some(fp) = force_provider {
            let target = match fp.to_lowercase().as_str() {
                "openai" | "open_ai" => Some(Provider::OpenAi),
                "anthropic" => Some(Provider::Anthropic),
                "azure" | "azure_open_ai" => Some(Provider::AzureOpenAi),
                "google" | "google_vertex_ai" => Some(Provider::GoogleVertexAi),
                "local" => Some(Provider::Local),
                _ => None,
            };
            if let Some(provider_variant) = target {
                return self.providers.iter()
                    .find(|c| c.provider == provider_variant && c.enabled);
            }
        }

        // Pick highest-weight enabled provider that has the model.
        self.providers.iter()
            .filter(|c| c.enabled && c.models.contains(&resolved_model))
            .max_by_key(|c| c.weight)
    }

    /// Build a fallback chain for a given model.
    ///
    /// Returns `(provider, model_id)` pairs ordered by preference.
    /// Explicit fallbacks are prepended; all other enabled providers are appended.
    pub fn get_fallback_chain(&self, model: &str, explicit_fallbacks: &[String]) -> Vec<(Provider, String)> {
        let mut chain: Vec<(Provider, String)> = Vec::new();

        // Add primary resolved model.
        if let Some(primary) = self.resolve_model(model) {
            chain.push(primary);
        }

        // Add explicit fallbacks.
        for fb in explicit_fallbacks {
            if let Some(resolved) = self.resolve_model(fb) {
                if !chain.contains(&resolved) {
                    chain.push(resolved);
                }
            }
        }

        // Append any remaining enabled providers.
        for config in &self.providers {
            if !config.enabled {
                continue;
            }
            for m in &config.models {
                let entry = (config.provider.clone(), m.clone());
                if !chain.contains(&entry) {
                    chain.push(entry);
                }
            }
        }

        chain
    }

    /// Estimate the USD cost for a request.
    ///
    /// Pricing is approximate (per 1 million tokens):
    /// - OpenAI gpt-4o: $5 prompt / $15 completion
    /// - OpenAI gpt-4o-mini: $0.15 prompt / $0.60 completion
    /// - Anthropic claude-sonnet-4-6: $3 prompt / $15 completion
    /// - Anthropic claude-haiku-4-5: $0.25 prompt / $1.25 completion
    /// - Default: $1 prompt / $3 completion
    pub fn estimate_cost(provider: &Provider, model: &str, prompt_tokens: u32, completion_tokens: u32) -> f64 {
        let (prompt_per_m, completion_per_m): (f64, f64) = match (provider, model) {
            (Provider::OpenAi, "gpt-4o") => (5.0, 15.0),
            (Provider::OpenAi, "gpt-4o-mini") => (0.15, 0.60),
            (Provider::OpenAi, "gpt-3.5-turbo") => (0.50, 1.50),
            (Provider::Anthropic, "claude-opus-4-6") => (15.0, 75.0),
            (Provider::Anthropic, "claude-sonnet-4-6") => (3.0, 15.0),
            (Provider::Anthropic, "claude-haiku-4-5") => (0.25, 1.25),
            _ => (1.0, 3.0),
        };

        let prompt_cost = (prompt_tokens as f64 / 1_000_000.0) * prompt_per_m;
        let completion_cost = (completion_tokens as f64 / 1_000_000.0) * completion_per_m;
        prompt_cost + completion_cost
    }

    /// Add or overwrite a model alias.
    pub fn add_model_alias(&mut self, alias: ModelAlias) {
        self.model_aliases.insert(alias.alias.clone(), alias);
    }

    /// Return all known model names: aliases plus every provider model.
    pub fn list_models(&self) -> Vec<String> {
        let mut models: Vec<String> = self.model_aliases.keys().cloned().collect();
        for config in &self.providers {
            for m in &config.models {
                if !models.contains(m) {
                    models.push(m.clone());
                }
            }
        }
        models.sort();
        models
    }

    /// Return all provider configurations.
    pub fn list_providers(&self) -> Vec<&ProviderConfig> {
        self.providers.iter().collect()
    }
}

impl Default for ProviderRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_alias_smart() {
        let router = ProviderRouter::new();
        let result = router.resolve_model("smart");
        assert_eq!(result, Some((Provider::OpenAi, "gpt-4o".to_string())));
    }

    #[test]
    fn test_resolve_alias_fast() {
        let router = ProviderRouter::new();
        let result = router.resolve_model("fast");
        assert_eq!(result, Some((Provider::OpenAi, "gpt-4o-mini".to_string())));
    }

    #[test]
    fn test_resolve_alias_embedding() {
        let router = ProviderRouter::new();
        let result = router.resolve_model("embedding");
        assert_eq!(result, Some((Provider::OpenAi, "text-embedding-3-small".to_string())));
    }

    #[test]
    fn test_resolve_direct_model() {
        let router = ProviderRouter::new();
        let result = router.resolve_model("gpt-4o");
        assert_eq!(result, Some((Provider::OpenAi, "gpt-4o".to_string())));
    }

    #[test]
    fn test_resolve_unknown_model() {
        let router = ProviderRouter::new();
        assert!(router.resolve_model("unknown-model-xyz").is_none());
    }

    #[test]
    fn test_select_provider_default() {
        let router = ProviderRouter::new();
        let config = router.select_provider("gpt-4o", None);
        assert!(config.is_some());
        assert_eq!(config.unwrap().provider, Provider::OpenAi);
    }

    #[test]
    fn test_select_provider_forced() {
        let router = ProviderRouter::new();
        // Anthropic is enabled; forcing should work even if model doesn't match.
        let config = router.select_provider("gpt-4o", Some("anthropic"));
        assert!(config.is_some());
        assert_eq!(config.unwrap().provider, Provider::Anthropic);
    }

    #[test]
    fn test_cost_gpt4o() {
        // 1M prompt + 1M completion = $5 + $15 = $20
        let cost = ProviderRouter::estimate_cost(&Provider::OpenAi, "gpt-4o", 1_000_000, 1_000_000);
        assert!((cost - 20.0).abs() < 0.001);
    }

    #[test]
    fn test_cost_gpt4o_mini() {
        let cost = ProviderRouter::estimate_cost(&Provider::OpenAi, "gpt-4o-mini", 1_000_000, 1_000_000);
        assert!((cost - 0.75).abs() < 0.001);
    }

    #[test]
    fn test_cost_claude_sonnet() {
        let cost = ProviderRouter::estimate_cost(&Provider::Anthropic, "claude-sonnet-4-6", 1_000_000, 1_000_000);
        assert!((cost - 18.0).abs() < 0.001);
    }

    #[test]
    fn test_cost_claude_haiku() {
        let cost = ProviderRouter::estimate_cost(&Provider::Anthropic, "claude-haiku-4-5", 1_000_000, 1_000_000);
        assert!((cost - 1.5).abs() < 0.001);
    }

    #[test]
    fn test_cost_default_fallback() {
        let cost = ProviderRouter::estimate_cost(&Provider::Local, "llama3", 1_000_000, 1_000_000);
        assert!((cost - 4.0).abs() < 0.001);
    }

    #[test]
    fn test_fallback_chain_includes_primary() {
        let router = ProviderRouter::new();
        let chain = router.get_fallback_chain("smart", &[]);
        assert!(!chain.is_empty());
        assert_eq!(chain[0], (Provider::OpenAi, "gpt-4o".to_string()));
    }

    #[test]
    fn test_fallback_chain_explicit() {
        let router = ProviderRouter::new();
        let chain = router.get_fallback_chain("smart", &["claude-fast".to_string()]);
        assert!(chain.contains(&(Provider::Anthropic, "claude-haiku-4-5".to_string())));
    }

    #[test]
    fn test_list_models_contains_aliases_and_providers() {
        let router = ProviderRouter::new();
        let models = router.list_models();
        assert!(models.contains(&"smart".to_string()));
        assert!(models.contains(&"gpt-4o".to_string()));
        assert!(models.contains(&"claude-sonnet-4-6".to_string()));
    }

    #[test]
    fn test_add_model_alias() {
        let mut router = ProviderRouter::new();
        router.add_model_alias(ModelAlias {
            alias: "custom".to_string(),
            provider: Provider::Local,
            model_id: "my-local-model".to_string(),
            max_tokens: Some(2048),
            pinned_version: None,
        });
        let result = router.resolve_model("custom");
        assert_eq!(result, Some((Provider::Local, "my-local-model".to_string())));
    }
}
