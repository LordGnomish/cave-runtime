// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! ProviderConfig — credentials source + usage tracking.
//!
//! Upstream: internal/controller/pkg/manager/providerconfig.go

use crate::error::{CrossplaneError, CrossplaneResult};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "source", rename_all = "PascalCase")]
pub enum Credentials {
    Secret { namespace: String, name: String, key: String },
    Filesystem { path: String },
    Environment { var: String },
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub name: String,
    pub credentials: Credentials,
}

impl ProviderConfig {
    pub fn new(name: impl Into<String>, credentials: Credentials) -> Self {
        Self {
            name: name.into(),
            credentials,
        }
    }
}

#[derive(Default)]
pub struct ProviderConfigStore {
    configs: DashMap<String, ProviderConfig>,
    usage: DashMap<String, AtomicUsize>,
}

impl ProviderConfigStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert(&self, cfg: ProviderConfig) -> CrossplaneResult<()> {
        if cfg.name.is_empty() {
            return Err(CrossplaneError::Internal("config name required".into()));
        }
        self.usage.entry(cfg.name.clone()).or_default();
        self.configs.insert(cfg.name.clone(), cfg);
        Ok(())
    }

    pub fn get(&self, name: &str) -> CrossplaneResult<ProviderConfig> {
        self.configs
            .get(name)
            .map(|r| r.clone())
            .ok_or_else(|| CrossplaneError::ProviderNotFound(name.to_owned()))
    }

    pub fn list(&self) -> Vec<ProviderConfig> {
        self.configs.iter().map(|r| r.value().clone()).collect()
    }

    pub fn add_usage(&self, name: &str) -> usize {
        self.usage
            .entry(name.to_string())
            .or_default()
            .fetch_add(1, Ordering::SeqCst)
            + 1
    }

    pub fn release_usage(&self, name: &str) -> usize {
        let entry = self.usage.entry(name.to_string()).or_default();
        let cur = entry.load(Ordering::SeqCst);
        if cur == 0 {
            return 0;
        }
        entry.fetch_sub(1, Ordering::SeqCst) - 1
    }

    pub fn usage_of(&self, name: &str) -> usize {
        self.usage
            .get(name)
            .map(|r| r.load(Ordering::SeqCst))
            .unwrap_or(0)
    }

    pub fn delete(&self, name: &str) -> CrossplaneResult<()> {
        if self.usage_of(name) > 0 {
            return Err(CrossplaneError::Internal(format!(
                "providerconfig {} still has usage {} > 0",
                name,
                self.usage_of(name)
            )));
        }
        self.configs.remove(name);
        self.usage.remove(name);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_and_get() {
        let s = ProviderConfigStore::new();
        s.upsert(ProviderConfig::new("default", Credentials::None))
            .unwrap();
        assert_eq!(s.get("default").unwrap().name, "default");
    }

    #[test]
    fn empty_name_rejected() {
        let s = ProviderConfigStore::new();
        assert!(s
            .upsert(ProviderConfig::new("", Credentials::None))
            .is_err());
    }

    #[test]
    fn usage_increment_decrement() {
        let s = ProviderConfigStore::new();
        s.upsert(ProviderConfig::new("p", Credentials::None)).unwrap();
        assert_eq!(s.add_usage("p"), 1);
        assert_eq!(s.add_usage("p"), 2);
        assert_eq!(s.release_usage("p"), 1);
        assert_eq!(s.usage_of("p"), 1);
    }

    #[test]
    fn delete_with_usage_errors() {
        let s = ProviderConfigStore::new();
        s.upsert(ProviderConfig::new("p", Credentials::None)).unwrap();
        s.add_usage("p");
        assert!(s.delete("p").is_err());
    }

    #[test]
    fn delete_zero_usage_ok() {
        let s = ProviderConfigStore::new();
        s.upsert(ProviderConfig::new("p", Credentials::None)).unwrap();
        s.delete("p").unwrap();
        assert!(s.get("p").is_err());
    }

    #[test]
    fn list_returns_all() {
        let s = ProviderConfigStore::new();
        s.upsert(ProviderConfig::new("a", Credentials::None)).unwrap();
        s.upsert(ProviderConfig::new("b", Credentials::None)).unwrap();
        assert_eq!(s.list().len(), 2);
    }

    #[test]
    fn credentials_secret_variant() {
        let c = Credentials::Secret {
            namespace: "ns".into(),
            name: "n".into(),
            key: "k".into(),
        };
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("Secret"));
    }

    #[test]
    fn release_usage_floor_zero() {
        let s = ProviderConfigStore::new();
        s.upsert(ProviderConfig::new("p", Credentials::None)).unwrap();
        assert_eq!(s.release_usage("p"), 0);
        assert_eq!(s.release_usage("p"), 0);
    }
}
