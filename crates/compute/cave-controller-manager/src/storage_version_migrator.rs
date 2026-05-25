// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! StorageVersionMigration reconciler.
//!
//! Cite: `pkg/controller/storageversionmigrator/` (v1.36.0).
//!
//! Walks every instance of a target GVR (group / version / resource)
//! and re-issues a PUT to upgrade the at-rest storage version. The
//! upgrade is a no-op touch — fields are not rewritten — but the round-
//! trip forces the apiserver to re-serialize each object under the new
//! storage encoding. Used after a CRD or built-in resource is moved
//! from one storage version to another and the operator wants the
//! etcd payload converted in bulk.
//!
//! State machine:
//!
//! ```text
//!     Pending  ──►  Running  ──►  Succeeded
//!                       │
//!                       └─►  Failed (with reason)
//! ```
//!
//! Progress is reported as a `(total, completed, errors)` triple so the
//! cave-portal `/admin/cm/storage-version-migrator/` page can render a
//! per-CRD progress bar.
//!
//! The reconciler does not perform IO directly; it consumes a
//! [`MigrationSource`] trait object that enumerates instance keys and
//! reissues the touch PUT. The default in-process implementation is
//! [`InMemoryMigrationSource`] for tests; cave-runtime wires in an
//! `ApiserverClient`-backed implementation at the call site.

use crate::types::Cite;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::Mutex;

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/storageversionmigrator/storageversionmigrator_controller.go",
    "SVMController.sync",
);

/// Target group/version/resource the migration walks.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TargetGvr {
    pub group: String,
    pub version: String,
    pub resource: String,
}

impl TargetGvr {
    pub fn new(group: &str, version: &str, resource: &str) -> Self {
        Self {
            group: group.to_string(),
            version: version.to_string(),
            resource: resource.to_string(),
        }
    }

    pub fn display(&self) -> String {
        if self.group.is_empty() {
            format!("{}/{}", self.version, self.resource)
        } else {
            format!("{}/{}/{}", self.group, self.version, self.resource)
        }
    }
}

/// CRD-style resource the user creates to request a migration. Mirrors
/// upstream `storage.k8s.io/v1.StorageVersionMigration`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageVersionMigration {
    pub name: String,
    pub target: TargetGvr,
    /// Resource version observed at the start of the migration; used
    /// to skip objects that have already been touched by another path.
    pub resource_version_floor: Option<String>,
}

/// Lifecycle phase. Matches the upstream `.status.conditions[].type`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Pending,
    Running,
    Succeeded,
    Failed(String),
}

/// Mutable progress counters surfaced on `.status`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub total: u64,
    pub completed: u64,
    pub errors: u64,
}

impl Progress {
    pub fn percent(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        (self.completed as f64) / (self.total as f64)
    }
}

/// Status block — phase + progress + transition timestamps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationStatus {
    pub phase: Phase,
    pub progress: Progress,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

impl Default for MigrationStatus {
    fn default() -> Self {
        Self {
            phase: Phase::Pending,
            progress: Progress::default(),
            started_at: None,
            completed_at: None,
        }
    }
}

/// Pluggable backend the reconciler talks to. Real implementation
/// shells out to the apiserver; tests inject [`InMemoryMigrationSource`].
#[async_trait]
pub trait MigrationSource: Send + Sync {
    /// List every instance key (namespace, name) for `target`.
    async fn list(&self, target: &TargetGvr) -> Result<Vec<(String, String)>, String>;
    /// Re-issue an identity PUT for one instance — re-encoding to the
    /// new storage version. Returns `Ok(true)` on success, `Ok(false)`
    /// if the object already moved storage version (count it, skip it).
    async fn touch(
        &self,
        target: &TargetGvr,
        namespace: &str,
        name: &str,
    ) -> Result<bool, String>;
}

/// In-memory fake source used by tests and the cave-portal preview.
pub struct InMemoryMigrationSource {
    inner: Arc<Mutex<InMemoryState>>,
}

#[derive(Debug, Default)]
struct InMemoryState {
    instances: BTreeMap<String, Vec<(String, String)>>,
    poison: Vec<String>,
    already_migrated: Vec<String>,
}

impl InMemoryMigrationSource {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(InMemoryState::default())),
        }
    }

    pub async fn seed(&self, target: &TargetGvr, items: Vec<(&str, &str)>) {
        let mut g = self.inner.lock().await;
        let key = target.display();
        let entry = g.instances.entry(key).or_default();
        for (ns, name) in items {
            entry.push((ns.to_string(), name.to_string()));
        }
    }

    pub async fn poison(&self, name: &str) {
        let mut g = self.inner.lock().await;
        g.poison.push(name.to_string());
    }

    pub async fn already_migrated(&self, name: &str) {
        let mut g = self.inner.lock().await;
        g.already_migrated.push(name.to_string());
    }
}

