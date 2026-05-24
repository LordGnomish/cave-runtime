// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! XRD spec model + parse + emit.
//!
//! Upstream: apis/apiextensions/v1/xrd_types.go

use crate::models::XrdScope;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XrdSpec {
    pub group: String,
    pub names: XrdNames,
    pub claim_names: Option<XrdNames>,
    pub scope: XrdScope,
    pub versions: Vec<XrdSpecVersion>,
    pub default_composition_ref: Option<String>,
    pub enforced_composition_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct XrdNames {
    pub kind: String,
    pub plural: String,
    pub list_kind: String,
    pub singular: String,
}

impl XrdNames {
    pub fn from_kind(kind: &str) -> Self {
        let plural = format!("{}s", kind.to_lowercase());
        let list_kind = format!("{}List", kind);
        let singular = kind.to_lowercase();
        Self {
            kind: kind.to_string(),
            plural,
            list_kind,
            singular,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct XrdSpecVersion {
    pub name: String,
    pub served: bool,
    pub referenceable: bool,
    pub schema: Option<Value>,
}

impl XrdSpec {
    pub fn new(group: impl Into<String>, kind: impl Into<String>, scope: XrdScope) -> Self {
        let kind = kind.into();
        Self {
            group: group.into(),
            names: XrdNames::from_kind(&kind),
            claim_names: None,
            scope,
            versions: vec![XrdSpecVersion {
                name: "v1".into(),
                served: true,
                referenceable: true,
                schema: None,
            }],
            default_composition_ref: None,
            enforced_composition_ref: None,
        }
    }

    pub fn list_kind(&self) -> String {
        self.names.list_kind.clone()
    }

    /// The api-version for the v1 served+referenceable version.
    pub fn api_version_v1(&self) -> String {
        format!("{}/v1", self.group)
    }

    /// Return the referenceable version, falling back to first served.
    pub fn referenceable_version(&self) -> Option<&XrdSpecVersion> {
        self.versions
            .iter()
            .find(|v| v.referenceable)
            .or_else(|| self.versions.iter().find(|v| v.served))
    }

    pub fn with_claim_names(mut self, names: XrdNames) -> Self {
        self.claim_names = Some(names);
        self
    }

    pub fn with_version(mut self, version: XrdSpecVersion) -> Self {
        self.versions.push(version);
        self
    }

    pub fn with_default_composition(mut self, name: impl Into<String>) -> Self {
        self.default_composition_ref = Some(name.into());
        self
    }
}

/// Serialise to upstream-shape Kubernetes JSON.
pub fn to_k8s_json(s: &XrdSpec) -> Value {
    serde_json::json!({
        "apiVersion": "apiextensions.crossplane.io/v2",
        "kind": "CompositeResourceDefinition",
        "metadata": {"name": format!("{}.{}", s.names.plural, s.group)},
        "spec": {
            "group": s.group,
            "names": {
                "kind": s.names.kind,
                "plural": s.names.plural,
                "listKind": s.names.list_kind,
                "singular": s.names.singular,
            },
            "claimNames": s.claim_names.as_ref().map(|n| serde_json::json!({
                "kind": n.kind,
                "plural": n.plural,
                "listKind": n.list_kind,
                "singular": n.singular,
            })),
            "scope": match s.scope { XrdScope::Cluster => "Cluster", XrdScope::Namespaced => "Namespaced" },
            "versions": s.versions,
            "defaultCompositionRef": s.default_composition_ref.as_ref().map(|n| serde_json::json!({"name": n})),
            "enforcedCompositionRef": s.enforced_composition_ref.as_ref().map(|n| serde_json::json!({"name": n})),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_seeds_v1() {
        let s = XrdSpec::new("ex.cave.io", "XDb", XrdScope::Cluster);
        assert_eq!(s.versions[0].name, "v1");
        assert!(s.versions[0].referenceable);
        assert_eq!(s.list_kind(), "XDbList");
    }

    #[test]
    fn api_version_v1_format() {
        let s = XrdSpec::new("ex.cave.io", "XDb", XrdScope::Cluster);
        assert_eq!(s.api_version_v1(), "ex.cave.io/v1");
    }

    #[test]
    fn referenceable_version_picks_correctly() {
        let mut s = XrdSpec::new("g", "K", XrdScope::Cluster);
        s.versions[0].referenceable = false;
        s.versions[0].served = true;
        assert_eq!(s.referenceable_version().unwrap().name, "v1");
    }

    #[test]
    fn xrd_names_from_kind() {
        let n = XrdNames::from_kind("Database");
        assert_eq!(n.plural, "databases");
        assert_eq!(n.list_kind, "DatabaseList");
        assert_eq!(n.singular, "database");
    }

    #[test]
    fn with_claim_names_attaches() {
        let s = XrdSpec::new("g", "X", XrdScope::Cluster)
            .with_claim_names(XrdNames::from_kind("XC"));
        assert!(s.claim_names.is_some());
    }

    #[test]
    fn with_default_composition_attaches() {
        let s = XrdSpec::new("g", "X", XrdScope::Cluster).with_default_composition("comp-a");
        assert_eq!(s.default_composition_ref.as_deref(), Some("comp-a"));
    }

    #[test]
    fn to_k8s_json_shape() {
        let s = XrdSpec::new("ex.cave.io", "XDb", XrdScope::Cluster);
        let j = to_k8s_json(&s);
        assert_eq!(j["kind"], serde_json::json!("CompositeResourceDefinition"));
        assert_eq!(j["spec"]["group"], serde_json::json!("ex.cave.io"));
        assert_eq!(j["spec"]["scope"], serde_json::json!("Cluster"));
    }

    #[test]
    fn with_version_appends() {
        let s = XrdSpec::new("g", "X", XrdScope::Cluster).with_version(XrdSpecVersion {
            name: "v2".into(),
            served: true,
            referenceable: false,
            schema: None,
        });
        assert_eq!(s.versions.len(), 2);
    }
}
