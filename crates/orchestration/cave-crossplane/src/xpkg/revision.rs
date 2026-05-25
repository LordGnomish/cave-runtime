// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Package revision rollout — active/inactive bookkeeping with
//! revisionHistoryLimit + manual activate/deactivate.
//!
//! Upstream: internal/controller/pkg/revision/reconciler.go::ReconcileRevision

use crate::error::{CrossplaneError, CrossplaneResult};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};

const DEFAULT_HISTORY_LIMIT: usize = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PackageRevisionState {
    Active,
    Inactive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageRevision {
    pub package: String,
    pub revision: String,
    pub state: PackageRevisionState,
    pub recorded_at: DateTime<Utc>,
}

#[derive(Default)]
pub struct PackageRevisionTracker {
    revisions: DashMap<String, Vec<PackageRevision>>,
    history_limit: usize,
}

impl PackageRevisionTracker {
    pub fn new() -> Self {
        Self {
            revisions: DashMap::new(),
            history_limit: DEFAULT_HISTORY_LIMIT,
        }
    }

    pub fn with_history_limit(limit: usize) -> Self {
        Self {
            revisions: DashMap::new(),
            history_limit: limit,
        }
    }

    pub fn record(&self, package: &str, revision: &str) -> PackageRevision {
        let new = PackageRevision {
            package: package.to_string(),
            revision: revision.to_string(),
            state: PackageRevisionState::Active,
            recorded_at: Utc::now(),
        };
        let mut list = self.revisions.entry(package.to_string()).or_default();
        for r in list.iter_mut() {
            r.state = PackageRevisionState::Inactive;
        }
        list.push(new.clone());
        let inactive_keep = self.history_limit.max(1);
        let total = list.len();
        if total > inactive_keep + 1 {
            let drop = total - (inactive_keep + 1);
            list.drain(0..drop);
        }
        new
    }

    pub fn activate(&self, package: &str, revision: &str) -> CrossplaneResult<()> {
        let mut list = self
            .revisions
            .get_mut(package)
            .ok_or_else(|| CrossplaneError::Internal(format!("no package {}", package)))?;
        let mut found = false;
        for r in list.iter_mut() {
            if r.revision == revision {
                r.state = PackageRevisionState::Active;
                found = true;
            } else {
                r.state = PackageRevisionState::Inactive;
            }
        }
        if !found {
            return Err(CrossplaneError::Internal(format!(
                "revision {} not present for {}",
                revision, package
            )));
        }
        Ok(())
    }

    pub fn deactivate_all(&self, package: &str) {
        if let Some(mut list) = self.revisions.get_mut(package) {
            for r in list.iter_mut() {
                r.state = PackageRevisionState::Inactive;
            }
        }
    }

    pub fn active(&self, package: &str) -> Option<PackageRevision> {
        self.revisions.get(package).and_then(|l| {
            l.iter()
                .find(|r| r.state == PackageRevisionState::Active)
                .cloned()
        })
    }

    pub fn history(&self, package: &str) -> Vec<PackageRevision> {
        self.revisions
            .get(package)
            .map(|l| l.clone())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_first_active() {
        let t = PackageRevisionTracker::new();
        let r = t.record("p", "v1");
        assert_eq!(r.state, PackageRevisionState::Active);
    }

    #[test]
    fn record_demotes_prior() {
        let t = PackageRevisionTracker::new();
        t.record("p", "v1");
        t.record("p", "v2");
        let h = t.history("p");
        assert_eq!(h[0].state, PackageRevisionState::Inactive);
        assert_eq!(h[1].state, PackageRevisionState::Active);
    }

    #[test]
    fn history_limit_caps() {
        let t = PackageRevisionTracker::with_history_limit(1);
        for v in &["v1", "v2", "v3", "v4"] {
            t.record("p", v);
        }
        assert!(t.history("p").len() <= 2);
    }

    #[test]
    fn manual_activate_older() {
        let t = PackageRevisionTracker::with_history_limit(5);
        t.record("p", "v1");
        t.record("p", "v2");
        t.activate("p", "v1").unwrap();
        assert_eq!(t.active("p").unwrap().revision, "v1");
    }

    #[test]
    fn activate_unknown_errors() {
        let t = PackageRevisionTracker::new();
        assert!(t.activate("p", "v1").is_err());
    }

    #[test]
    fn activate_unknown_revision_errors() {
        let t = PackageRevisionTracker::new();
        t.record("p", "v1");
        assert!(t.activate("p", "v999").is_err());
    }

    #[test]
    fn deactivate_all_then_no_active() {
        let t = PackageRevisionTracker::new();
        t.record("p", "v1");
        t.deactivate_all("p");
        assert!(t.active("p").is_none());
    }
}
