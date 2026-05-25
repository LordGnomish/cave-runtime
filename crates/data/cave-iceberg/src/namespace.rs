// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Namespace identifier — Iceberg `Namespace` + `NamespaceIdent`.
//!
//! Upstream: `crates/iceberg/src/namespace.rs`
//! Spec: Iceberg REST `Namespace` (a list of levels).

use serde::{Deserialize, Serialize};

/// A hierarchical namespace identifier (e.g. `["analytics", "raw"]`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct NamespaceIdent(pub Vec<String>);

impl NamespaceIdent {
    pub fn new<I, S>(levels: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self(levels.into_iter().map(Into::into).collect())
    }

    pub fn from_dot(dotted: &str) -> Self {
        Self(dotted.split('.').map(str::to_string).collect())
    }

    pub fn as_dot(&self) -> String {
        self.0.join(".")
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Namespace {
    pub ident: NamespaceIdent,
    pub properties: std::collections::HashMap<String, String>,
}

impl Namespace {
    pub fn new(ident: NamespaceIdent) -> Self {
        Self {
            ident,
            properties: Default::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_roundtrip() {
        let n = NamespaceIdent::from_dot("analytics.raw");
        assert_eq!(n.as_dot(), "analytics.raw");
        assert_eq!(n.0, vec!["analytics", "raw"]);
    }

    #[test]
    fn root_is_empty_vec() {
        let n = NamespaceIdent::new(Vec::<&str>::new());
        assert!(n.is_root());
    }

    #[test]
    fn namespace_carries_properties() {
        let mut n = Namespace::new(NamespaceIdent::from_dot("x"));
        n.properties.insert("owner".into(), "team".into());
        assert_eq!(n.properties.get("owner"), Some(&"team".to_string()));
    }
}
