// SPDX-License-Identifier: AGPL-3.0-or-later
//! Workspace-wide parity manifest discovery.
//!
//! Walks `{workspace_root}/crates/*/parity.manifest.toml`, parses each manifest,
//! and runs the calculator against that crate's source tree. Used by the portal
//! to populate the parity cache with every module that ships a manifest — no
//! hard-coded whitelist.

use super::calculator::{FsResolver, ParityCalculator};
use super::manifest::ParityManifest;
use super::types::ParityReport;
use std::path::{Path, PathBuf};

/// One discovery result. `manifest_path` is the absolute path to the manifest
/// that produced this report, useful for logging / cache invalidation.
#[derive(Debug, Clone)]
pub struct DiscoveredReport {
    pub manifest_path: PathBuf,
    pub report: ParityReport,
}

/// Walk `{workspace_root}/crates/*/parity.manifest.toml` and run the calculator
/// against each crate. Returns one `DiscoveredReport` per successfully-parsed
/// manifest. Crates with no manifest are silently skipped; crates with a
/// malformed manifest are skipped (the parse error is dropped — callers that
/// want to surface parse failures should iterate manifests directly).
pub fn discover_workspace(workspace_root: impl AsRef<Path>) -> Vec<DiscoveredReport> {
    let crates_dir = workspace_root.as_ref().join("crates");
    let Ok(entries) = std::fs::read_dir(&crates_dir) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let crate_root = entry.path();
        if !crate_root.is_dir() {
            continue;
        }
        let manifest_path = crate_root.join("parity.manifest.toml");
        if !manifest_path.exists() {
            continue;
        }
        let Ok(toml_str) = std::fs::read_to_string(&manifest_path) else {
            continue;
        };
        let Ok(manifest) = toml::from_str::<ParityManifest>(&toml_str) else {
            continue;
        };
        let resolver = FsResolver::new(&crate_root);
        let report = ParityCalculator::new(resolver).calculate(&manifest);
        out.push(DiscoveredReport {
            manifest_path,
            report,
        });
    }

    out.sort_by(|a, b| a.report.module.cmp(&b.report.module));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_manifest(crate_dir: &Path, module_name: &str) {
        fs::create_dir_all(crate_dir.join("src")).unwrap();
        fs::write(
            crate_dir.join("src").join("lib.rs"),
            "pub fn placeholder() {}\n",
        )
        .unwrap();
        let toml = format!(
            r#"
[upstream]
org = "u-org"
repo = "u-repo"
version = "v0.1.0"

[module]
name = "{module_name}"
description = "test"
source_root = "src"
"#
        );
        fs::write(crate_dir.join("parity.manifest.toml"), toml).unwrap();
    }

    #[test]
    fn discover_finds_all_manifests() {
        let tmp = tempdir();
        let crates = tmp.join("crates");
        fs::create_dir_all(&crates).unwrap();
        write_manifest(&crates.join("cave-foo"), "foo");
        write_manifest(&crates.join("cave-bar"), "bar");
        // crate without a manifest — must be skipped
        fs::create_dir_all(crates.join("cave-noparity").join("src")).unwrap();

        let results = discover_workspace(&tmp);
        assert_eq!(results.len(), 2);
        let names: Vec<_> = results.iter().map(|r| r.report.module.as_str()).collect();
        assert_eq!(names, vec!["bar", "foo"]); // sorted
        cleanup(&tmp);
    }

    #[test]
    fn discover_skips_malformed_manifest() {
        let tmp = tempdir();
        let crates = tmp.join("crates");
        let bad = crates.join("cave-broken");
        fs::create_dir_all(bad.join("src")).unwrap();
        fs::write(bad.join("parity.manifest.toml"), "this is not = valid [[ toml").unwrap();
        write_manifest(&crates.join("cave-good"), "good");

        let results = discover_workspace(&tmp);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].report.module, "good");
        cleanup(&tmp);
    }

    #[test]
    fn discover_returns_empty_for_missing_crates_dir() {
        let tmp = tempdir();
        // no crates/ dir at all
        let results = discover_workspace(&tmp);
        assert!(results.is_empty());
        cleanup(&tmp);
    }

    fn tempdir() -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "cave-parity-discover-{}-{}",
            std::process::id(),
            unique_suffix(),
        ));
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    fn cleanup(p: &Path) {
        let _ = std::fs::remove_dir_all(p);
    }

    fn unique_suffix() -> u128 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }
}
