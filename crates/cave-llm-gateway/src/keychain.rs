// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Read SaaS provider API keys from the macOS Keychain or an env-var
//! fallback.
//!
//! Convention: keys are stored under the service name `cave-llm-gateway-<provider>`
//! (e.g. `cave-llm-gateway-anthropic`). The lookup order is:
//!   1. explicit env-var override `CAVE_LLM_<PROVIDER>_API_KEY`
//!   2. macOS Keychain via `/usr/bin/security find-generic-password -w -s ...`
//!   3. `None`
//!
//! No key is ever written from this module — secrets only flow inbound.

use std::process::Command;

/// Logical SaaS providers cave-llm-gateway needs an API key for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeychainProvider {
    Anthropic,
    OpenAi,
    Mistral,
}

impl KeychainProvider {
    pub fn slug(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
            Self::Mistral => "mistral",
        }
    }

    pub fn service_name(self) -> String {
        format!("cave-llm-gateway-{}", self.slug())
    }

    pub fn env_var(self) -> String {
        format!("CAVE_LLM_{}_API_KEY", self.slug().to_ascii_uppercase())
    }
}

/// Lookup result — distinguishes the source so callers can audit which
/// path delivered the key without ever logging the secret itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySource {
    EnvVar(String),
    Keychain(String),
    NotFound,
}

impl KeySource {
    pub fn label(&self) -> &'static str {
        match self {
            Self::EnvVar(_) => "env-var",
            Self::Keychain(_) => "keychain",
            Self::NotFound => "not-found",
        }
    }

    pub fn secret(&self) -> Option<&str> {
        match self {
            Self::EnvVar(s) | Self::Keychain(s) => Some(s.as_str()),
            Self::NotFound => None,
        }
    }
}

pub trait SecretBackend: Send + Sync {
    fn env_get(&self, var: &str) -> Option<String>;
    fn keychain_get(&self, service: &str) -> Option<String>;
}

pub struct SystemBackend;

impl SecretBackend for SystemBackend {
    fn env_get(&self, var: &str) -> Option<String> {
        std::env::var(var).ok().filter(|s| !s.is_empty())
    }

    fn keychain_get(&self, service: &str) -> Option<String> {
        // macOS only; on Linux/Windows `security` isn't on PATH so we
        // silently return None.
        let out = Command::new("/usr/bin/security")
            .args(["find-generic-password", "-w", "-s", service])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let raw = String::from_utf8(out.stdout).ok()?;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }
}

pub fn resolve_with(backend: &dyn SecretBackend, provider: KeychainProvider) -> KeySource {
    if let Some(s) = backend.env_get(&provider.env_var()) {
        return KeySource::EnvVar(s);
    }
    if let Some(s) = backend.keychain_get(&provider.service_name()) {
        return KeySource::Keychain(s);
    }
    KeySource::NotFound
}

/// Convenience: resolve from the real system backend.
pub fn resolve(provider: KeychainProvider) -> KeySource {
    resolve_with(&SystemBackend, provider)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct FakeBackend {
        env: Mutex<HashMap<String, String>>,
        keychain: Mutex<HashMap<String, String>>,
    }

    impl FakeBackend {
        fn new() -> Self {
            Self {
                env: Mutex::new(HashMap::new()),
                keychain: Mutex::new(HashMap::new()),
            }
        }
        fn set_env(&self, k: &str, v: &str) {
            self.env.lock().unwrap().insert(k.into(), v.into());
        }
        fn set_keychain(&self, k: &str, v: &str) {
            self.keychain.lock().unwrap().insert(k.into(), v.into());
        }
    }

    impl SecretBackend for FakeBackend {
        fn env_get(&self, var: &str) -> Option<String> {
            self.env.lock().unwrap().get(var).cloned()
        }
        fn keychain_get(&self, service: &str) -> Option<String> {
            self.keychain.lock().unwrap().get(service).cloned()
        }
    }

    #[test]
    fn service_name_is_namespaced_with_crate_name() {
        assert_eq!(
            KeychainProvider::Anthropic.service_name(),
            "cave-llm-gateway-anthropic"
        );
    }

    #[test]
    fn env_var_is_uppercase_with_underscores() {
        assert_eq!(
            KeychainProvider::OpenAi.env_var(),
            "CAVE_LLM_OPENAI_API_KEY"
        );
    }

    #[test]
    fn env_takes_precedence_over_keychain() {
        let b = FakeBackend::new();
        b.set_env("CAVE_LLM_ANTHROPIC_API_KEY", "from-env");
        b.set_keychain("cave-llm-gateway-anthropic", "from-keychain");
        let r = resolve_with(&b, KeychainProvider::Anthropic);
        assert_eq!(r, KeySource::EnvVar("from-env".into()));
        assert_eq!(r.label(), "env-var");
    }

    #[test]
    fn keychain_used_when_env_missing() {
        let b = FakeBackend::new();
        b.set_keychain("cave-llm-gateway-openai", "kc-key");
        let r = resolve_with(&b, KeychainProvider::OpenAi);
        assert_eq!(r, KeySource::Keychain("kc-key".into()));
        assert_eq!(r.secret(), Some("kc-key"));
    }

    #[test]
    fn not_found_when_both_sources_empty() {
        let b = FakeBackend::new();
        let r = resolve_with(&b, KeychainProvider::Mistral);
        assert_eq!(r, KeySource::NotFound);
        assert_eq!(r.label(), "not-found");
        assert_eq!(r.secret(), None);
    }
}
