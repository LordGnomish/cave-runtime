// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! State management — persistent tracking, locking, drift detection, import, history.

use crate::models::{DriftReport, InfraState, ResourceDrift, ResourceState};
pub use crate::models::InfraResource;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use tracing::info;
use uuid::Uuid;

/// Persistent state store — tracks desired vs actual infrastructure.
///
/// Conceptually compatible with `terraform.tfstate` but with richer semantics:
/// versioned snapshots, state locking, and drift detection via MCP.
#[derive(Default)]
pub struct InfraStateStore {
    pub state: InfraState,
    /// Versioned snapshots for rollback.
    pub history: Vec<StateSnapshot>,
}

impl InfraStateStore {
    /// Acquire a state lock before applying. Returns error if already locked.
    pub fn lock_state(&mut self, owner: &str) -> Result<(), StateError> {
        if let Some(ref locked_by) = self.state.locked_by {
            return Err(StateError::AlreadyLocked(locked_by.clone()));
        }
        self.state.locked_by = Some(owner.to_string());
        self.state.lock_acquired_at = Some(Utc::now());
        info!(owner = %owner, "State lock acquired");
        Ok(())
    }

    /// Release the state lock after apply completes (or fails).
    pub fn unlock_state(&mut self, owner: &str) -> Result<(), StateError> {
        match &self.state.locked_by {
            None => Err(StateError::NotLocked),
            Some(current) if current != owner => Err(StateError::LockOwnerMismatch {
                expected: owner.to_string(),
                actual: current.clone(),
            }),
            _ => {
                self.state.locked_by = None;
                self.state.lock_acquired_at = None;
                info!(owner = %owner, "State lock released");
                Ok(())
            }
        }
    }

    /// Detect drift between desired state and actual cloud resources.
    ///
    /// In production: calls MCP tools to query actual resource state and
    /// compares against `self.state.resources`. Here we report resources
    /// already marked `Drifted` in state.
    pub async fn detect_drift(&self) -> DriftReport {
        let drifted: Vec<ResourceDrift> = self
            .state
            .resources
            .values()
            .filter(|r| r.state == ResourceState::Drifted)
            .map(|r| ResourceDrift {
                resource_id: r.id,
                resource_name: r.name.clone(),
                field: "state".to_string(),
                desired: serde_json::Value::String("Active".to_string()),
                actual: serde_json::Value::String("Drifted".to_string()),
            })
            .collect();

        let total = drifted.len();
        DriftReport {
            id: Uuid::new_v4(),
            detected_at: Utc::now(),
            drifted_resources: drifted,
            total_drifted: total,
        }
    }

    /// Import an existing cloud resource into state without going through a plan.
    pub fn import_resource(
        &mut self,
        name: String,
        provider: String,
        resource_type: String,
        actual_id: String,
        config: HashMap<String, serde_json::Value>,
    ) -> InfraResource {
        let now = Utc::now();
        let resource = InfraResource {
            id: Uuid::new_v4(),
            name: name.clone(),
            provider,
            resource_type,
            config,
            state: ResourceState::Active,
            dependencies: vec![],
            actual_id: Some(actual_id),
            created_at: now,
            updated_at: now,
        };
        info!(resource = %name, "Resource imported into state");
        self.state.resources.insert(resource.id, resource.clone());
        resource
    }

    /// Return the full versioned state history.
    pub fn state_history(&self) -> &[StateSnapshot] {
        &self.history
    }

    /// Snapshot the current state before a mutation. Increments state version.
    pub fn snapshot(&mut self) {
        self.history.push(StateSnapshot {
            version: self.state.version,
            snapshotted_at: Utc::now(),
            resource_count: self.state.resources.len(),
        });
        self.state.version += 1;
    }
}

/// Lightweight state snapshot stored in history.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StateSnapshot {
    pub version: u64,
    pub snapshotted_at: DateTime<Utc>,
    pub resource_count: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("state already locked by: {0}")]
    AlreadyLocked(String),
    #[error("state is not locked")]
    NotLocked,
    #[error("lock owner mismatch: expected '{expected}', held by '{actual}'")]
    LockOwnerMismatch { expected: String, actual: String },
}
