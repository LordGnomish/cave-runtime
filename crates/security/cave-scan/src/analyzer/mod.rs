// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: aquasecurity/trivy@8a3177a pkg/fanal/analyzer/analyzer.go

//! Analyzer registry — Trivy-style file-driven package discovery.
//!
//! Each analyzer declares which paths it cares about (`required`) and
//! parses the contents into [`PackageInfo`] records.

pub mod binary;
pub mod language;
pub mod os;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PackageInfo {
    pub name: String,
    pub version: String,
    pub arch: Option<String>,
    pub license: Option<String>,
    pub origin: Option<String>,
    pub source: Option<String>,
    pub source_version: Option<String>,
    pub provides: Vec<String>,
    pub depends: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnalyzerType {
    AlpineApk,
    DpkgStatus,
    Npm,
    CargoLock,
    Binary,
}

pub trait Analyzer: Send + Sync {
    fn kind(&self) -> AnalyzerType;
    fn required(&self, path: &str) -> bool;
}

pub struct AnalyzerRegistry {
    analyzers: Vec<Box<dyn Analyzer>>,
}

impl AnalyzerRegistry {
    pub fn default_set() -> Self {
        Self {
            analyzers: vec![
                Box::new(os::AlpineApkAnalyzer),
                Box::new(os::DpkgStatusAnalyzer),
                Box::new(language::NpmLockAnalyzer),
                Box::new(language::CargoLockAnalyzer),
                Box::new(binary::BinaryAnalyzer),
            ],
        }
    }

    pub fn analyzers_for(&self, path: &str) -> Vec<&dyn Analyzer> {
        self.analyzers
            .iter()
            .filter(|a| a.required(path))
            .map(|a| a.as_ref())
            .collect()
    }
}
