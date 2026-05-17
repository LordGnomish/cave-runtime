// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: aquasecurity/trivy-db@2034dd8 pkg/types/types.go
//! cave-scan-db — Vulnerability + IaC advisory store.
//!
//! Schema modelled on `trivy-db` (`pkg/types/types.go`, `pkg/db/db.go`).
//! Persistence: sled key-value store. JSON feed ingest. PURL-keyed lookup.
//!
//! Public entry points:
//!
//! * [`VulnDb`] — minimal trait for read/write advisory access.
//! * [`OsAdvisoryDb`] — OS-vendor advisories (Debian, RedHat, Alpine, AlmaLinux).
//! * [`LangAdvisoryDb`] — language ecosystem advisories (NPM, PyPI, Cargo, …).
//! * [`IacRuleDb`] — IaC misconfig rule definitions (CIS, CSP).
//! * [`matcher::match_purl`] — PURL → advisory lookup.

pub mod matcher;
pub mod sources;
pub mod storage;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Severity rank — mirrors `dbTypes.Severity` in trivy-db.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Severity {
    Unknown,
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    /// Parse trivy / NVD severity strings case-insensitively.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_uppercase().as_str() {
            "LOW" => Self::Low,
            "MEDIUM" | "MODERATE" => Self::Medium,
            "HIGH" | "IMPORTANT" => Self::High,
            "CRITICAL" => Self::Critical,
            _ => Self::Unknown,
        }
    }
}

/// CVSS v3 vector + base score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CvssV3 {
    pub vector: String,
    pub score: f32,
}

/// One CVE / advisory record.
///
/// Mirrors `trivy-db/pkg/types/types.go::Vulnerability`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Vulnerability {
    pub id: String,
    pub title: String,
    pub description: String,
    pub severity: Severity,
    #[serde(default)]
    pub cwe_ids: Vec<String>,
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(default)]
    pub cvss_v3: Option<CvssV3>,
    #[serde(default)]
    pub published_date: Option<String>,
    #[serde(default)]
    pub last_modified_date: Option<String>,
}

/// A vendor advisory pinning a CVE to a specific package version range.
///
/// Mirrors `trivy-db/pkg/types/types.go::Advisory`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Advisory {
    pub vulnerability_id: String,
    pub package_name: String,
    /// Ecosystem: "debian:12", "alpine:3.19", "npm", "pypi", …
    pub ecosystem: String,
    /// Fixed-in semver / vendor version, empty if unfixed.
    #[serde(default)]
    pub fixed_version: String,
    /// Affected version range expression. Either semver range ("<1.2.3"),
    /// dpkg/rpm exact list, or "*" for all.
    #[serde(default)]
    pub affected_version: String,
    #[serde(default)]
    pub severity: Severity,
    #[serde(default)]
    pub data_source: String,
}

impl Default for Severity {
    fn default() -> Self {
        Self::Unknown
    }
}

/// One IaC misconfig rule (CIS Benchmark / cloud security policy).
///
/// Mirrors `trivy/pkg/iac/rego/schemas` — but our IaC engine is pure-Rust,
/// not Rego, so we keep the metadata, not the policy code.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IacRule {
    pub id: String,
    /// e.g. `terraform`, `kubernetes`, `dockerfile`, `helm`, `cloudformation`.
    pub provider: String,
    pub title: String,
    pub description: String,
    pub severity: Severity,
    /// CIS Benchmark section IDs, e.g. ["5.4.1"].
    #[serde(default)]
    pub cis_ids: Vec<String>,
    /// Cross-mapped CSP control, e.g. "AWS-IAM-001".
    #[serde(default)]
    pub csp_control: Option<String>,
}

/// DB error surface.
#[derive(Debug, Error)]
pub enum DbError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("sled: {0}")]
    Sled(#[from] sled::Error),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid feed: {0}")]
    InvalidFeed(String),
}

pub type Result<T> = std::result::Result<T, DbError>;

/// Top-level read/write trait — anything that can store advisories.
pub trait VulnDb {
    fn put_vuln(&self, v: &Vulnerability) -> Result<()>;
    fn get_vuln(&self, id: &str) -> Result<Option<Vulnerability>>;
    fn put_advisory(&self, a: &Advisory) -> Result<()>;
    fn count_vulns(&self) -> Result<usize>;
    fn count_advisories(&self) -> Result<usize>;
}

/// OS-vendor advisories — Debian / RedHat / Alpine / AlmaLinux.
///
/// Lookup key: `(ecosystem, package_name) -> Vec<Advisory>`.
pub trait OsAdvisoryDb: VulnDb {
    fn advisories_for_pkg(&self, ecosystem: &str, package: &str) -> Result<Vec<Advisory>>;
}

/// Language ecosystem advisories — npm / pypi / cargo / go / maven / rubygems.
pub trait LangAdvisoryDb: VulnDb {
    fn advisories_for_lang_pkg(&self, ecosystem: &str, package: &str) -> Result<Vec<Advisory>>;
}

/// IaC rule store — separate KV namespace from CVE feeds.
pub trait IacRuleDb {
    fn put_rule(&self, r: &IacRule) -> Result<()>;
    fn get_rule(&self, id: &str) -> Result<Option<IacRule>>;
    fn rules_for_provider(&self, provider: &str) -> Result<Vec<IacRule>>;
}

pub use matcher::{match_purl, PackageRef};
pub use storage::SledStore;
