// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-memory finding store. Matches the upstream `JobReport` aggregation
//! semantics — append-only, queryable by job id / detector type, with a
//! verdict-counter view for the Portal dashboard.

use crate::dedup::Dedup;
use crate::models::{DetectorType, Finding};
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub struct StoredFinding {
    pub id: String,
    pub finding: Finding,
}

pub struct FindingStore {
    inner: Mutex<Vec<StoredFinding>>,
    dedup: Dedup,
}

impl Default for FindingStore {
    fn default() -> Self {
        Self::new()
    }
}

impl FindingStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Vec::new()),
            dedup: Dedup::new(),
        }
    }

    pub fn insert(&self, f: Finding) -> Option<StoredFinding> {
        if !self.dedup.insert_finding(&f) {
            return None;
        }
        let sf = StoredFinding {
            id: Uuid::new_v4().to_string(),
            finding: f,
        };
        self.inner.lock().unwrap().push(sf.clone());
        Some(sf)
    }

    pub fn count(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.count() == 0
    }

    pub fn all(&self) -> Vec<StoredFinding> {
        self.inner.lock().unwrap().clone()
    }

    pub fn by_detector(&self, t: DetectorType) -> Vec<StoredFinding> {
        self.inner
            .lock()
            .unwrap()
            .iter()
            .filter(|sf| sf.finding.result.detector_type == t)
            .cloned()
            .collect()
    }

    pub fn verdict_counts(&self) -> HashMap<&'static str, usize> {
        let mut h: HashMap<&'static str, usize> = HashMap::new();
        for sf in self.inner.lock().unwrap().iter() {
            let key = if sf.finding.result.verified {
                "verified"
            } else if sf.finding.result.verification_error.is_some() {
                "indeterminate"
            } else {
                "unverified"
            };
            *h.entry(key).or_insert(0) += 1;
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{DetectionResult, SourceMetadata};

    fn finding(raw: &str, file: &str, verified: bool) -> Finding {
        let mut r = DetectionResult::new(DetectorType::Stripe, raw);
        r.verified = verified;
        Finding {
            result: r,
            chunk_source: "git".into(),
            source_metadata: SourceMetadata {
                file: Some(file.into()),
                ..Default::default()
            },
            redacted: "sk_…".into(),
        }
    }

    #[test]
    fn insert_dedupes() {
        let s = FindingStore::new();
        assert!(s.insert(finding("sk_live_a", "/a", true)).is_some());
        assert!(s.insert(finding("sk_live_a", "/a", true)).is_none());
        assert_eq!(s.count(), 1);
    }

    #[test]
    fn verdict_counts_by_status() {
        let s = FindingStore::new();
        s.insert(finding("sk_live_a", "/a", true));
        s.insert(finding("sk_live_b", "/b", false));
        s.insert(finding("sk_live_c", "/c", false));
        let c = s.verdict_counts();
        assert_eq!(c.get("verified"), Some(&1));
        assert_eq!(c.get("unverified"), Some(&2));
    }

    #[test]
    fn by_detector_filters() {
        let s = FindingStore::new();
        s.insert(finding("sk_live_a", "/a", true));
        let by = s.by_detector(DetectorType::Stripe);
        assert_eq!(by.len(), 1);
        let other = s.by_detector(DetectorType::Github);
        assert!(other.is_empty());
    }

    #[test]
    fn empty_store_is_empty() {
        let s = FindingStore::new();
        assert!(s.is_empty());
    }
}
