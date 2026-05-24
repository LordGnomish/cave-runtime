// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! ProviderRevision rollout: active revision tracking + deactivation +
//! revision history limit.
//!
//! Upstream: internal/controller/pkg/revision/reconciler.go

use crate::error::{CrossplaneError, CrossplaneResult};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};

const DEFAULT_HISTORY_LIMIT: usize = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RevisionState {
    Active,
    Inactive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRevision {
    pub package: String,
    pub revision: String,
    pub state: RevisionState,
    pub created_at: DateTime<Utc>,
}

#[derive(Default)]
pub struct ProviderRevisionStore {
    /// package_name → ordered revisions (newest at end)
    revisions: DashMap<String, Vec<ProviderRevision>>,
    history_limit: usize,
}

impl ProviderRevisionStore {
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

    /// Append a new revision and mark it active; demote all prior to Inactive.
    pub fn append(&self, package: &str, revision: &str) -> CrossplaneResult<ProviderRevision> {
        let new = ProviderRevision {
            package: package.to_string(),
            revision: revision.to_string(),
            state: RevisionState::Active,
            created_at: Utc::now(),
        };
        let mut list = self.revisions.entry(package.to_string()).or_default();
        for r in list.iter_mut() {
            r.state = RevisionState::Inactive;
        }
        list.push(new.clone());
        let total = list.len();
        // Trim history beyond inactive limit; keep the active one.
        let inactive_keep = self.history_limit.max(1);
        // Inactive count is total - 1 (one is active).
        if total > inactive_keep + 1 {
            let drop = total - (inactive_keep + 1);
            list.drain(0..drop);
        }
        Ok(new)
    }

    /// Deactivate the active revision (admin override). Returns the demoted rev.
    pub fn deactivate_active(&self, package: &str) -> CrossplaneResult<ProviderRevision> {
        let mut list = self
            .revisions
            .get_mut(package)
            .ok_or_else(|| CrossplaneError::ProviderNotFound(package.to_owned()))?;
        let idx = list
            .iter()
            .position(|r| r.state == RevisionState::Active)
            .ok_or_else(|| CrossplaneError::Internal(format!("no active revision for {}", package)))?;
        list[idx].state = RevisionState::Inactive;
        Ok(list[idx].clone())
    }

    pub fn active(&self, package: &str) -> Option<ProviderRevision> {
        self.revisions
            .get(package)
            .and_then(|l| l.iter().find(|r| r.state == RevisionState::Active).cloned())
    }

    pub fn list(&self, package: &str) -> Vec<ProviderRevision> {
        self.revisions
            .get(package)
            .map(|l| l.clone())
            .unwrap_or_default()
    }

    pub fn history_limit(&self) -> usize {
        self.history_limit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_append_active() {
        let s = ProviderRevisionStore::new();
        let r = s.append("p", "v0.1.0").unwrap();
        assert_eq!(r.state, RevisionState::Active);
    }

    #[test]
    fn second_append_demotes_prior() {
        let s = ProviderRevisionStore::new();
        s.append("p", "v0.1.0").unwrap();
        s.append("p", "v0.2.0").unwrap();
        let list = s.list("p");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].state, RevisionState::Inactive);
        assert_eq!(list[1].state, RevisionState::Active);
    }

    #[test]
    fn history_limit_trims() {
        let s = ProviderRevisionStore::with_history_limit(1);
        s.append("p", "v0.1.0").unwrap();
        s.append("p", "v0.2.0").unwrap();
        s.append("p", "v0.3.0").unwrap();
        s.append("p", "v0.4.0").unwrap();
        let list = s.list("p");
        assert!(list.len() <= 2);
        assert_eq!(list.last().unwrap().revision, "v0.4.0");
    }

    #[test]
    fn deactivate_active_demotes() {
        let s = ProviderRevisionStore::new();
        s.append("p", "v0.1.0").unwrap();
        s.deactivate_active("p").unwrap();
        assert!(s.active("p").is_none());
    }

    #[test]
    fn deactivate_unknown_package_errors() {
        let s = ProviderRevisionStore::new();
        assert!(s.deactivate_active("none").is_err());
    }

    #[test]
    fn active_returns_latest() {
        let s = ProviderRevisionStore::new();
        s.append("p", "v0.1.0").unwrap();
        s.append("p", "v0.2.0").unwrap();
        assert_eq!(s.active("p").unwrap().revision, "v0.2.0");
    }

    #[test]
    fn history_limit_default_is_one() {
        let s = ProviderRevisionStore::new();
        assert_eq!(s.history_limit(), 1);
    }
}
