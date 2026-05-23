// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Function module — Function + FunctionRevision rollout + gRPC codec + builtins.
//!
//! Upstream: apis/pkg/v1beta1/function_types.go + proto/fn/v1/run_function.proto

pub mod auto_ready;
pub mod go_template;
pub mod grpc_codec;
pub mod kcl;
pub mod patch_transform;

use crate::error::{CrossplaneError, CrossplaneResult};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FunctionState {
    Installed,
    Updating,
    Unhealthy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Function {
    pub name: String,
    pub package: String,
    pub revision: String,
    pub state: FunctionState,
    pub installed_at: DateTime<Utc>,
}

#[derive(Default)]
pub struct FunctionStore {
    fns: DashMap<String, Function>,
}

impl FunctionStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn install(
        &self,
        name: &str,
        revision: &str,
        package: &str,
    ) -> CrossplaneResult<Function> {
        if name.is_empty() {
            return Err(CrossplaneError::Internal("function name empty".into()));
        }
        if self.fns.contains_key(name) {
            return Err(CrossplaneError::Internal(format!(
                "function already installed: {}",
                name
            )));
        }
        let f = Function {
            name: name.to_string(),
            package: package.to_string(),
            revision: revision.to_string(),
            state: FunctionState::Installed,
            installed_at: Utc::now(),
        };
        self.fns.insert(name.to_string(), f.clone());
        Ok(f)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.fns.contains_key(name)
    }

    pub fn get(&self, name: &str) -> Option<Function> {
        self.fns.get(name).map(|r| r.clone())
    }

    pub fn list(&self) -> Vec<Function> {
        self.fns.iter().map(|r| r.clone()).collect()
    }

    pub fn delete(&self, name: &str) -> CrossplaneResult<()> {
        self.fns
            .remove(name)
            .map(|_| ())
            .ok_or_else(|| CrossplaneError::Internal(format!("function not found: {}", name)))
    }

    pub fn set_state(&self, name: &str, state: FunctionState) -> CrossplaneResult<()> {
        match self.fns.get_mut(name) {
            Some(mut f) => {
                f.state = state;
                Ok(())
            }
            None => Err(CrossplaneError::Internal(format!(
                "function not found: {}",
                name
            ))),
        }
    }

    pub fn len(&self) -> usize {
        self.fns.len()
    }
    pub fn is_empty(&self) -> bool {
        self.fns.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_list_get() {
        let s = FunctionStore::new();
        s.install("function-x", "v0.1.0", "pkg").unwrap();
        assert_eq!(s.list().len(), 1);
        assert!(s.contains("function-x"));
        assert!(s.get("function-x").is_some());
    }

    #[test]
    fn duplicate_install_errors() {
        let s = FunctionStore::new();
        s.install("f", "v0.1.0", "p").unwrap();
        assert!(s.install("f", "v0.1.0", "p").is_err());
    }

    #[test]
    fn empty_name_errors() {
        let s = FunctionStore::new();
        assert!(s.install("", "v0.1.0", "p").is_err());
    }

    #[test]
    fn delete_removes() {
        let s = FunctionStore::new();
        s.install("f", "v0.1.0", "p").unwrap();
        s.delete("f").unwrap();
        assert!(!s.contains("f"));
    }

    #[test]
    fn set_state_unhealthy() {
        let s = FunctionStore::new();
        s.install("f", "v0.1.0", "p").unwrap();
        s.set_state("f", FunctionState::Unhealthy).unwrap();
        assert_eq!(s.get("f").unwrap().state, FunctionState::Unhealthy);
    }

    #[test]
    fn set_state_unknown_errors() {
        let s = FunctionStore::new();
        assert!(s.set_state("none", FunctionState::Updating).is_err());
    }
}
