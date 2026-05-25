// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: gitleaks/gitleaks@9febafb config/gitleaks.toml
//! Secret detection — regex patterns + entropy.
//!
//! Two-stage pipeline (mirroring gitleaks):
//!
//! 1. **patterns** — fixed regex set (≥40 rules), each with an id, kind, severity.
//! 2. **entropy** — Shannon entropy over candidate strings caught by a
//!    generic high-entropy regex; threshold-gated.

pub mod entropy;
pub mod patterns;

/// One detected secret instance.
#[derive(Debug, Clone, PartialEq)]
pub struct SecretFinding {
    pub rule_id: String,
    pub severity: Severity,
    pub file: String,
    /// 1-based line number where the secret begins.
    pub line: usize,
    /// Truncated/masked sample of the matched secret (first 6 chars + …).
    pub sample: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

/// Detector trait — anything that returns SecretFindings for a buffer.
pub trait SecretDetector {
    fn scan(&self, content: &str, path: &str) -> Vec<SecretFinding>;
}
