// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-memory CRUD store for Applications, ApplicationSets and AppProjects.
//!
//! Backs the routes layer until cave-db wiring lands in Phase 2. Concurrent
//! callers see a consistent snapshot through `Arc<RwLock<…>>`.

use crate::appset::ApplicationSet;
use crate::error::DeployError;
use crate::models::{Application, ApplicationStatus, RevisionHistory};
use crate::rbac::AppProject;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

pub const MODULE_NAME: &str = "deploy";

#[derive(Default)]
pub struct DeployStore {
    inner: Arc<RwLock<StoreInner>>,
}

#[derive(Default)]
struct StoreInner {
    applications: HashMap<String, Application>,
    appsets: HashMap<String, ApplicationSet>,
    projects: HashMap<String, AppProject>,
}

impl DeployStore {
    pub fn new() -> Self {
        Self::default()
    }

    // ─── Applications ─────────────────────────────────────────────────────

    pub fn list_applications(&self) -> Vec<Application> {
        let s = self.inner.read().unwrap();
        let mut apps: Vec<Application> = s.applications.values().cloned().collect();
        apps.sort_by(|a, b| a.name.cmp(&b.name));
        apps
    }

    pub fn get_application(&self, name: &str) -> Option<Application> {
        self.inner.read().unwrap().applications.get(name).cloned()
    }

    pub fn create_application(&self, app: Application) -> Result<(), DeployError> {
        let mut s = self.inner.write().unwrap();
        if s.applications.contains_key(&app.name) {
            return Err(DeployError::AlreadyExists(app.name.clone()));
        }
        s.applications.insert(app.name.clone(), app);
        Ok(())
    }

    pub fn update_application_status(
        &self,
        name: &str,
        status: ApplicationStatus,
    ) -> Result<(), DeployError> {
        let mut s = self.inner.write().unwrap();
        let app = s
            .applications
            .get_mut(name)
            .ok_or_else(|| DeployError::NotFound(name.to_string()))?;
        app.status = Some(status);
        app.updated_at = chrono::Utc::now();
        Ok(())
    }

    pub fn delete_application(&self, name: &str) -> Result<(), DeployError> {
        let mut s = self.inner.write().unwrap();
        s.applications
            .remove(name)
            .ok_or_else(|| DeployError::NotFound(name.to_string()))?;
        Ok(())
    }

    // ─── Revision history (mutates app.status.history) ────────────────────

    pub fn append_revision(
        &self,
        name: &str,
        entry: RevisionHistory,
    ) -> Result<(), DeployError> {
        let mut s = self.inner.write().unwrap();
        let app = s
            .applications
            .get_mut(name)
            .ok_or_else(|| DeployError::NotFound(name.to_string()))?;
        let status = app.status.get_or_insert_with(default_status);
        status.history.push(entry);
        Ok(())
    }

    pub fn rollback_to_history_id(
        &self,
        name: &str,
        history_id: u64,
    ) -> Result<RevisionHistory, DeployError> {
        let s = self.inner.read().unwrap();
        let app = s
            .applications
            .get(name)
            .ok_or_else(|| DeployError::NotFound(name.to_string()))?;
        let status = app
            .status
            .as_ref()
            .ok_or_else(|| DeployError::NotFound(format!("{name}.status")))?;
        status
            .history
            .iter()
            .find(|h| h.id == history_id)
            .cloned()
            .ok_or_else(|| DeployError::NotFound(format!("history #{history_id}")))
    }

    // ─── ApplicationSet ───────────────────────────────────────────────────

    pub fn create_appset(&self, set: ApplicationSet) -> Result<(), DeployError> {
        let mut s = self.inner.write().unwrap();
        if s.appsets.contains_key(&set.name) {
            return Err(DeployError::AlreadyExists(set.name.clone()));
        }
        s.appsets.insert(set.name.clone(), set);
        Ok(())
    }

    pub fn list_appsets(&self) -> Vec<ApplicationSet> {
        self.inner.read().unwrap().appsets.values().cloned().collect()
    }

    pub fn get_appset(&self, name: &str) -> Option<ApplicationSet> {
        self.inner.read().unwrap().appsets.get(name).cloned()
    }

    pub fn delete_appset(&self, name: &str) -> Result<(), DeployError> {
        self.inner
            .write()
            .unwrap()
            .appsets
            .remove(name)
            .ok_or_else(|| DeployError::NotFound(name.to_string()))?;
        Ok(())
    }

    // ─── Projects ─────────────────────────────────────────────────────────

    pub fn upsert_project(&self, project: AppProject) {
        self.inner
            .write()
            .unwrap()
            .projects
            .insert(project.name.clone(), project);
    }

