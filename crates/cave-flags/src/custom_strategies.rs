// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Custom activation strategies — parity with
//! `src/lib/features/custom-activation-strategies/*` (Unleash v5.0.0).
//!
//! Server-side custom strategy registration: the admin UI POSTs a
//! strategy definition (name + parameters), cave-flags persists it and
//! advertises it in the `/api/admin/strategies` response. The
//! evaluation engine treats unknown strategies as `false` unless they
//! match a registered custom strategy, in which case it uses the
//! caller-supplied parameter checks.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StrategyParameterType {
    String,
    Percentage,
    List,
    Number,
    Boolean,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomStrategyParameter {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: StrategyParameterType,
    pub description: Option<String>,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomStrategy {
    pub name: String,
    pub description: Option<String>,
    pub parameters: Vec<CustomStrategyParameter>,
    #[serde(default)]
    pub deprecated: bool,
    #[serde(rename = "editable", default = "default_true")]
    pub editable: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CustomStrategyError {
    #[error("strategy name must be non-empty and kebab-case")]
    InvalidName,
    #[error("strategy '{0}' already exists")]
    AlreadyExists(String),
    #[error("strategy '{0}' is built-in and cannot be modified")]
    BuiltinReserved(String),
    #[error("strategy '{0}' not found")]
    NotFound(String),
}

const BUILTIN_NAMES: &[&str] = &[
    "default",
    "userWithId",
    "gradualRolloutRandom",
    "gradualRolloutSessionId",
    "gradualRolloutUserId",
    "flexibleRollout",
    "remoteAddress",
    "applicationHostname",
];

/// In-memory custom-strategy registry. The runtime mirrors this from
/// the `custom_strategies` table on startup and on writes.
#[derive(Default)]
pub struct CustomStrategyRegistry {
    inner: RwLock<HashMap<String, CustomStrategy>>,
}

impl CustomStrategyRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, strategy: CustomStrategy) -> Result<(), CustomStrategyError> {
        validate_name(&strategy.name)?;
        if BUILTIN_NAMES.contains(&strategy.name.as_str()) {
            return Err(CustomStrategyError::BuiltinReserved(strategy.name));
        }
        let mut g = self.inner.write().unwrap();
        if g.contains_key(&strategy.name) {
            return Err(CustomStrategyError::AlreadyExists(strategy.name));
        }
        g.insert(strategy.name.clone(), strategy);
        Ok(())
    }

    pub fn update(&self, strategy: CustomStrategy) -> Result<(), CustomStrategyError> {
        validate_name(&strategy.name)?;
        if BUILTIN_NAMES.contains(&strategy.name.as_str()) {
            return Err(CustomStrategyError::BuiltinReserved(strategy.name));
        }
        let mut g = self.inner.write().unwrap();
        if !g.contains_key(&strategy.name) {
            return Err(CustomStrategyError::NotFound(strategy.name));
        }
        g.insert(strategy.name.clone(), strategy);
        Ok(())
    }

    pub fn deprecate(&self, name: &str) -> Result<(), CustomStrategyError> {
        if BUILTIN_NAMES.contains(&name) {
            return Err(CustomStrategyError::BuiltinReserved(name.to_string()));
        }
        let mut g = self.inner.write().unwrap();
        let entry = g
            .get_mut(name)
            .ok_or_else(|| CustomStrategyError::NotFound(name.to_string()))?;
        entry.deprecated = true;
        Ok(())
    }

    pub fn delete(&self, name: &str) -> Result<(), CustomStrategyError> {
        if BUILTIN_NAMES.contains(&name) {
            return Err(CustomStrategyError::BuiltinReserved(name.to_string()));
        }
        let mut g = self.inner.write().unwrap();
        g.remove(name)
            .ok_or_else(|| CustomStrategyError::NotFound(name.to_string()))?;
        Ok(())
    }

    pub fn list(&self) -> Vec<CustomStrategy> {
        let g = self.inner.read().unwrap();
        let mut v: Vec<CustomStrategy> = g.values().cloned().collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }

    pub fn get(&self, name: &str) -> Option<CustomStrategy> {
        self.inner.read().unwrap().get(name).cloned()
    }

    pub fn is_known(&self, name: &str) -> bool {
        BUILTIN_NAMES.contains(&name) || self.inner.read().unwrap().contains_key(name)
    }
}

