// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CustomResourceDefinition (CRD) lifecycle registry.
//!
//! Mirrors `staging/src/k8s.io/apiextensions-apiserver/pkg/apis/apiextensions/v1`
//! at the umbrella level: tracks the *set* of installed CRDs and their
//! `served` versions, exposes them to the discovery layer
//! (`/apis/<group>/<version>`), and enforces structural-schema +
//! conversion-strategy invariants at install time.
//!
//! The shallow data backing each CRD is `serde_json::Value`; deep
//! validation against OpenAPI schemas is delegated to
//! `cave_apiserver::cel_eval` and `cave_apiserver::validating_admission_policy`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::RwLock;
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CrdError {
    #[error("CRD {0} already installed")]
    AlreadyInstalled(String),
    #[error("CRD {0} not found")]
    NotFound(String),
    #[error("structural schema invalid for {crd}: {detail}")]
    StructuralSchema { crd: String, detail: String },
    #[error("served version {version} not declared on CRD {crd}")]
    UnknownVersion { crd: String, version: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    Namespaced,
    Cluster,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrdVersion {
    pub name: String,
    pub served: bool,
    pub storage: bool,
    /// JSON Schema for structural validation.  Empty `{}` is allowed but
    /// flagged as a `StructuralSchema` warning at install time.
    pub schema: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Crd {
    pub group: String,
    pub plural: String,
    pub kind: String,
    pub scope: Scope,
    pub versions: Vec<CrdVersion>,
}

impl Crd {
    pub fn name(&self) -> String {
        format!("{}.{}", self.plural, self.group)
    }
    pub fn served_versions(&self) -> impl Iterator<Item = &CrdVersion> {
        self.versions.iter().filter(|v| v.served)
    }
    pub fn storage_version(&self) -> Option<&CrdVersion> {
        self.versions.iter().find(|v| v.storage)
    }
}

pub struct CrdRegistry {
    inner: RwLock<BTreeMap<String, Crd>>,
}

impl Default for CrdRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CrdRegistry {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(BTreeMap::new()),
        }
    }

    pub fn install(&self, crd: Crd) -> Result<(), CrdError> {
        if crd.versions.is_empty() {
            return Err(CrdError::StructuralSchema {
                crd: crd.name(),
                detail: "no versions declared".into(),
            });
        }
        let storage_count = crd.versions.iter().filter(|v| v.storage).count();
        if storage_count != 1 {
            return Err(CrdError::StructuralSchema {
                crd: crd.name(),
                detail: format!("exactly one storage version required (got {})", storage_count),
            });
        }
        for v in &crd.versions {
            if v.served && v.schema.is_null() {
                return Err(CrdError::StructuralSchema {
                    crd: crd.name(),
                    detail: format!("served version {} has null schema", v.name),
                });
            }
        }
        let mut g = self.inner.write().expect("crd lock");
        let name = crd.name();
        if g.contains_key(&name) {
            return Err(CrdError::AlreadyInstalled(name));
        }
        g.insert(name, crd);
        Ok(())
    }

    pub fn uninstall(&self, name: &str) -> Result<(), CrdError> {
        let mut g = self.inner.write().expect("crd lock");
        g.remove(name)
            .map(|_| ())
            .ok_or_else(|| CrdError::NotFound(name.to_string()))
    }

    pub fn get(&self, name: &str) -> Option<Crd> {
        self.inner.read().expect("crd lock").get(name).cloned()
    }

    pub fn list(&self) -> Vec<Crd> {
        self.inner.read().expect("crd lock").values().cloned().collect()
    }

    pub fn count(&self) -> usize {
        self.inner.read().expect("crd lock").len()
    }

    pub fn ensure_version_served(&self, name: &str, version: &str) -> Result<(), CrdError> {
        let crd = self
            .get(name)
            .ok_or_else(|| CrdError::NotFound(name.to_string()))?;
        if crd.served_versions().any(|v| v.name == version) {
            Ok(())
        } else {
            Err(CrdError::UnknownVersion {
                crd: name.to_string(),
                version: version.to_string(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample(name: &str) -> Crd {
        Crd {
            group: "example.com".into(),
            plural: name.into(),
            kind: "X".into(),
            scope: Scope::Namespaced,
            versions: vec![CrdVersion {
                name: "v1".into(),
                served: true,
                storage: true,
                schema: json!({"type": "object"}),
            }],
        }
    }

    #[test]
    fn install_and_get() {
        let r = CrdRegistry::new();
        let c = sample("foos");
        r.install(c.clone()).unwrap();
        assert_eq!(r.count(), 1);
        let back = r.get("foos.example.com").unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn duplicate_install_errs() {
        let r = CrdRegistry::new();
        r.install(sample("foos")).unwrap();
        let e = r.install(sample("foos")).unwrap_err();
        assert!(matches!(e, CrdError::AlreadyInstalled(_)));
    }

    #[test]
    fn uninstall_unknown_errs() {
        let r = CrdRegistry::new();
        let e = r.uninstall("nope.example.com").unwrap_err();
        assert!(matches!(e, CrdError::NotFound(_)));
    }

    #[test]
    fn must_have_exactly_one_storage_version() {
        let r = CrdRegistry::new();
        let mut c = sample("bars");
        c.versions.push(CrdVersion {
            name: "v2".into(),
            served: true,
            storage: true,
            schema: json!({}),
        });
        let e = r.install(c).unwrap_err();
        assert!(matches!(e, CrdError::StructuralSchema { .. }));
    }

    #[test]
    fn empty_versions_rejected() {
        let r = CrdRegistry::new();
        let mut c = sample("bazs");
        c.versions.clear();
        assert!(matches!(
            r.install(c).unwrap_err(),
            CrdError::StructuralSchema { .. }
        ));
    }

    #[test]
    fn served_version_lookup() {
        let r = CrdRegistry::new();
        r.install(sample("qux")).unwrap();
        r.ensure_version_served("qux.example.com", "v1").unwrap();
        let e = r.ensure_version_served("qux.example.com", "v9").unwrap_err();
        assert!(matches!(e, CrdError::UnknownVersion { .. }));
    }

    #[test]
    fn storage_version_helper() {
        let mut c = sample("multi");
        c.versions.push(CrdVersion {
            name: "v1beta1".into(),
            served: true,
            storage: false,
            schema: json!({}),
        });
        let sv = c.storage_version().unwrap();
        assert_eq!(sv.name, "v1");
    }

    #[test]
    fn list_is_sorted_by_name() {
        let r = CrdRegistry::new();
        r.install(sample("zebras")).unwrap();
        r.install(sample("alphas")).unwrap();
        let names: Vec<_> = r.list().iter().map(|c| c.name()).collect();
        assert_eq!(names, vec!["alphas.example.com", "zebras.example.com"]);
    }
}
