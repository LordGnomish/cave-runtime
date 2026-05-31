// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! falcoctl artifact index + reference resolution.
//!
//! NOTICE: upstream is falcosecurity/falcoctl v0.13.0 (Apache-2.0),
//! `pkg/index/index/index.go` (`Entry`, `Index`, `MergedIndexes::ResolveReference`,
//! `parseIndexRef`). falcoctl is the Falco artifact manager: it reads
//! `index.yaml` catalogues of rulesfiles/plugins and resolves a short
//! artifact name (e.g. `cloudtrail:0.5.1`) into a full OCI reference
//! (`ghcr.io/falcosecurity/plugins/cloudtrail:0.5.1`).
//!
//! This is the pure-userspace metadata surface — the OCI pull/push transport
//! (oras registry I/O) is a network side-effect handled out-of-process per
//! ADR-RUNTIME-SANDBOX-NO-FFI-001.

use crate::error::{FalcoError, Result};
use serde::{Deserialize, Serialize};

/// `oci.DefaultTag` — the tag appended when an artifact reference omits one.
pub const DEFAULT_TAG: &str = "latest";

/// One catalogue entry in a falcoctl `index.yaml` (`index.Entry`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexEntry {
    pub name: String,
    #[serde(rename = "type", default)]
    pub artifact_type: String,
    pub registry: String,
    pub repository: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub home: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub sources: Vec<String>,
}

/// A falcoctl artifact index (`index.Index`). Entry order is preserved on
/// insert; `entry_by_name` is the catalogue lookup.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Index {
    name: String,
    entries: Vec<IndexEntry>,
}

impl Index {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), entries: Vec::new() }
    }

    /// Parse an `index.yaml` document (a YAML sequence of entries).
    pub fn from_yaml(name: impl Into<String>, yaml: &str) -> Result<Self> {
        let entries: Vec<IndexEntry> = serde_yaml::from_str(yaml)?;
        Ok(Self { name: name.into(), entries })
    }

    pub fn name(&self) -> &str { &self.name }
    pub fn entries(&self) -> &[IndexEntry] { &self.entries }

    /// `Index.Upsert` — update the entry with the same name in place, or
    /// append it if new.
    pub fn upsert(&mut self, entry: IndexEntry) {
        if let Some(slot) = self.entries.iter_mut().find(|e| e.name == entry.name) {
            *slot = entry;
        } else {
            self.entries.push(entry);
        }
    }

    /// `Index.Remove` — remove the named entry; error if it is not present.
    pub fn remove(&mut self, name: &str) -> Result<()> {
        let before = self.entries.len();
        self.entries.retain(|e| e.name != name);
        if self.entries.len() == before {
            return Err(FalcoError::NotFound(format!("cannot remove {name}: not found")));
        }
        Ok(())
    }

    /// `Index.EntryByName`.
    pub fn entry_by_name(&self, name: &str) -> Option<&IndexEntry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// `Index.Normalize` — canonical form: entries sorted by name ascending.
    pub fn normalize(&mut self) {
        self.entries.sort_by(|a, b| a.name.cmp(&b.name));
    }

    /// `MergedIndexes.ResolveReference` — resolve `name` into a full OCI
    /// reference:
    ///
    /// 1. A bare artifact name (`cloudtrail`, `cloudtrail:0.5.1`,
    ///    `cloudtrail@sha256:...`) is looked up in the index and expanded to
    ///    `{registry}/{repository}` with `:latest` / `:tag` / `@digest`.
    /// 2. A full reference without tag or digest gets `:latest` appended.
    /// 3. A complete reference is returned as-is.
    pub fn resolve_reference(&self, name: &str) -> Result<String> {
        if looks_like_full_reference(name) {
            if full_ref_has_tag_or_digest(name) {
                Ok(name.to_string())
            } else {
                Ok(format!("{name}:{DEFAULT_TAG}"))
            }
        } else {
            let (entry_name, tag, digest) = parse_index_ref(name)?;
            let entry = self.entry_by_name(&entry_name).ok_or_else(|| {
                FalcoError::NotFound(format!(
                    "cannot find {name} among the configured indexes, skipping"
                ))
            })?;
            let mut reference = format!("{}/{}", entry.registry, entry.repository);
            if !tag.is_empty() {
                reference.push(':');
                reference.push_str(&tag);
            } else if !digest.is_empty() {
                reference.push('@');
                reference.push_str(&digest);
            } else {
                reference.push(':');
                reference.push_str(DEFAULT_TAG);
            }
            Ok(reference)
        }
    }
}

/// Heuristic for oras `registry.ParseReference` success: the string has a
/// `/` and its first segment is a registry host (contains `.`/`:`, or is
/// `localhost`). A bare `cloudtrail` or `cloudtrail:0.5.1` has no host and is
/// treated as an index name.
fn looks_like_full_reference(name: &str) -> bool {
    match name.split_once('/') {
        Some((registry, _)) => {
            registry == "localhost" || registry.contains('.') || registry.contains(':')
        }
        None => false,
    }
}

/// A full reference carries a tag/digest if it contains `@`, or the final
/// path component (after the last `/`) contains `:` (a registry port colon
/// lives in the first segment, never the last).
fn full_ref_has_tag_or_digest(name: &str) -> bool {
    if name.contains('@') {
        return true;
    }
    name.rsplit('/').next().map(|last| last.contains(':')).unwrap_or(false)
}

