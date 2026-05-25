// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Baseline file — persistent set of known-false-positive fingerprints.
//!
//! Mirrors `detect/baseline.go` upstream (`v8.29.1`). The upstream
//! baseline format is a JSON array of full `report.Finding` rows
//! emitted by a previous run, dedup-keyed by the `Fingerprint` field.
//! cave-gitleaks accepts a more compact TOML format on disk
//! (`[[entries]] { fingerprint, rule_id, file, start_line, note }`)
//! AND can read the upstream JSON shape on demand via [`Baseline::from_json`].

use std::collections::HashSet;
use std::path::Path;

use serde::Deserialize;
use thiserror::Error;

use crate::finding::Finding;

#[derive(Debug, Error)]
pub enum BaselineError {
    #[error("malformed TOML: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("malformed JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// On-disk TOML schema.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BaselineFile {
    #[serde(default)]
    pub entries: Vec<BaselineEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BaselineEntry {
    pub fingerprint: String,
    #[serde(default)]
    pub rule_id: String,
    #[serde(default)]
    pub file: String,
    #[serde(default)]
    pub start_line: usize,
    #[serde(default)]
    pub note: String,
}

impl BaselineFile {
    /// Parse a TOML baseline document.
    pub fn parse(toml_text: &str) -> Result<Self, BaselineError> {
        if toml_text.trim().is_empty() {
            return Ok(Self::default());
        }
        Ok(toml::from_str(toml_text)?)
    }

    /// Load from disk.
    pub fn load(path: &Path) -> Result<Self, BaselineError> {
        let text = std::fs::read_to_string(path)?;
        Self::parse(&text)
    }
}

/// In-memory baseline lookup — wraps a `HashSet<fingerprint>`.
#[derive(Debug, Default, Clone)]
pub struct Baseline {
    seen: HashSet<String>,
}

impl Baseline {
    /// Build from the upstream JSON baseline format (array of full Findings).
    pub fn from_json(json_text: &str) -> Result<Self, BaselineError> {
        if json_text.trim().is_empty() {
            return Ok(Self::default());
        }
        let findings: Vec<JsonBaselineRow> = serde_json::from_str(json_text)?;
        let seen = findings.into_iter().map(|f| f.fingerprint).collect();
        Ok(Self { seen })
    }

    /// Returns true if `finding.fingerprint` was already known.
    pub fn contains(&self, finding: &Finding) -> bool {
        self.seen.contains(&finding.fingerprint)
    }

    /// Strip findings whose fingerprint is already in the baseline.
    pub fn filter(&self, findings: Vec<Finding>) -> Vec<Finding> {
        findings.into_iter().filter(|f| !self.contains(f)).collect()
    }

    /// Add a finding's fingerprint to the in-memory baseline.
    pub fn ingest(&mut self, fingerprint: String) {
        self.seen.insert(fingerprint);
    }

    /// Total tracked fingerprints.
    pub fn len(&self) -> usize {
        self.seen.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }
}

impl From<BaselineFile> for Baseline {
    fn from(b: BaselineFile) -> Self {
        let seen = b.entries.into_iter().map(|e| e.fingerprint).collect();
        Self { seen }
    }
}

/// Upstream JSON shape we read for baseline JSON files — just enough to
/// pluck the fingerprint.
#[derive(Debug, Deserialize)]
struct JsonBaselineRow {
    #[serde(alias = "Fingerprint")]
    fingerprint: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_baseline_toml() {
        let b = BaselineFile::parse(
            r#"
            [[entries]]
            fingerprint = "abc"
            rule_id = "r"
            "#,
        )
        .unwrap();
        assert_eq!(b.entries.len(), 1);
        assert_eq!(b.entries[0].fingerprint, "abc");
    }

    #[test]
    fn rejects_unknown_fields() {
        let err = BaselineFile::parse(
            r#"
            [[entries]]
            fingerprint = "x"
            bogus       = "should fail"
            "#,
        )
        .unwrap_err();
        let s = format!("{err}");
        assert!(s.contains("bogus") || s.contains("unknown"));
    }

    #[test]
    fn baseline_from_json_extracts_fingerprints() {
        let json = r#"[{"Fingerprint":"abc"},{"Fingerprint":"def"}]"#;
        let b = Baseline::from_json(json).unwrap();
        assert_eq!(b.len(), 2);
    }

    #[test]
    fn baseline_empty_string_yields_empty_baseline() {
        assert!(Baseline::from_json("").unwrap().is_empty());
        assert!(Baseline::from(BaselineFile::parse("").unwrap()).is_empty());
    }
}
