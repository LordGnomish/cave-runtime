// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! In-memory store for scan results.
//!
//! Mirrors a small subset of trivy's `pkg/report/io` persistence: keep the
//! latest N reports keyed by artifact name so the server mode + Portal can
//! list them. Persistent (sled-backed) storage is a scope cut.

use crate::error::{TrivyError, TrivyResult};
use crate::models::Report;
use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Debug, Default)]
pub struct ScanStore {
    inner: RwLock<HashMap<String, Report>>,
    history: RwLock<Vec<String>>,
    max_history: usize,
}

impl Clone for ScanStore {
    fn clone(&self) -> Self {
        let g = self.inner.read().expect("scan store poisoned");
        let h = self.history.read().expect("scan store poisoned");
        Self {
            inner: RwLock::new(g.clone()),
            history: RwLock::new(h.clone()),
            max_history: self.max_history,
        }
    }
}

impl ScanStore {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
            history: RwLock::new(Vec::new()),
            max_history: 256,
        }
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            max_history: cap,
            ..Self::new()
        }
    }

    pub fn insert(&self, report: Report) -> TrivyResult<String> {
        if report.artifact_name.is_empty() {
            return Err(TrivyError::Report("empty artifact_name".into()));
        }
        let id = report.artifact_name.clone();
        let mut m = self.inner.write().expect("scan store poisoned");
        let mut h = self.history.write().expect("scan store poisoned");
        m.insert(id.clone(), report);
        h.push(id.clone());
        while h.len() > self.max_history {
            let oldest = h.remove(0);
            m.remove(&oldest);
        }
        Ok(id)
    }

    pub fn get(&self, id: &str) -> Option<Report> {
        self.inner
            .read()
            .expect("scan store poisoned")
            .get(id)
            .cloned()
    }

    pub fn count(&self) -> TrivyResult<usize> {
        Ok(self.inner.read().expect("scan store poisoned").len())
    }

    pub fn ids(&self) -> Vec<String> {
        self.history
            .read()
            .expect("scan store poisoned")
            .iter()
            .cloned()
            .collect()
    }

    pub fn delete(&self, id: &str) -> bool {
        let mut m = self.inner.write().expect("scan store poisoned");
        let mut h = self.history.write().expect("scan store poisoned");
        let removed = m.remove(id).is_some();
        h.retain(|x| x != id);
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_get() {
        let s = ScanStore::new();
        let r = Report::new("alpine", "container_image");
        let id = s.insert(r.clone()).unwrap();
        assert_eq!(id, "alpine");
        let back = s.get("alpine").unwrap();
        assert_eq!(back.artifact_name, "alpine");
    }

    #[test]
    fn empty_artifact_name_rejected() {
        let s = ScanStore::new();
        let r = Report::new("", "container_image");
        assert!(s.insert(r).is_err());
    }

    #[test]
    fn count_increments() {
        let s = ScanStore::new();
        s.insert(Report::new("a", "x")).unwrap();
        s.insert(Report::new("b", "x")).unwrap();
        assert_eq!(s.count().unwrap(), 2);
    }

    #[test]
    fn history_caps() {
        let s = ScanStore::with_capacity(2);
        s.insert(Report::new("a", "x")).unwrap();
        s.insert(Report::new("b", "x")).unwrap();
        s.insert(Report::new("c", "x")).unwrap();
        assert_eq!(s.count().unwrap(), 2);
        assert!(s.get("a").is_none());
        assert!(s.get("c").is_some());
    }

    #[test]
    fn delete_removes() {
        let s = ScanStore::new();
        s.insert(Report::new("a", "x")).unwrap();
        assert!(s.delete("a"));
        assert_eq!(s.count().unwrap(), 0);
        assert!(!s.delete("nope"));
    }

    #[test]
    fn ids_listing() {
        let s = ScanStore::new();
        s.insert(Report::new("a", "x")).unwrap();
        s.insert(Report::new("b", "x")).unwrap();
        let ids = s.ids();
        assert_eq!(ids, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn store_clone() {
        let s = ScanStore::new();
        s.insert(Report::new("a", "x")).unwrap();
        let cloned = s.clone();
        assert_eq!(cloned.count().unwrap(), 1);
    }
}