/// `parseIndexRef` — split a bare artifact name into (name, tag, digest).
fn parse_index_ref(name: &str) -> Result<(String, String, String)> {
    if !name.contains(':') && !name.contains('@') {
        Ok((name.to_string(), String::new(), String::new()))
    } else if name.contains(':') && !name.contains('@') {
        let (n, tag) = name.split_once(':').unwrap();
        Ok((n.to_string(), tag.to_string(), String::new()))
    } else if name.contains('@') {
        let (n, digest) = name.split_once('@').unwrap();
        Ok((n.to_string(), String::new(), digest.to_string()))
    } else {
        Err(FalcoError::RuleParse(format!("cannot parse {name:?}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, repo: &str) -> IndexEntry {
        IndexEntry {
            name: name.into(),
            artifact_type: "plugin".into(),
            registry: "ghcr.io".into(),
            repository: repo.into(),
            description: String::new(),
            home: String::new(),
            keywords: vec![],
            license: "apache-2.0".into(),
            sources: vec![],
        }
    }

    #[test]
    fn parses_index_yaml() {
        let y = r#"
- name: cloudtrail
  type: plugin
  registry: ghcr.io
  repository: falcosecurity/plugins/cloudtrail
  description: AWS CloudTrail plugin
  keywords: [aws, cloudtrail]
  license: apache-2.0
  sources: ["https://github.com/falcosecurity/plugins"]
"#;
        let idx = Index::from_yaml("falcosecurity", y).unwrap();
        assert_eq!(idx.entries().len(), 1);
        assert_eq!(idx.entries()[0].repository, "falcosecurity/plugins/cloudtrail");
        assert_eq!(idx.entries()[0].keywords, vec!["aws".to_string(), "cloudtrail".to_string()]);
    }

    #[test]
    fn upsert_appends_new_and_updates_existing() {
        let mut idx = Index::new("test");
        idx.upsert(entry("a", "falcosecurity/a"));
        idx.upsert(entry("b", "falcosecurity/b"));
        assert_eq!(idx.entries().len(), 2);
        // update "a" in place — count stays 2, repo changes
        idx.upsert(entry("a", "falcosecurity/a-v2"));
        assert_eq!(idx.entries().len(), 2);
        assert_eq!(idx.entry_by_name("a").unwrap().repository, "falcosecurity/a-v2");
    }

    #[test]
    fn remove_existing_and_missing() {
        let mut idx = Index::new("test");
        idx.upsert(entry("a", "falcosecurity/a"));
        assert!(idx.remove("a").is_ok());
        assert_eq!(idx.entries().len(), 0);
        assert!(idx.remove("a").is_err());
    }

    #[test]
    fn entry_by_name_found_and_absent() {
        let mut idx = Index::new("test");
        idx.upsert(entry("cloudtrail", "falcosecurity/plugins/cloudtrail"));
        assert!(idx.entry_by_name("cloudtrail").is_some());
        assert!(idx.entry_by_name("nope").is_none());
    }

    #[test]
    fn normalize_sorts_by_name() {
        let mut idx = Index::new("test");
        idx.upsert(entry("zeta", "x/zeta"));
        idx.upsert(entry("alpha", "x/alpha"));
        idx.upsert(entry("mu", "x/mu"));
        idx.normalize();
        let names: Vec<_> = idx.entries().iter().map(|e| e.name.clone()).collect();
        assert_eq!(names, vec!["alpha".to_string(), "mu".to_string(), "zeta".to_string()]);
    }

    fn sample() -> Index {
        let mut idx = Index::new("falcosecurity");
        idx.upsert(entry("cloudtrail", "falcosecurity/plugins/cloudtrail"));
        idx
    }

    #[test]
    fn resolve_bare_name_appends_latest() {
        let idx = sample();
        assert_eq!(
            idx.resolve_reference("cloudtrail").unwrap(),
            "ghcr.io/falcosecurity/plugins/cloudtrail:latest"
        );
    }

    #[test]
    fn resolve_name_with_tag() {
        let idx = sample();
        assert_eq!(
            idx.resolve_reference("cloudtrail:0.5.1").unwrap(),
            "ghcr.io/falcosecurity/plugins/cloudtrail:0.5.1"
        );
    }

    #[test]
    fn resolve_name_with_digest() {
        let idx = sample();
        assert_eq!(
            idx.resolve_reference("cloudtrail@sha256:abc123").unwrap(),
            "ghcr.io/falcosecurity/plugins/cloudtrail@sha256:abc123"
        );
    }

    #[test]
    fn resolve_full_ref_without_tag_appends_latest() {
        let idx = sample();
        assert_eq!(
            idx.resolve_reference("ghcr.io/falcosecurity/plugins/cloudtrail").unwrap(),
            "ghcr.io/falcosecurity/plugins/cloudtrail:latest"
        );
    }

    #[test]
    fn resolve_full_ref_with_tag_is_unchanged() {
        let idx = sample();
        assert_eq!(
            idx.resolve_reference("ghcr.io/falcosecurity/plugins/cloudtrail:1.2.3").unwrap(),
            "ghcr.io/falcosecurity/plugins/cloudtrail:1.2.3"
        );
    }

    #[test]
    fn resolve_unknown_index_name_errors() {
        let idx = sample();
        assert!(idx.resolve_reference("doesnotexist").is_err());
    }
}