impl Default for InMemoryMigrationSource {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MigrationSource for InMemoryMigrationSource {
    async fn list(&self, target: &TargetGvr) -> Result<Vec<(String, String)>, String> {
        let g = self.inner.lock().await;
        Ok(g.instances
            .get(&target.display())
            .cloned()
            .unwrap_or_default())
    }

    async fn touch(
        &self,
        _target: &TargetGvr,
        _namespace: &str,
        name: &str,
    ) -> Result<bool, String> {
        let g = self.inner.lock().await;
        if g.poison.iter().any(|p| p == name) {
            return Err(format!("synthetic-poison touch failed for {name}"));
        }
        if g.already_migrated.iter().any(|p| p == name) {
            return Ok(false);
        }
        Ok(true)
    }
}

/// Driver. Holds the spec + status + a scratch queue of items the
/// `Running` phase still has to touch.
pub struct StorageVersionMigrator {
    spec: StorageVersionMigration,
    status: MigrationStatus,
    source: Arc<dyn MigrationSource>,
    pending_items: Vec<(String, String)>,
}

impl StorageVersionMigrator {
    pub fn new(spec: StorageVersionMigration, source: Arc<dyn MigrationSource>) -> Self {
        Self {
            spec,
            status: MigrationStatus::default(),
            source,
            pending_items: Vec::new(),
        }
    }

    pub fn status(&self) -> &MigrationStatus {
        &self.status
    }

    pub fn spec(&self) -> &StorageVersionMigration {
        &self.spec
    }

    pub fn pending_items_len(&self) -> usize {
        self.pending_items.len()
    }

    /// One reconciliation step: Pending → Running (list) → Running
    /// (touch a batch) → Succeeded / Failed.
    pub async fn step(&mut self) -> Phase {
        match self.status.phase.clone() {
            Phase::Pending => match self.source.list(&self.spec.target).await {
                Ok(items) => {
                    self.status.phase = Phase::Running;
                    self.status.progress.total = items.len() as u64;
                    self.status.started_at = Some(Utc::now());
                    self.pending_items = items;
                    // If list returned empty, advance immediately so
                    // `step()` is idempotent for zero-instance targets.
                    if self.pending_items.is_empty() {
                        self.status.phase = Phase::Succeeded;
                        self.status.completed_at = Some(Utc::now());
                    }
                    self.status.phase.clone()
                }
                Err(e) => {
                    self.status.phase = Phase::Failed(format!("list failed: {e}"));
                    self.status.completed_at = Some(Utc::now());
                    self.status.phase.clone()
                }
            },
            Phase::Running => {
                const BATCH: usize = 64;
                let take = self.pending_items.len().min(BATCH);
                let chunk: Vec<(String, String)> =
                    self.pending_items.drain(..take).collect();
                for (ns, name) in chunk {
                    match self.source.touch(&self.spec.target, &ns, &name).await {
                        Ok(_) => self.status.progress.completed += 1,
                        Err(_) => self.status.progress.errors += 1,
                    }
                }
                if self.pending_items.is_empty() {
                    if self.status.progress.errors == 0 {
                        self.status.phase = Phase::Succeeded;
                    } else {
                        self.status.phase = Phase::Failed(format!(
                            "{} object(s) failed to migrate",
                            self.status.progress.errors
                        ));
                    }
                    self.status.completed_at = Some(Utc::now());
                }
                self.status.phase.clone()
            }
            other => other,
        }
    }

