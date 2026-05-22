// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: aquasecurity/trivy@8a3177a pkg/fanal/artifact/local/fs.go

//! Filesystem scanner — walks a directory and dispatches analyzers.

use super::{ScanError, ScanReport, ScanRequest, ScanTarget, Scanner};
use crate::analyzer::{
    AnalyzerRegistry, AnalyzerType, PackageInfo,
    language::{CargoLockAnalyzer, NpmLockAnalyzer},
    os::{AlpineApkAnalyzer, DpkgStatusAnalyzer},
};
use std::path::Path;

pub struct FsScanner {
    registry: AnalyzerRegistry,
}

impl Default for FsScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl FsScanner {
    pub fn new() -> Self {
        Self {
            registry: AnalyzerRegistry::default_set(),
        }
    }
}

impl Scanner for FsScanner {
    fn name(&self) -> &'static str {
        "fs"
    }

    fn scan(&self, req: &ScanRequest) -> Result<ScanReport, ScanError> {
        let root = match &req.target {
            ScanTarget::Filesystem(p) => p.clone(),
            other => return Err(ScanError::InvalidTarget(format!("{other:?}"))),
        };

        let mut packages = Vec::new();
        walk(&root, &root, &self.registry, &mut packages)?;
        Ok(ScanReport {
            target: root.display().to_string(),
            packages,
        })
    }
}

fn walk(
    root: &Path,
    dir: &Path,
    reg: &AnalyzerRegistry,
    out: &mut Vec<PackageInfo>,
) -> Result<(), ScanError> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, reg, out)?;
        } else if path.is_file() {
            let rel = path
                .strip_prefix(root)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| path.to_string_lossy().into_owned());
            let chosen = reg.analyzers_for(&rel);
            if chosen.is_empty() {
                continue;
            }
            let contents = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(_) => continue, // binary or unreadable; skip text-based analyzers
            };
            for a in chosen {
                let pkgs = dispatch_parse(a.kind(), &contents);
                out.extend(pkgs);
            }
        }
    }
    Ok(())
}

fn dispatch_parse(kind: AnalyzerType, contents: &str) -> Vec<PackageInfo> {
    match kind {
        AnalyzerType::AlpineApk => AlpineApkAnalyzer.parse_installed_db(contents),
        AnalyzerType::DpkgStatus => DpkgStatusAnalyzer.parse_status(contents),
        AnalyzerType::Npm => NpmLockAnalyzer.parse_lock(contents).unwrap_or_default(),
        AnalyzerType::CargoLock => CargoLockAnalyzer.parse_lock(contents).unwrap_or_default(),
        AnalyzerType::Binary => Vec::new(),
    }
}