fn validate_name(name: &str) -> Result<(), CustomStrategyError> {
    if name.is_empty() {
        return Err(CustomStrategyError::InvalidName);
    }
    if name.chars().next().unwrap().is_ascii_digit() {
        return Err(CustomStrategyError::InvalidName);
    }
    for c in name.chars() {
        if !(c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(CustomStrategyError::InvalidName);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(name: &str) -> CustomStrategy {
        CustomStrategy {
            name: name.into(),
            description: None,
            parameters: vec![CustomStrategyParameter {
                name: "country".into(),
                kind: StrategyParameterType::List,
                description: Some("ISO country codes".into()),
                required: true,
            }],
            deprecated: false,
            editable: true,
        }
    }

    #[test]
    fn register_and_list() {
        let r = CustomStrategyRegistry::new();
        r.register(mk("by-country")).unwrap();
        let list = r.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "by-country");
    }

    #[test]
    fn duplicate_register_rejected() {
        let r = CustomStrategyRegistry::new();
        r.register(mk("by-country")).unwrap();
        assert!(matches!(
            r.register(mk("by-country")),
            Err(CustomStrategyError::AlreadyExists(_))
        ));
    }

    #[test]
    fn builtin_name_rejected() {
        let r = CustomStrategyRegistry::new();
        assert!(matches!(
            r.register(mk("default")),
            Err(CustomStrategyError::BuiltinReserved(_))
        ));
    }

    #[test]
    fn invalid_name_rejected() {
        let r = CustomStrategyRegistry::new();
        assert!(matches!(
            r.register(mk("")),
            Err(CustomStrategyError::InvalidName)
        ));
        assert!(matches!(
            r.register(mk("1bad")),
            Err(CustomStrategyError::InvalidName)
        ));
        assert!(matches!(
            r.register(mk("has space")),
            Err(CustomStrategyError::InvalidName)
        ));
    }

    #[test]
    fn update_existing() {
        let r = CustomStrategyRegistry::new();
        r.register(mk("by-country")).unwrap();
        let mut updated = mk("by-country");
        updated.description = Some("v2".into());
        r.update(updated).unwrap();
        assert_eq!(r.get("by-country").unwrap().description.as_deref(), Some("v2"));
    }

    #[test]
    fn update_unknown_errors() {
        let r = CustomStrategyRegistry::new();
        assert!(matches!(
            r.update(mk("not-here")),
            Err(CustomStrategyError::NotFound(_))
        ));
    }

    #[test]
    fn deprecate_and_delete() {
        let r = CustomStrategyRegistry::new();
        r.register(mk("by-country")).unwrap();
        r.deprecate("by-country").unwrap();
        assert!(r.get("by-country").unwrap().deprecated);
        r.delete("by-country").unwrap();
        assert!(r.get("by-country").is_none());
    }

    #[test]
    fn delete_builtin_rejected() {
        let r = CustomStrategyRegistry::new();
        assert!(matches!(
            r.delete("default"),
            Err(CustomStrategyError::BuiltinReserved(_))
        ));
    }

    #[test]
    fn is_known_covers_builtin_and_custom() {
        let r = CustomStrategyRegistry::new();
        assert!(r.is_known("default"));
        assert!(!r.is_known("by-country"));
        r.register(mk("by-country")).unwrap();
        assert!(r.is_known("by-country"));
    }

    #[test]
    fn list_is_alphabetical() {
        let r = CustomStrategyRegistry::new();
        r.register(mk("zulu")).unwrap();
        r.register(mk("alpha")).unwrap();
        r.register(mk("mike")).unwrap();
        let names: Vec<String> = r.list().into_iter().map(|s| s.name).collect();
        assert_eq!(names, vec!["alpha", "mike", "zulu"]);
    }
}