    /// Pump `step()` until terminal (Succeeded / Failed). Bounded by
    /// `max_iters` for test determinism.
    pub async fn run_to_completion(&mut self, max_iters: usize) -> Phase {
        for _ in 0..max_iters {
            let p = self.step().await;
            if matches!(p, Phase::Succeeded | Phase::Failed(_)) {
                return p;
            }
        }
        self.status.phase.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn sample_target() -> TargetGvr {
        TargetGvr::new("apps", "v1", "deployments")
    }

    fn sample_spec() -> StorageVersionMigration {
        StorageVersionMigration {
            name: "deploy-apps-v1".to_string(),
            target: sample_target(),
            resource_version_floor: None,
        }
    }

    #[tokio::test]
    async fn step_from_pending_lists_instances_and_moves_to_running() {
        let (_c, _t) = test_ctx!(
            "pkg/controller/storageversionmigrator/storageversionmigrator_controller.go",
            "Pending->Running",
            "tenant-svm-pending"
        );
        let src = Arc::new(InMemoryMigrationSource::new());
        src.seed(
            &sample_target(),
            vec![("default", "d1"), ("default", "d2"), ("kube-system", "d3")],
        )
        .await;
        let mut m = StorageVersionMigrator::new(sample_spec(), src);
        let phase = m.step().await;
        assert_eq!(phase, Phase::Running);
        assert_eq!(m.status().progress.total, 3);
        assert!(m.status().started_at.is_some());
    }

    #[tokio::test]
    async fn run_to_completion_succeeds_when_all_touches_ok() {
        let src = Arc::new(InMemoryMigrationSource::new());
        src.seed(
            &sample_target(),
            vec![("a", "x"), ("b", "y"), ("c", "z")],
        )
        .await;
        let mut m = StorageVersionMigrator::new(sample_spec(), src);
        let phase = m.run_to_completion(8).await;
        assert_eq!(phase, Phase::Succeeded);
        assert_eq!(m.status().progress.completed, 3);
        assert_eq!(m.status().progress.errors, 0);
        assert!(m.status().completed_at.is_some());
    }

    #[tokio::test]
    async fn touch_failure_marks_run_failed_with_error_count() {
        let src = Arc::new(InMemoryMigrationSource::new());
        src.seed(
            &sample_target(),
            vec![("a", "ok-1"), ("a", "poisoned"), ("a", "ok-2")],
        )
        .await;
        src.poison("poisoned").await;
        let mut m = StorageVersionMigrator::new(sample_spec(), src);
        let phase = m.run_to_completion(8).await;
        assert!(matches!(phase, Phase::Failed(_)));
        assert_eq!(m.status().progress.completed, 2);
        assert_eq!(m.status().progress.errors, 1);
    }

    #[tokio::test]
    async fn already_migrated_objects_count_but_do_not_error() {
        let src = Arc::new(InMemoryMigrationSource::new());
        src.seed(&sample_target(), vec![("a", "fresh"), ("a", "stale")])
            .await;
        src.already_migrated("stale").await;
        let mut m = StorageVersionMigrator::new(sample_spec(), src);
        let phase = m.run_to_completion(8).await;
        assert_eq!(phase, Phase::Succeeded);
        assert_eq!(m.status().progress.completed, 2);
        assert_eq!(m.status().progress.errors, 0);
    }

    #[tokio::test]
    async fn empty_target_succeeds_immediately() {
        let src = Arc::new(InMemoryMigrationSource::new());
        let mut m = StorageVersionMigrator::new(sample_spec(), src);
        let phase = m.run_to_completion(4).await;
        assert_eq!(phase, Phase::Succeeded);
        assert_eq!(m.status().progress.total, 0);
        assert_eq!(m.status().progress.completed, 0);
    }

    #[test]
    fn progress_percent_reflects_completed_over_total() {
        let mut p = Progress::default();
        assert_eq!(p.percent(), 0.0);
        p.total = 4;
        p.completed = 1;
        assert!((p.percent() - 0.25).abs() < 1e-9);
        p.completed = 4;
        assert!((p.percent() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn target_gvr_display_uses_slash_separator() {
        assert_eq!(
            TargetGvr::new("apps", "v1", "deployments").display(),
            "apps/v1/deployments"
        );
        assert_eq!(
            TargetGvr::new("", "v1", "configmaps").display(),
            "v1/configmaps"
        );
    }

    #[tokio::test]
    async fn terminal_phase_is_sticky_under_repeated_step() {
        let src = Arc::new(InMemoryMigrationSource::new());
        src.seed(&sample_target(), vec![("a", "x")]).await;
        let mut m = StorageVersionMigrator::new(sample_spec(), src);
        let p1 = m.run_to_completion(8).await;
        assert_eq!(p1, Phase::Succeeded);
        let p2 = m.step().await;
        assert_eq!(p2, Phase::Succeeded);
        assert_eq!(m.status().progress.completed, 1);
    }

    #[tokio::test]
    async fn migration_status_serialises_via_serde_json() {
        let src = Arc::new(InMemoryMigrationSource::new());
        src.seed(&sample_target(), vec![("a", "x"), ("a", "y")])
            .await;
        let mut m = StorageVersionMigrator::new(sample_spec(), src);
        let _ = m.run_to_completion(8).await;
        let v = serde_json::to_value(m.status()).unwrap();
        assert!(v.get("phase").is_some());
        assert_eq!(v["progress"]["total"], 2);
        assert_eq!(v["progress"]["completed"], 2);
    }
}
