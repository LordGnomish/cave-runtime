// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: aquasecurity/trivy@8a3177a pkg/fanal/analyzer/language/{nodejs/npm,rust/cargo}

//! Language-ecosystem lockfile analyzers (npm, Cargo).

use super::{Analyzer, AnalyzerType, PackageInfo};
use serde::Deserialize;
use std::collections::BTreeMap;

// ── npm ────────────────────────────────────────────────────────────────────

pub struct NpmLockAnalyzer;

#[derive(Debug, Deserialize)]
struct NpmLockFile {
    #[serde(default)]
    #[serde(rename = "lockfileVersion")]
    lockfile_version: u32,
    #[serde(default)]
    packages: BTreeMap<String, NpmLockEntry>,
    #[serde(default)]
    dependencies: BTreeMap<String, NpmLockEntry>,
}

#[derive(Debug, Default, Deserialize)]
struct NpmLockEntry {
    #[serde(default)]
    version: String,
    #[serde(default)]
    license: Option<String>,
}

impl NpmLockAnalyzer {
    pub fn parse_lock(&self, input: &str) -> Result<Vec<PackageInfo>, serde_json::Error> {
        let lock: NpmLockFile = serde_json::from_str(input)?;
        let mut pkgs = Vec::new();
        if lock.lockfile_version >= 2 || !lock.packages.is_empty() {
            for (path, entry) in &lock.packages {
                // Root entry has empty path key; skip it.
                if path.is_empty() {
                    continue;
                }
                // Derive name from the trailing `node_modules/<name>` segment.
                let name = path
                    .rsplit("node_modules/")
                    .next()
                    .unwrap_or(path)
                    .to_string();
                if name.is_empty() {
                    continue;
                }
                pkgs.push(PackageInfo {
                    name,
                    version: entry.version.clone(),
                    license: entry.license.clone(),
                    ..Default::default()
                });
            }
        } else {
            for (name, entry) in &lock.dependencies {
                pkgs.push(PackageInfo {
                    name: name.clone(),
                    version: entry.version.clone(),
                    license: entry.license.clone(),
                    ..Default::default()
                });
            }
        }
        Ok(pkgs)
    }
}

impl Analyzer for NpmLockAnalyzer {
    fn kind(&self) -> AnalyzerType {
        AnalyzerType::Npm
    }
    fn required(&self, path: &str) -> bool {
        let trimmed = path.trim_start_matches('/');
        if !trimmed.ends_with("package-lock.json") {
            return false;
        }
        // Skip lockfiles found inside a node_modules subtree — these belong
        // to transitive packages, not the project under scan.
        !trimmed.split('/').any(|seg| seg == "node_modules")
    }
}

// ── Cargo ──────────────────────────────────────────────────────────────────

pub struct CargoLockAnalyzer;

#[derive(Debug, Deserialize)]
struct CargoLockFile {
    #[serde(default, rename = "package")]
    packages: Vec<CargoLockEntry>,
}

#[derive(Debug, Deserialize)]
struct CargoLockEntry {
    name: String,
    version: String,
}

impl CargoLockAnalyzer {
    pub fn parse_lock(&self, input: &str) -> Result<Vec<PackageInfo>, toml::de::Error> {
        let lock: CargoLockFile = toml::from_str(input)?;
        Ok(lock
            .packages
            .into_iter()
            .map(|p| PackageInfo {
                name: p.name,
                version: p.version,
                ..Default::default()
            })
            .collect())
    }
}

impl Analyzer for CargoLockAnalyzer {
    fn kind(&self) -> AnalyzerType {
        AnalyzerType::CargoLock
    }
    fn required(&self, path: &str) -> bool {
        let trimmed = path.trim_start_matches('/');
        if !trimmed.ends_with("Cargo.lock") {
            return false;
        }
        !trimmed.split('/').any(|seg| seg == "vendor")
    }
}
