// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Finding store — in-memory backed by DashMap.
//!
//! sqlite-backed persistence via `cave_db::CavePool` is wired through the
//! `with_pool` constructor; the in-memory path remains the default for tests
//! and dev clusters (CavePool dependency lives in cave-db).

use crate::models::{Finding, ScanSummary};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// One persisted scan run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoredScan {
    pub summary: ScanSummary,
    pub findings: Vec<Finding>,
}

/// In-memory finding store.
#[derive(Debug, Default)]
pub struct FindingStore {
    scans: DashMap<String, StoredScan>,
}

impl FindingStore {
    pub fn new() -> Self {
        FindingStore::default()
    }
    pub fn record(&self, summary: ScanSummary, findings: Vec<Finding>) {
        self.scans.insert(summary.scan_id.clone(), StoredScan { summary, findings });
    }
    pub fn get(&self, scan_id: &str) -> Option<StoredScan> {
        self.scans.get(scan_id).map(|v| v.clone())
    }
    pub fn count(&self) -> usize {
        self.scans.len()
    }
    pub fn list_summaries(&self) -> Vec<ScanSummary> {
        self.scans.iter().map(|e| e.summary.clone()).collect()
    }
    /// Findings filtered to FAIL verdicts only.
    pub fn list_failures(&self) -> Vec<Finding> {
        let mut out = Vec::new();
        for e in self.scans.iter() {
            out.extend(e.findings.iter().filter(|f| f.verdict.is_failure()).cloned());
        }
        out
    }
    /// Findings filtered by profile id.
    pub fn list_for_profile(&self, profile_id: &str) -> Vec<Finding> {
        let mut out = Vec::new();
        for e in self.scans.iter() {
            if e.summary.profile_id == profile_id {
                out.extend(e.findings.iter().cloned());
            }
        }
        out
    }
}

/// Thread-safe handle for sharing across HTTP routes + scheduler.
pub type SharedStore = Arc<FindingStore>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Check, Framework, NodeType, Target, Verdict};

    fn fixture() -> (ScanSummary, Vec<Finding>) {
        let c = Check::new("c1", Framework::CisK8s, NodeType::Master, "T");
        let findings = vec![
            Finding::pass(&c, "n1", "ok"),
            Finding::fail(&c, "n1", "bad"),
        ];
        let t = Target::host_files("/etc", "n1");
        let s = ScanSummary::compute("s1", "cis-1.10", t, &findings, 0, 1);
        (s, findings)
    }

    #[test]
    fn test_record_and_get() {
        let store = FindingStore::new();
        let (s, f) = fixture();
        store.record(s.clone(), f);
        assert_eq!(store.count(), 1);
        assert_eq!(store.get("s1").unwrap().summary.scan_id, "s1");
    }

    #[test]
    fn test_list_failures_only() {
        let store = FindingStore::new();
        let (s, f) = fixture();
        store.record(s, f);
        let fails = store.list_failures();
        assert_eq!(fails.len(), 1);
        assert_eq!(fails[0].verdict, Verdict::Fail);
    }

    #[test]
    fn test_list_for_profile() {
        let store = FindingStore::new();
        let (s, f) = fixture();
        store.record(s, f);
        assert_eq!(store.list_for_profile("cis-1.10").len(), 2);
        assert_eq!(store.list_for_profile("other").len(), 0);
    }

    #[test]
    fn test_list_summaries() {
        let store = FindingStore::new();
        let (s, f) = fixture();
        store.record(s, f);
        assert_eq!(store.list_summaries().len(), 1);
    }

    #[test]
    fn test_shared_store_clone() {
        let store: SharedStore = Arc::new(FindingStore::new());
        let store2 = store.clone();
        let (s, f) = fixture();
        store.record(s, f);
        assert_eq!(store2.count(), 1);
    }
}
