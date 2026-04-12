//! In-memory infrastructure state store with locking and history.

use crate::intent::detect_drift;
use crate::models::{DriftReport, InfraResource, InfraState, ResourceState};
use anyhow::{bail, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use tracing::info;

/// Maximum number of state snapshots retained in history.
const MAX_HISTORY: usize = 50;

/// A versioned snapshot of the infrastructure state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateSnapshot {
    pub version: u64,
    pub state: InfraState,
    pub comment: String,
    pub taken_at: chrono::DateTime<Utc>,
}

/// In-memory state store with optimistic locking and history.
#[derive(Debug)]
pub struct InfraStateStore {
    pub current: InfraState,
    history: VecDeque<StateSnapshot>,
}

impl Default for InfraStateStore {
    fn default() -> Self {
        Self {
            current: InfraState::default(),
            history: VecDeque::new(),
        }
    }
}

impl InfraStateStore {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Locking ───────────────────────────────────────────────────────────────

    /// Acquire a write lock on the state.
    pub fn lock(&mut self, holder: impl Into<String>) -> Result<()> {
        if self.current.locked {
            bail!(
                "State is already locked by '{}'",
                self.current.lock_holder.as_deref().unwrap_or("unknown")
            );
        }
        self.current.locked = true;
        self.current.lock_holder = Some(holder.into());
        info!(holder = ?self.current.lock_holder, "State locked");
        Ok(())
    }

    /// Release the write lock.
    pub fn unlock(&mut self) -> Result<()> {
        if !self.current.locked {
            bail!("State is not locked");
        }
        info!(holder = ?self.current.lock_holder, "State unlocked");
        self.current.locked = false;
        self.current.lock_holder = None;
        Ok(())
    }

    // ── Mutations ─────────────────────────────────────────────────────────────

    /// Snapshot current state before a mutation, then apply `f`.
    pub fn with_snapshot<F>(&mut self, comment: impl Into<String>, f: F) -> Result<()>
    where
        F: FnOnce(&mut InfraState) -> Result<()>,
    {
        // Snapshot before.
        let snap = StateSnapshot {
            version: self.current.version,
            state: self.current.clone(),
            comment: comment.into(),
            taken_at: Utc::now(),
        };
        if self.history.len() >= MAX_HISTORY {
            self.history.pop_front();
        }
        self.history.push_back(snap);

        f(&mut self.current)?;
        self.current.version += 1;
        self.current.last_synced = Some(Utc::now());
        Ok(())
    }

    /// Upsert a resource into desired state.
    pub fn upsert_desired(&mut self, resource: InfraResource, comment: impl Into<String>) -> Result<()> {
        let name = resource.name.clone();
        self.with_snapshot(comment, |state| {
            if let Some(existing) = state.desired.iter_mut().find(|r| r.name == name) {
                *existing = resource;
            } else {
                state.desired.push(resource);
            }
            Ok(())
        })
    }

    /// Remove a resource from desired state by name.
    pub fn remove_desired(&mut self, name: &str, comment: impl Into<String>) -> Result<()> {
        self.with_snapshot(comment, |state| {
            state.desired.retain(|r| r.name != name);
            Ok(())
        })
    }

    /// Update the actual (observed) state for a resource.
    pub fn update_actual(&mut self, resource: InfraResource) -> Result<()> {
        let name = resource.name.clone();
        self.with_snapshot(format!("sync actual state for '{}'", name), |state| {
            if let Some(existing) = state.actual.iter_mut().find(|r| r.name == name) {
                *existing = resource;
            } else {
                state.actual.push(resource);
            }
            Ok(())
        })
    }

    // ── Drift Detection ───────────────────────────────────────────────────────

    /// Compute current drift between desired and actual state.
    pub fn detect_drift(&self) -> DriftReport {
        detect_drift(&self.current)
    }

    // ── Import ────────────────────────────────────────────────────────────────

    /// Import an externally-managed resource into desired state.
    pub fn import_resource(
        &mut self,
        mut resource: InfraResource,
        remote_id: impl Into<String>,
    ) -> Result<()> {
        let remote_id = remote_id.into();
        let name = resource.name.clone();
        resource.remote_id = Some(remote_id.clone());
        resource.state = ResourceState::Synced;

        info!(name = %name, remote_id = %remote_id, "Importing resource");

        // Add to both desired and actual (it already exists remotely).
        let actual = resource.clone();
        self.with_snapshot(format!("import '{}'", name), |state| {
            if state.desired.iter().any(|r| r.name == name) {
                bail!("Resource '{}' already exists in desired state", name);
            }
            state.desired.push(resource);
            state.actual.push(actual);
            Ok(())
        })
    }

    // ── History ───────────────────────────────────────────────────────────────

    /// Return the state history (newest first).
    pub fn state_history(&self) -> Vec<&StateSnapshot> {
        self.history.iter().rev().collect()
    }

    /// Rollback to a specific version.
    pub fn rollback_to_version(&mut self, version: u64) -> Result<()> {
        let snap = self
            .history
            .iter()
            .rev()
            .find(|s| s.version == version)
            .cloned();

        match snap {
            None => bail!("No snapshot found for version {}", version),
            Some(s) => {
                info!(version = version, "Rolling back state");
                self.current = s.state;
                Ok(())
            }
        }
    }
}
