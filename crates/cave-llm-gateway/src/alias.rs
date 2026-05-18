// SPDX-License-Identifier: AGPL-3.0-or-later
//! Model aliasing — map friendly names to canonical provider+model pairs.

use dashmap::DashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelAlias {
    pub alias: String,
    pub provider: String,
    pub model: String,
    pub description: Option<String>,
}

pub struct AliasRegistry {
    aliases: DashMap<String, ModelAlias>,
}

impl AliasRegistry {
    pub fn new() -> Self {
        let r = Self { aliases: DashMap::new() };
        // Register sensible defaults
        for (alias, provider, model) in default_aliases() {
            r.register(ModelAlias { alias, provider, model, description: None });
        }
        r
    }

    pub fn register(&self, alias: ModelAlias) {
        self.aliases.insert(alias.alias.clone(), alias);
    }

    /// Resolve an alias to (provider, model). Returns None if no alias matches;
    /// caller should use the name verbatim as the model.
    pub fn resolve(&self, name: &str) -> Option<ModelAlias> {
        self.aliases.get(name).map(|a| a.clone())
    }

    pub fn list(&self) -> Vec<ModelAlias> {
        self.aliases.iter().map(|e| e.value().clone()).collect()
    }

    pub fn delete(&self, alias: &str) -> bool {
        self.aliases.remove(alias).is_some()
    }
}

impl Default for AliasRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn default_aliases() -> Vec<(String, String, String)> {
    vec![
        // OpenAI shorthand
        ("gpt4".into(), "openai".into(), "gpt-4o".into()),
        ("gpt4o".into(), "openai".into(), "gpt-4o".into()),
        ("gpt4-mini".into(), "openai".into(), "gpt-4o-mini".into()),
        ("gpt4o-mini".into(), "openai".into(), "gpt-4o-mini".into()),
        ("gpt35".into(), "openai".into(), "gpt-3.5-turbo".into()),
        // Anthropic shorthand
        ("opus".into(), "anthropic".into(), "claude-opus-4-6".into()),
        ("sonnet".into(), "anthropic".into(), "claude-sonnet-4-6".into()),
        ("haiku".into(), "anthropic".into(), "claude-haiku-4-5-20251001".into()),
        ("claude".into(), "anthropic".into(), "claude-sonnet-4-6".into()),
        // Local
        ("local".into(), "local".into(), "llama3".into()),
        ("llama".into(), "local".into(), "llama3".into()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_default_alias() {
        let r = AliasRegistry::new();
        let alias = r.resolve("sonnet").unwrap();
        assert_eq!(alias.provider, "anthropic");
        assert_eq!(alias.model, "claude-sonnet-4-6");
    }

    #[test]
    fn resolve_unknown_returns_none() {
        let r = AliasRegistry::new();
        assert!(r.resolve("my-custom-model").is_none());
    }

    #[test]
    fn register_custom_alias() {
        let r = AliasRegistry::new();
        r.register(ModelAlias {
            alias: "my-model".into(),
            provider: "local".into(),
            model: "phi3".into(),
            description: Some("My local Phi-3".into()),
        });
        let a = r.resolve("my-model").unwrap();
        assert_eq!(a.model, "phi3");
    }

    #[test]
    fn delete_alias() {
        let r = AliasRegistry::new();
        assert!(r.delete("haiku"));
        assert!(r.resolve("haiku").is_none());
    }
}
