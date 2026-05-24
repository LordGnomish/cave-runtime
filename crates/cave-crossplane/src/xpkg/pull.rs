// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! XPKG package pull — offline OCI layout reader + manifest digest verify.
//! Live HTTP registry pull is delegated to cave-artifacts (Phase 2).
//!
//! Upstream: internal/xpkg/fetch/fetch.go + internal/xpkg/fetch/k8s.go

use crate::error::{CrossplaneError, CrossplaneResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PackageKind {
    Configuration,
    Provider,
    Function,
}

impl PackageKind {
    pub fn from_meta_kind(s: &str) -> Self {
        match s {
            "Provider" => PackageKind::Provider,
            "Function" => PackageKind::Function,
            _ => PackageKind::Configuration,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageBundle {
    pub name: String,
    pub kind: PackageKind,
    pub digest: String,
    /// Embedded yaml documents (one per top-level CRD / Composition / Function manifest).
    pub manifests: Vec<String>,
}

impl PackageBundle {
    pub fn empty(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: PackageKind::Configuration,
            digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                .to_string(),
            manifests: Vec::new(),
        }
    }
}

/// Pull from an offline OCI layout directory. Expects:
///   `<dir>/manifest.yaml`  (the package meta)
///   `<dir>/*.yaml`         (CRD / Composition / Function manifests)
pub fn pull_offline(path: impl AsRef<Path>) -> CrossplaneResult<PackageBundle> {
    let path = path.as_ref().to_path_buf();
    if !path.exists() {
        return Err(CrossplaneError::Internal(format!(
            "xpkg path not found: {}",
            path.display()
        )));
    }
    let meta_path = path.join("manifest.yaml");
    let (name, kind) = if meta_path.exists() {
        let meta = fs::read_to_string(&meta_path)
            .map_err(|e| CrossplaneError::Internal(format!("read manifest.yaml: {}", e)))?;
        parse_meta(&meta)
    } else {
        let basename = path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        (basename, PackageKind::Configuration)
    };

    let mut manifests: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(&path) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_file() {
                let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
                if ext == "yaml" && p.file_name() != Some(std::ffi::OsStr::new("manifest.yaml")) {
                    if let Ok(s) = fs::read_to_string(&p) {
                        manifests.push(s);
                    }
                }
            }
        }
    }
    manifests.sort(); // stable ordering for reproducible digests

    let digest = compute_digest(&name, &kind, &manifests);
    Ok(PackageBundle {
        name,
        kind,
        digest,
        manifests,
    })
}

/// Verify the digest matches.
pub fn verify_digest(bundle: &PackageBundle) -> bool {
    bundle.digest == compute_digest(&bundle.name, &bundle.kind, &bundle.manifests)
}

fn parse_meta(yaml: &str) -> (String, PackageKind) {
    let mut name = "unknown".to_string();
    let mut kind = PackageKind::Configuration;
    for line in yaml.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("kind:") {
            kind = PackageKind::from_meta_kind(rest.trim());
        }
        if let Some(rest) = t.strip_prefix("name:") {
            name = rest.trim().trim_matches('"').to_string();
        }
    }
    (name, kind)
}

fn compute_digest(name: &str, kind: &PackageKind, manifests: &[String]) -> String {
    let mut h = Sha256::new();
    h.update(name.as_bytes());
    h.update(b":");
    h.update(format!("{:?}", kind).as_bytes());
    h.update(b":");
    for m in manifests {
        h.update(m.as_bytes());
    }
    format!("sha256:{}", hex::encode(h.finalize()))
}

/// Helper: write a minimal fixture xpkg layout to `dir`.
pub fn write_fixture_xpkg(
    dir: impl AsRef<Path>,
    name: &str,
    kind: PackageKind,
    manifests: &[&str],
) -> CrossplaneResult<PathBuf> {
    let dir = dir.as_ref().to_path_buf();
    fs::create_dir_all(&dir).map_err(|e| CrossplaneError::Internal(e.to_string()))?;
    let kind_str = match kind {
        PackageKind::Configuration => "Configuration",
        PackageKind::Provider => "Provider",
        PackageKind::Function => "Function",
    };
    fs::write(
        dir.join("manifest.yaml"),
        format!("apiVersion: meta.pkg.crossplane.io/v1\nkind: {}\nname: \"{}\"\n", kind_str, name),
    )
    .map_err(|e| CrossplaneError::Internal(e.to_string()))?;
    for (i, m) in manifests.iter().enumerate() {
        fs::write(dir.join(format!("manifest-{:03}.yaml", i)), m)
            .map_err(|e| CrossplaneError::Internal(e.to_string()))?;
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_bundle_construct() {
        let b = PackageBundle::empty("p");
        assert_eq!(b.name, "p");
        assert_eq!(b.kind, PackageKind::Configuration);
    }

    #[test]
    fn pull_nonexistent_errors() {
        assert!(pull_offline("/tmp/__never_should_exist_xpkg").is_err());
    }

    #[test]
    fn pull_fixture_roundtrip() {
        let tmp = std::env::temp_dir().join(format!(
            "cave-xpkg-test-{}",
            uuid::Uuid::new_v4()
        ));
        write_fixture_xpkg(
            &tmp,
            "pkg-a",
            PackageKind::Configuration,
            &["apiVersion: x/v1\nkind: CRD\n"],
        )
        .unwrap();
        let b = pull_offline(&tmp).unwrap();
        assert_eq!(b.name, "pkg-a");
        assert_eq!(b.kind, PackageKind::Configuration);
        assert_eq!(b.manifests.len(), 1);
        assert!(b.digest.starts_with("sha256:"));
        assert!(verify_digest(&b));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn provider_kind_parsed() {
        let tmp = std::env::temp_dir().join(format!(
            "cave-xpkg-prov-{}",
            uuid::Uuid::new_v4()
        ));
        write_fixture_xpkg(&tmp, "provider-x", PackageKind::Provider, &[]).unwrap();
        let b = pull_offline(&tmp).unwrap();
        assert_eq!(b.kind, PackageKind::Provider);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn function_kind_parsed() {
        let tmp = std::env::temp_dir().join(format!(
            "cave-xpkg-fn-{}",
            uuid::Uuid::new_v4()
        ));
        write_fixture_xpkg(&tmp, "function-x", PackageKind::Function, &[]).unwrap();
        let b = pull_offline(&tmp).unwrap();
        assert_eq!(b.kind, PackageKind::Function);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn verify_digest_mismatch_detected() {
        let mut b = PackageBundle::empty("p");
        b.manifests.push("hi".to_string());
        assert!(!verify_digest(&b));
    }

    #[test]
    fn package_kind_from_meta() {
        assert_eq!(PackageKind::from_meta_kind("Provider"), PackageKind::Provider);
        assert_eq!(PackageKind::from_meta_kind("Function"), PackageKind::Function);
        assert_eq!(
            PackageKind::from_meta_kind("Configuration"),
            PackageKind::Configuration
        );
    }

    #[test]
    fn parse_meta_extracts_name() {
        let (n, _) = parse_meta("kind: Configuration\nname: \"foo\"\n");
        assert_eq!(n, "foo");
    }

    #[test]
    fn pull_dir_without_manifest_yaml_uses_basename() {
        let tmp = std::env::temp_dir().join(format!(
            "cave-xpkg-nm-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("a.yaml"), "x: 1").unwrap();
        let b = pull_offline(&tmp).unwrap();
        assert!(b.name.contains("cave-xpkg-nm-"));
        let _ = fs::remove_dir_all(&tmp);
    }
}
