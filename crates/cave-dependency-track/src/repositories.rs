// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Repository metadata + latest-version resolver.
//!
//! Mirrors `org.dependencytrack.tasks.repositories.*` — Maven Central,
//! npmjs, NuGet, PyPI, RubyGems, Cargo, Go, Composer, Hex, etc.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum RepositoryType {
    Maven,
    Npm,
    Nuget,
    Pypi,
    Gem,
    Cargo,
    Composer,
    Hex,
    Go,
    Generic,
}

impl RepositoryType {
    pub fn from_purl_type(t: &str) -> Self {
        match t.to_ascii_lowercase().as_str() {
            "maven" => Self::Maven,
            "npm" => Self::Npm,
            "nuget" => Self::Nuget,
            "pypi" => Self::Pypi,
            "gem" => Self::Gem,
            "cargo" => Self::Cargo,
            "composer" => Self::Composer,
            "hex" => Self::Hex,
            "golang" => Self::Go,
            _ => Self::Generic,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Repository {
    pub r#type: RepositoryType,
    pub identifier: String,
    pub url: String,
    pub enabled: bool,
    pub priority: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepositoryMetaComponent {
    pub r#type: RepositoryType,
    pub namespace: Option<String>,
    pub name: String,
    pub latest_version: Option<String>,
    pub published: Option<String>,
}

#[derive(Default)]
pub struct RepositoryStore {
    repositories: RwLock<Vec<Repository>>,
    meta: RwLock<HashMap<(RepositoryType, Option<String>, String), RepositoryMetaComponent>>,
}

impl RepositoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put(&self, r: Repository) {
        self.repositories.write().unwrap().push(r);
    }

    pub fn list(&self) -> Vec<Repository> {
        let mut v = self.repositories.read().unwrap().clone();
        v.sort_by_key(|r| -r.priority);
        v
    }

    pub fn list_enabled(&self, t: RepositoryType) -> Vec<Repository> {
        self.list()
            .into_iter()
            .filter(|r| r.r#type == t && r.enabled)
            .collect()
    }

    pub fn upsert_meta(&self, m: RepositoryMetaComponent) {
        let key = (m.r#type, m.namespace.clone(), m.name.clone());
        self.meta.write().unwrap().insert(key, m);
    }

    pub fn get_latest(
        &self,
        t: RepositoryType,
        namespace: Option<&str>,
        name: &str,
    ) -> Option<RepositoryMetaComponent> {
        self.meta
            .read()
            .unwrap()
            .get(&(t, namespace.map(String::from), name.to_string()))
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_resolves_from_purl_type() {
        assert_eq!(RepositoryType::from_purl_type("cargo"), RepositoryType::Cargo);
        assert_eq!(RepositoryType::from_purl_type("NPM"), RepositoryType::Npm);
        assert_eq!(RepositoryType::from_purl_type("golang"), RepositoryType::Go);
        assert_eq!(RepositoryType::from_purl_type("rocket"), RepositoryType::Generic);
    }

    #[test]
    fn list_sorted_by_priority_descending() {
        let s = RepositoryStore::new();
        s.put(Repository {
            r#type: RepositoryType::Maven,
            identifier: "primary".into(),
            url: "https://a".into(),
            enabled: true,
            priority: 100,
        });
        s.put(Repository {
            r#type: RepositoryType::Maven,
            identifier: "mirror".into(),
            url: "https://b".into(),
            enabled: true,
            priority: 50,
        });
        let l = s.list();
        assert_eq!(l[0].identifier, "primary");
    }

    #[test]
    fn list_enabled_filters_type() {
        let s = RepositoryStore::new();
        s.put(Repository {
            r#type: RepositoryType::Maven,
            identifier: "a".into(),
            url: "".into(),
            enabled: false,
            priority: 0,
        });
        s.put(Repository {
            r#type: RepositoryType::Npm,
            identifier: "b".into(),
            url: "".into(),
            enabled: true,
            priority: 0,
        });
        assert_eq!(s.list_enabled(RepositoryType::Npm).len(), 1);
        assert_eq!(s.list_enabled(RepositoryType::Maven).len(), 0);
    }

    #[test]
    fn upsert_and_retrieve_meta() {
        let s = RepositoryStore::new();
        s.upsert_meta(RepositoryMetaComponent {
            r#type: RepositoryType::Cargo,
            namespace: None,
            name: "serde".into(),
            latest_version: Some("1.0.200".into()),
            published: None,
        });
        let m = s.get_latest(RepositoryType::Cargo, None, "serde").unwrap();
        assert_eq!(m.latest_version.as_deref(), Some("1.0.200"));
    }

    #[test]
    fn meta_namespace_disambiguates() {
        let s = RepositoryStore::new();
        s.upsert_meta(RepositoryMetaComponent {
            r#type: RepositoryType::Maven,
            namespace: Some("org.a".into()),
            name: "x".into(),
            latest_version: Some("1".into()),
            published: None,
        });
        s.upsert_meta(RepositoryMetaComponent {
            r#type: RepositoryType::Maven,
            namespace: Some("org.b".into()),
            name: "x".into(),
            latest_version: Some("2".into()),
            published: None,
        });
        assert_eq!(
            s.get_latest(RepositoryType::Maven, Some("org.b"), "x")
                .unwrap()
                .latest_version
                .as_deref(),
            Some("2")
        );
    }
}