    pub fn get_project(&self, name: &str) -> Option<AppProject> {
        self.inner.read().unwrap().projects.get(name).cloned()
    }

    pub fn list_projects(&self) -> Vec<AppProject> {
        self.inner.read().unwrap().projects.values().cloned().collect()
    }
}

fn default_status() -> ApplicationStatus {
    use crate::models::{HealthCondition, HealthStatus, SyncCondition, SyncStatus};
    ApplicationStatus {
        health: HealthCondition {
            status: HealthStatus::Unknown,
            message: None,
        },
        sync: SyncCondition {
            status: SyncStatus::Unknown,
            revision: String::new(),
            revisions: vec![],
        },
        resources: vec![],
        history: vec![],
        conditions: vec![],
        observed_at: None,
        reconciled_at: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        ApplicationSource, ApplicationSpec, Destination, HealthCondition, HealthStatus,
        ResourceTracking, SyncCondition, SyncStatus,
    };
    use chrono::Utc;
    use uuid::Uuid;

    fn make_app(name: &str) -> Application {
        Application {
            id: Uuid::new_v4(),
            name: name.into(),
            namespace: "argocd".into(),
            spec: ApplicationSpec {
                source: ApplicationSource {
                    repo_url: "https://github.com/example/repo.git".into(),
                    target_revision: Some("main".into()),
                    path: Some("manifests/".into()),
                    helm: None,
                    kustomize: None,
                    directory: None,
                },
                sources: vec![],
                destination: Destination {
                    server: "https://kubernetes.default.svc".into(),
                    name: None,
                    namespace: "default".into(),
                },
                project: "default".into(),
                sync_policy: None,
                ignored_differences: None,
                info: None,
                revision_history_limit: None,
            },
            status: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            labels: Default::default(),
            annotations: Default::default(),
            tracking: ResourceTracking::default(),
        }
    }

    #[test]
    fn test_in_memory_crud_create_get_delete() {
        let store = DeployStore::new();
        store.create_application(make_app("myapp")).unwrap();
        assert!(store.get_application("myapp").is_some());
        assert_eq!(store.list_applications().len(), 1);
        store.delete_application("myapp").unwrap();
        assert!(store.get_application("myapp").is_none());
        assert_eq!(store.list_applications().len(), 0);
    }

    #[test]
    fn test_in_memory_duplicate_create_fails() {
        let store = DeployStore::new();
        store.create_application(make_app("myapp")).unwrap();
        let err = store.create_application(make_app("myapp")).unwrap_err();
        assert!(matches!(err, DeployError::AlreadyExists(_)));
    }

    #[test]
    fn test_in_memory_update_status() {
        let store = DeployStore::new();
        store.create_application(make_app("app1")).unwrap();
        let new_status = ApplicationStatus {
            health: HealthCondition {
                status: HealthStatus::Degraded,
                message: Some("CrashLoopBackOff".into()),
            },
            sync: SyncCondition {
                status: SyncStatus::Synced,
                revision: "abc".into(),
                revisions: vec![],
            },
            resources: vec![],
            history: vec![],
            conditions: vec![],
            observed_at: None,
            reconciled_at: Some(Utc::now()),
        };
        store.update_application_status("app1", new_status).unwrap();
        let updated = store.get_application("app1").unwrap();
        assert_eq!(updated.status.unwrap().health.status, HealthStatus::Degraded);
    }

    #[test]
    fn test_in_memory_delete_not_found() {
        let store = DeployStore::new();
        let err = store.delete_application("nonexistent").unwrap_err();
        assert!(matches!(err, DeployError::NotFound(_)));
    }

    #[test]
    fn test_in_memory_rollback_via_history() {
        let store = DeployStore::new();
        let app = make_app("rollback-test");
        let source = app.spec.source.clone();
        store.create_application(app).unwrap();

        let h1 = RevisionHistory {
            id: 1,
            revision: "abc111".into(),
            deployed_at: Utc::now(),
            initiated_by: "ci-bot".into(),
            source: source.clone(),
        };
        let h2 = RevisionHistory {
            id: 2,
            revision: "def222".into(),
            deployed_at: Utc::now(),
            initiated_by: "alice".into(),
            source,
        };
        store.append_revision("rollback-test", h1).unwrap();
        store.append_revision("rollback-test", h2).unwrap();

        let target = store.rollback_to_history_id("rollback-test", 1).unwrap();
        assert_eq!(target.revision, "abc111");
    }

    #[test]
    fn rollback_unknown_history_fails() {
        let store = DeployStore::new();
        store.create_application(make_app("z")).unwrap();
        let err = store.rollback_to_history_id("z", 99).unwrap_err();
        assert!(matches!(err, DeployError::NotFound(_)));
    }
}
