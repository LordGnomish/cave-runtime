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
