// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Baseline persistence — store known/accepted findings on disk and use them
//! to suppress noise on subsequent scans (mirrors gitleaks' `--baseline-path`
//! and TruffleHog's `--exclude-detectors` allowlist semantics).
//!
//! Finding identity is a SHA-256 over `detector + file + line + redacted`,
//! producing stable IDs across runs as long as the line number and redacted
//! preview do not drift.

use crate::detector::Finding;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BaselineFile {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub entries: Vec<BaselineEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineEntry {
    pub id: String,
    pub detector: String,
    pub file: String,
    pub line: usize,
    #[serde(default)]
    pub redacted: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Default, Clone)]
pub struct Baseline {
    ids: HashSet<String>,
}

pub fn finding_id(detector: &str, file: &str, line: usize, redacted: &str) -> String {
    let mut h = Sha256::new();
    h.update(detector.as_bytes());
    h.update([0u8]);
    h.update(file.as_bytes());
    h.update([0u8]);
    h.update(line.to_le_bytes());
    h.update([0u8]);
    h.update(redacted.as_bytes());
    hex::encode(h.finalize())
}

impl Baseline {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_file(file: &BaselineFile) -> Self {
        Self {
            ids: file.entries.iter().map(|e| e.id.clone()).collect(),
        }
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.ids.contains(id)
    }

    pub fn contains_finding(&self, f: &Finding) -> bool {
        let id = finding_id(&f.detector, &f.file, f.line, &f.matched);
        self.contains(&id)
    }

    /// Add a finding's ID to the baseline.
    pub fn add_finding(&mut self, f: &Finding) -> String {
        let id = finding_id(&f.detector, &f.file, f.line, &f.matched);
        self.ids.insert(id.clone());
        id
    }

    /// Read JSON baseline file from disk; missing path returns empty baseline.
    pub fn load_json<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::new()),
            Err(e) => return Err(e),
        };
        let file: BaselineFile = serde_json::from_slice(&bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok(Self::from_file(&file))
    }

    /// Persist baseline back to disk as JSON. Currently emits a snapshot built
    /// from the given findings list (callers retain authoritative finding data).
    pub fn save_json<P: AsRef<Path>>(path: P, findings: &[Finding], note: &str) -> io::Result<()> {
        let entries: Vec<BaselineEntry> = findings
            .iter()
            .map(|f| {
                let id = finding_id(&f.detector, &f.file, f.line, &f.matched);
                BaselineEntry {
                    id,
                    detector: f.detector.clone(),
                    file: f.file.clone(),
                    line: f.line,
                    redacted: f.matched.clone(),
                    note: note.to_string(),
                }
            })
            .collect();
        let file = BaselineFile { version: 1, entries };
        let json = serde_json::to_vec_pretty(&file)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if let Some(parent) = path.as_ref().parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        fs::write(path, json)
    }

    /// Filter findings to drop those that match the baseline. Returns the
    /// kept findings (new noise) and the count of suppressed findings.
    pub fn filter<'a>(&self, findings: &'a [Finding]) -> (Vec<&'a Finding>, usize) {
        let mut kept = Vec::with_capacity(findings.len());
        let mut suppressed = 0;
        for f in findings {
            if self.contains_finding(f) {
                suppressed += 1;
            } else {
                kept.push(f);
            }
        }
        (kept, suppressed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detector::{Finding, Severity};

    fn mk(det: &str, file: &str, line: usize, m: &str) -> Finding {
        Finding {
            detector: det.to_string(),
            file: file.to_string(),
            line,
            matched: m.to_string(),
            severity: Severity::High,
            verified: false,
        }
    }

    #[test]
    fn finding_id_is_deterministic() {
        let a = finding_id("aws", "f", 3, "AKIA...");
        let b = finding_id("aws", "f", 3, "AKIA...");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn finding_id_differs_on_inputs() {
        assert_ne!(finding_id("a", "f", 1, "x"), finding_id("b", "f", 1, "x"));
        assert_ne!(finding_id("a", "f", 1, "x"), finding_id("a", "g", 1, "x"));
        assert_ne!(finding_id("a", "f", 1, "x"), finding_id("a", "f", 2, "x"));
        assert_ne!(finding_id("a", "f", 1, "x"), finding_id("a", "f", 1, "y"));
    }

    #[test]
    fn empty_baseline_passes_through() {
        let b = Baseline::new();
        let f = mk("d", "x.env", 1, "X");
        assert!(!b.contains_finding(&f));
        let (kept, supp) = b.filter(std::slice::from_ref(&f));
        assert_eq!(kept.len(), 1);
        assert_eq!(supp, 0);
    }

    #[test]
    fn add_then_contains() {
        let mut b = Baseline::new();
        let f = mk("d", "x.env", 1, "X");
        b.add_finding(&f);
        assert!(b.contains_finding(&f));
    }

    #[test]
    fn filter_suppresses_known() {
        let mut b = Baseline::new();
        let known = mk("aws", "x.env", 2, "AKIA...");
        let novel = mk("aws", "x.env", 5, "AKIA...");
        b.add_finding(&known);
        let inputs = [known.clone(), novel.clone()];
        let (kept, supp) = b.filter(&inputs);
        assert_eq!(supp, 1);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].line, 5);
    }

    #[test]
    fn save_then_load_roundtrip() {
        let tmp = std::env::temp_dir().join(format!(
            "cave-secrets-baseline-{}.json",
            std::process::id()
        ));
        let f = mk("aws", "x.env", 7, "AKIA...");
        Baseline::save_json(&tmp, std::slice::from_ref(&f), "test").unwrap();
        let loaded = Baseline::load_json(&tmp).unwrap();
        assert!(loaded.contains_finding(&f));
        assert_eq!(loaded.len(), 1);
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn load_missing_path_returns_empty() {
        let p = std::env::temp_dir().join("does-not-exist-baseline-xyz.json");
        let b = Baseline::load_json(&p).unwrap();
        assert!(b.is_empty());
    }

    #[test]
    fn load_invalid_json_fails() {
        let p = std::env::temp_dir().join("cave-secrets-invalid-baseline.json");
        fs::write(&p, b"not json").unwrap();
        assert!(Baseline::load_json(&p).is_err());
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn baseline_file_version_persisted() {
        let tmp = std::env::temp_dir().join(format!(
            "cave-secrets-baseline-ver-{}.json",
            std::process::id()
        ));
        let f = mk("d", "x", 1, "v");
        Baseline::save_json(&tmp, std::slice::from_ref(&f), "n").unwrap();
        let raw = fs::read_to_string(&tmp).unwrap();
        assert!(raw.contains("\"version\""));
        let _ = fs::remove_file(&tmp);
    }
}
