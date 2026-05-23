// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Filesystem scanner.
//!
//! Mirrors trivy's `pkg/scan/artifact/local` for a directory tree: walk
//! entries, dispatch each path by basename to `pkg_lang::parse_lockfile`,
//! correlate against the vuln DB and emit per-file scan results. Live
//! filesystem traversal is provided via the `FsTree` abstraction so tests
//! and the server mode can ingest a pre-collected `Vec<(path, bytes)>`.

use crate::error::TrivyResult;
use crate::models::{Package, Report, ScanResult, Vulnerability};
use crate::pkg_lang::parse_lockfile;
use crate::vulndb::VulnDb;
use std::path::Path;

#[derive(Debug, Default, Clone)]
pub struct FsTree {
    pub files: Vec<(String, String)>,
}

impl FsTree {
    pub fn push(mut self, path: &str, body: &str) -> Self {
        self.files.push((path.into(), body.into()));
        self
    }

    /// Pull a real directory tree into memory. Limited to 5 MB / file to
    /// keep cave-trivy's MVP scanner predictable.
    pub fn from_dir<P: AsRef<Path>>(dir: P) -> std::io::Result<Self> {
        let mut t = FsTree::default();
        load_dir(dir.as_ref(), dir.as_ref(), &mut t)?;
        Ok(t)
    }
}

fn load_dir(root: &Path, dir: &Path, out: &mut FsTree) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let e = entry?;
        let p = e.path();
        if p.is_dir() {
            if p.file_name().and_then(|n| n.to_str()) == Some(".git") {
                continue;
            }
            load_dir(root, &p, out)?;
        } else if let Ok(meta) = p.metadata() {
            if meta.len() > 5 * 1024 * 1024 {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&p) {
                let rel = p
                    .strip_prefix(root)
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|_| p.to_string_lossy().to_string());
                out.files.push((rel, text));
            }
        }
    }
    Ok(())
}

pub fn scan_fs(name: &str, tree: &FsTree, db: &VulnDb) -> TrivyResult<Report> {
    let mut report = Report::new(name, "filesystem");
    for (path, text) in &tree.files {
        let base = path.rsplit('/').next().unwrap_or(path);
        let pkgs = parse_lockfile(base, text);
        if pkgs.is_empty() {
            continue;
        }
        let mut r = ScanResult {
            target: path.clone(),
            class: "lang-pkgs".into(),
            ..Default::default()
        };
        correlate_pkgs(db, &pkgs, &mut r.vulnerabilities);
        report.results.push(r);
    }
    Ok(report)
}

fn correlate_pkgs(db: &VulnDb, pkgs: &[Package], out: &mut Vec<Vulnerability>) {
    for p in pkgs {
        for adv in db.match_pkg(&p.ecosystem, &p.name, &p.version) {
            out.push(Vulnerability {
                id: adv.id.clone(),
                pkg_name: p.name.clone(),
                installed_version: p.version.clone(),
                fixed_version: adv.fixed.clone(),
                severity: adv.severity,
                references: adv.references.clone(),
                title: Some(adv.title.clone()),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_npm_lockfile() {
        let tree = FsTree::default()
            .push("package-lock.json", r#"{"dependencies":{"lodash":{"version":"4.17.20"}}}"#);
        let r = scan_fs("repo", &tree, &VulnDb::cave_default()).unwrap();
        assert_eq!(r.total_vulns(), 1);
        assert_eq!(r.results[0].class, "lang-pkgs");
    }

    #[test]
    fn ignores_unknown_files() {
        let tree = FsTree::default().push("README.md", "# hi");
        let r = scan_fs("repo", &tree, &VulnDb::cave_default()).unwrap();
        assert!(r.results.is_empty());
    }

    #[test]
    fn multiple_lockfiles() {
        let tree = FsTree::default()
            .push("a/Cargo.lock", "[[package]]\nname = \"openssl-sys\"\nversion = \"0.9.0\"\n")
            .push("b/requirements.txt", "requests==2.31.0\n");
        let r = scan_fs("multi", &tree, &VulnDb::cave_default()).unwrap();
        assert_eq!(r.results.len(), 2);
        assert!(r.total_vulns() >= 2);
    }

    #[test]
    fn fs_tree_from_real_dir() {
        let tmp = std::env::temp_dir().join(format!("cave-trivy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("Cargo.lock"), "[[package]]\nname=\"x\"\nversion=\"1\"\n").unwrap();
        let tree = FsTree::from_dir(&tmp).unwrap();
        assert!(tree.files.iter().any(|(p, _)| p.contains("Cargo.lock")));
    }
}
