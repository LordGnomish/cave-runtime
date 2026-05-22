// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Argo CD wrap — native re-implementation of the deployment timeline,
//! sync status, and rollback action.
//!
//! The portal **never** redirects to Argo CD's own web UI. cave-portal-api
//! brokers the data and this module models the views the renderer paints.

use super::ViewPersona;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum SyncStatus {
    Synced,
    OutOfSync,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum HealthStatus {
    Healthy,
    Progressing,
    Suspended,
    Degraded,
    Missing,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Application {
    pub name: String,
    pub tenant: String,
    pub project: String,
    pub repo_url: String,
    pub target_revision: String,
    pub current_revision: String,
    pub sync: SyncStatus,
    pub health: HealthStatus,
    pub last_sync_at: Option<String>,
    pub history: Vec<RevisionEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevisionEvent {
    pub revision: String,
    pub deployed_at: String,
    pub deployer: String,
    pub action: RevisionAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisionAction {
    Sync,
    Rollback,
    AutoSync,
    Drift,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ArgoError {
    #[error("application {0:?} not found")]
    NotFound(String),
    #[error("rollback at revision 0 not allowed")]
    NoPriorRevision,
    #[error("forbidden for persona {0:?}")]
    Forbidden(&'static str),
    #[error("application is already in sync")]
    AlreadyInSync,
}

impl Application {
    pub fn new(name: impl Into<String>, tenant: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            tenant: tenant.into(),
            project: "default".into(),
            repo_url: String::new(),
            target_revision: "HEAD".into(),
            current_revision: String::new(),
            sync: SyncStatus::Unknown,
            health: HealthStatus::Unknown,
            last_sync_at: None,
            history: Vec::new(),
        }
    }

    pub fn record_sync(
        &mut self,
        revision: impl Into<String>,
        deployer: impl Into<String>,
        action: RevisionAction,
    ) {
        let rev: String = revision.into();
        self.history.push(RevisionEvent {
            revision: rev.clone(),
            deployed_at: "1970-01-01T00:00:00Z".into(),
            deployer: deployer.into(),
            action,
        });
        self.current_revision = rev;
        self.last_sync_at = Some("1970-01-01T00:00:00Z".into());
        self.sync = if self.current_revision == self.target_revision {
            SyncStatus::Synced
        } else {
            SyncStatus::OutOfSync
        };
    }

    pub fn rollback(&mut self, deployer: impl Into<String>) -> Result<RevisionEvent, ArgoError> {
        if self.history.len() < 2 {
            return Err(ArgoError::NoPriorRevision);
        }
        let prev = self.history[self.history.len() - 2].clone();
        let event = RevisionEvent {
            revision: prev.revision.clone(),
            deployed_at: "1970-01-01T00:00:00Z".into(),
            deployer: deployer.into(),
            action: RevisionAction::Rollback,
        };
        self.history.push(event.clone());
        self.current_revision = prev.revision;
        self.sync = if self.current_revision == self.target_revision {
            SyncStatus::Synced
        } else {
            SyncStatus::OutOfSync
        };
        Ok(event)
    }

    pub fn timeline(&self) -> Vec<&RevisionEvent> {
        let mut out: Vec<&RevisionEvent> = self.history.iter().collect();
        out.reverse();
        out
    }

    pub fn drift_count(&self) -> usize {
        self.history
            .iter()
            .filter(|e| matches!(e.action, RevisionAction::Drift))
            .count()
    }

    /// Tenant + Operator + Admin can all see app state. Mutating actions
    /// require persona check via [`Self::can_mutate`].
    pub fn can_view(&self, _persona: ViewPersona) -> bool {
        true
    }

    pub fn can_mutate(&self, persona: ViewPersona) -> bool {
        // Tenants can sync/rollback their own; operators+admins can mutate any.
        matches!(
            persona,
            ViewPersona::Tenant | ViewPersona::Operator | ViewPersona::Admin
        )
    }
}

/// Per-tenant collection of applications. The portal renders this as the
/// "Deployments" page that replaces the Argo CD app list.
#[derive(Debug, Default)]
pub struct ArgoPlugin {
    apps: Vec<Application>,
}

impl ArgoPlugin {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert(&mut self, app: Application) {
        if let Some(idx) = self
            .apps
            .iter()
            .position(|a| a.name == app.name && a.tenant == app.tenant)
        {
            self.apps[idx] = app;
        } else {
            self.apps.push(app);
        }
    }

    pub fn find(&self, tenant: &str, name: &str) -> Option<&Application> {
        self.apps
            .iter()
            .find(|a| a.tenant == tenant && a.name == name)
    }

    pub fn find_mut(&mut self, tenant: &str, name: &str) -> Option<&mut Application> {
        self.apps
            .iter_mut()
            .find(|a| a.tenant == tenant && a.name == name)
    }

    pub fn list_for_tenant(&self, tenant: &str) -> Vec<&Application> {
        let mut out: Vec<&Application> = self.apps.iter().filter(|a| a.tenant == tenant).collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    pub fn count(&self) -> usize {
        self.apps.len()
    }

    pub fn synced_pct(&self, tenant: &str) -> u8 {
        let apps = self.list_for_tenant(tenant);
        if apps.is_empty() {
            return 0;
        }
        let synced = apps.iter().filter(|a| a.sync == SyncStatus::Synced).count();
        ((synced as u32 * 100) / apps.len() as u32) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_app(name: &str, tenant: &str) -> Application {
        let mut a = Application::new(name, tenant);
        a.target_revision = "v2".into();
        a
    }

    #[test]
    fn application_default_is_unknown() {
        let a = Application::new("a", "acme");
        assert_eq!(a.sync, SyncStatus::Unknown);
        assert_eq!(a.health, HealthStatus::Unknown);
        assert!(a.history.is_empty());
    }

    #[test]
    fn record_sync_to_target_marks_synced() {
        let mut a = make_app("a", "acme");
        a.record_sync("v2", "alice", RevisionAction::Sync);
        assert_eq!(a.sync, SyncStatus::Synced);
        assert_eq!(a.current_revision, "v2");
    }

    #[test]
    fn record_sync_to_other_marks_out_of_sync() {
        let mut a = make_app("a", "acme");
        a.record_sync("v1", "alice", RevisionAction::Sync);
        assert_eq!(a.sync, SyncStatus::OutOfSync);
    }

    #[test]
    fn record_sync_appends_to_history() {
        let mut a = make_app("a", "acme");
        a.record_sync("v1", "alice", RevisionAction::Sync);
        a.record_sync("v2", "bob", RevisionAction::AutoSync);
        assert_eq!(a.history.len(), 2);
        assert_eq!(a.history[0].revision, "v1");
        assert_eq!(a.history[1].deployer, "bob");
    }

    #[test]
    fn rollback_with_no_history_errors() {
        let mut a = make_app("a", "acme");
        let err = a.rollback("alice").unwrap_err();
        assert_eq!(err, ArgoError::NoPriorRevision);
    }

    #[test]
    fn rollback_with_one_revision_errors() {
        let mut a = make_app("a", "acme");
        a.record_sync("v1", "alice", RevisionAction::Sync);
        let err = a.rollback("alice").unwrap_err();
        assert_eq!(err, ArgoError::NoPriorRevision);
    }

    #[test]
    fn rollback_returns_to_previous_revision() {
        let mut a = make_app("a", "acme");
        a.record_sync("v1", "alice", RevisionAction::Sync);
        a.record_sync("v2", "bob", RevisionAction::Sync);
        let event = a.rollback("carol").unwrap();
        assert_eq!(event.revision, "v1");
        assert_eq!(event.action, RevisionAction::Rollback);
        assert_eq!(a.current_revision, "v1");
    }

    #[test]
    fn rollback_appends_to_history() {
        let mut a = make_app("a", "acme");
        a.record_sync("v1", "alice", RevisionAction::Sync);
        a.record_sync("v2", "bob", RevisionAction::Sync);
        a.rollback("carol").unwrap();
        assert_eq!(a.history.len(), 3);
        assert_eq!(a.history[2].action, RevisionAction::Rollback);
    }

    #[test]
    fn timeline_returns_newest_first() {
        let mut a = make_app("a", "acme");
        a.record_sync("v1", "alice", RevisionAction::Sync);
        a.record_sync("v2", "bob", RevisionAction::Sync);
        let t = a.timeline();
        assert_eq!(t[0].revision, "v2");
        assert_eq!(t[1].revision, "v1");
    }

    #[test]
    fn drift_count_counts_only_drift_actions() {
        let mut a = make_app("a", "acme");
        a.record_sync("v1", "alice", RevisionAction::Sync);
        a.record_sync("v1", "ops", RevisionAction::Drift);
        a.record_sync("v1", "ops", RevisionAction::Drift);
        a.record_sync("v2", "alice", RevisionAction::Sync);
        assert_eq!(a.drift_count(), 2);
    }

    #[test]
    fn can_view_open_to_all_personas() {
        let a = make_app("a", "acme");
        assert!(a.can_view(ViewPersona::Tenant));
        assert!(a.can_view(ViewPersona::Operator));
        assert!(a.can_view(ViewPersona::Admin));
    }

    #[test]
    fn can_mutate_open_to_all_personas() {
        let a = make_app("a", "acme");
        assert!(a.can_mutate(ViewPersona::Tenant));
        assert!(a.can_mutate(ViewPersona::Operator));
        assert!(a.can_mutate(ViewPersona::Admin));
    }

    #[test]
    fn plugin_upsert_inserts_new() {
        let mut p = ArgoPlugin::new();
        p.upsert(make_app("a", "acme"));
        assert_eq!(p.count(), 1);
    }

    #[test]
    fn plugin_upsert_replaces_existing() {
        let mut p = ArgoPlugin::new();
        let mut a = make_app("a", "acme");
        a.health = HealthStatus::Healthy;
        p.upsert(a);
        let mut a2 = make_app("a", "acme");
        a2.health = HealthStatus::Degraded;
        p.upsert(a2);
        assert_eq!(p.count(), 1);
        assert_eq!(p.find("acme", "a").unwrap().health, HealthStatus::Degraded);
    }

    #[test]
    fn plugin_find_by_tenant_and_name() {
        let mut p = ArgoPlugin::new();
        p.upsert(make_app("a", "acme"));
        p.upsert(make_app("a", "globex"));
        assert!(p.find("acme", "a").is_some());
        assert!(p.find("globex", "a").is_some());
        assert!(p.find("acme", "b").is_none());
    }

    #[test]
    fn plugin_list_for_tenant_filters_and_sorts() {
        let mut p = ArgoPlugin::new();
        p.upsert(make_app("zeta", "acme"));
        p.upsert(make_app("alpha", "acme"));
        p.upsert(make_app("a", "globex"));
        let out = p.list_for_tenant("acme");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].name, "alpha");
        assert_eq!(out[1].name, "zeta");
    }

    #[test]
    fn plugin_synced_pct_zero_when_empty() {
        let p = ArgoPlugin::new();
        assert_eq!(p.synced_pct("acme"), 0);
    }

    #[test]
    fn plugin_synced_pct_normal() {
        let mut p = ArgoPlugin::new();
        let mut a = make_app("a", "acme");
        a.record_sync("v2", "alice", RevisionAction::Sync); // synced
        p.upsert(a);
        let mut b = make_app("b", "acme");
        b.record_sync("v1", "bob", RevisionAction::Sync); // out of sync
        p.upsert(b);
        assert_eq!(p.synced_pct("acme"), 50);
    }

    #[test]
    fn sync_status_serializes_pascal_case() {
        let s = serde_json::to_string(&SyncStatus::OutOfSync).unwrap();
        assert_eq!(s, "\"OutOfSync\"");
    }

    #[test]
    fn revision_action_serializes_snake_case() {
        let s = serde_json::to_string(&RevisionAction::AutoSync).unwrap();
        assert_eq!(s, "\"auto_sync\"");
    }

    #[test]
    fn rollback_after_2_history_succeeds_and_3rd_event_logged() {
        let mut a = make_app("a", "acme");
        a.record_sync("v1", "x", RevisionAction::Sync);
        a.record_sync("v2", "x", RevisionAction::Sync);
        a.rollback("x").unwrap();
        let last = a.history.last().unwrap();
        assert_eq!(last.action, RevisionAction::Rollback);
        assert_eq!(last.revision, "v1");
    }

    #[test]
    fn record_sync_updates_last_sync_at() {
        let mut a = make_app("a", "acme");
        assert!(a.last_sync_at.is_none());
        a.record_sync("v1", "x", RevisionAction::Sync);
        assert!(a.last_sync_at.is_some());
    }
}
