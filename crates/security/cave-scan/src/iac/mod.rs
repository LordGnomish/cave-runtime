// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: aquasecurity/trivy@8a3177a pkg/iac/scanners/scanner.go
//! IaC (Infrastructure-as-Code) misconfig scanners.
//!
//! Each provider implements [`IacScanner`]. Findings carry a rule id (e.g.
//! `AVD-AWS-0001` mirroring Trivy's Aqua Vulnerability Database naming),
//! severity, and a 1-based line number.

pub mod cloudformation;
pub mod dockerfile;
pub mod helm;
pub mod kubernetes;
pub mod terraform;

use std::fmt;

/// IaC misconfig severity. Mirrors trivy `defsecTypes.Severity`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Severity::Low => f.write_str("LOW"),
            Severity::Medium => f.write_str("MEDIUM"),
            Severity::High => f.write_str("HIGH"),
            Severity::Critical => f.write_str("CRITICAL"),
        }
    }
}

/// One misconfig finding. `line` is 1-based; 0 means "file-level".
#[derive(Debug, Clone, PartialEq)]
pub struct IacFinding {
    pub rule_id: String,
    pub severity: Severity,
    pub message: String,
    pub file: String,
    pub line: usize,
}

/// Scanner contract: ingest text → produce findings.
///
/// Implementations are stateless after `new()`. Scan is single-pass; no
/// network or filesystem access (so tests stay deterministic).
pub trait IacScanner {
    fn provider(&self) -> &'static str;
    fn scan_str(&self, content: &str, path: &str) -> Result<Vec<IacFinding>, IacError>;
}

/// IaC scanner error.
#[derive(Debug, thiserror::Error)]
pub enum IacError {
    #[error("parse: {0}")]
    Parse(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
