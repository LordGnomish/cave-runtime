// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Checkpoint state — port of `pkg/sources/resume.go`. Persisted by the
//! Engine when running scans against large sources (e.g. millions of git
//! commits) so a `--resume` invocation picks up where it left off.

use crate::error::{Error, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ResumeState {
    pub job_id: String,
    pub source_name: String,
    pub last_chunk_offset: u64,
    pub last_commit: Option<String>,
    pub completed_units: BTreeMap<String, bool>,
    pub started_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl ResumeState {
    pub fn new(job_id: impl Into<String>, source_name: impl Into<String>) -> Self {
        Self {
            job_id: job_id.into(),
            source_name: source_name.into(),
            last_chunk_offset: 0,
            last_commit: None,
            completed_units: BTreeMap::new(),
            started_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
        }
    }

    pub fn mark_unit_complete(&mut self, unit: impl Into<String>) {
        self.completed_units.insert(unit.into(), true);
        self.updated_at = Some(Utc::now());
    }

    pub fn is_unit_complete(&self, unit: &str) -> bool {
        self.completed_units.get(unit).copied().unwrap_or(false)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let j = serde_json::to_string_pretty(self)
            .map_err(|e| Error::Serialization(e.to_string()))?;
        std::fs::write(path, j).map_err(|e| Error::Io(e.to_string()))
    }

    pub fn load(path: &Path) -> Result<Self> {
        let s = std::fs::read_to_string(path).map_err(|e| Error::Io(e.to_string()))?;
        serde_json::from_str(&s).map_err(|e| Error::Serialization(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn mark_unit_complete_idempotent() {
        let mut s = ResumeState::new("j1", "git");
        s.mark_unit_complete("commit-a");
        assert!(s.is_unit_complete("commit-a"));
        s.mark_unit_complete("commit-a");
        assert_eq!(s.completed_units.len(), 1);
    }

    #[test]
    fn save_and_load_round_trip() {
        let td = TempDir::new().unwrap();
        let p = td.path().join("rs.json");
        let mut s = ResumeState::new("j1", "git");
        s.last_chunk_offset = 4096;
        s.last_commit = Some("abc123".into());
        s.mark_unit_complete("file-1");
        s.save(&p).unwrap();
        let back = ResumeState::load(&p).unwrap();
        assert_eq!(back.last_chunk_offset, 4096);
        assert_eq!(back.last_commit.as_deref(), Some("abc123"));
        assert!(back.is_unit_complete("file-1"));
    }

    #[test]
    fn missing_unit_is_not_complete() {
        let s = ResumeState::new("j1", "git");
        assert!(!s.is_unit_complete("ghost"));
    }
}
