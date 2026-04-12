<<<<<<< HEAD
<<<<<<< HEAD
=======
>>>>>>> claude/interesting-khorana
//! State management — persistent tracking, locking, drift detection, import, history.

use crate::models::{DriftReport, InfraResource, InfraState, ResourceDrift, ResourceState};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use tracing::info;
use uuid::Uuid;

/// Persistent state store — tracks desired vs actual infrastructure.
///
/// Conceptually replaces `terraform.tfstate` but with richer semantics:
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
<<<<<<< HEAD
=======
//! Infrastructure state management — tracks desired vs. actual state.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::providers::ResourceType;

// ── ResourceState ─────────────────────────────────────────────────────────────

/// Lifecycle state of a managed resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceState {
    Planned,
    Creating,
    Running,
    Updating,
    Destroying,
    Destroyed,
    Failed(String),
}

impl PartialEq for ResourceState {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Planned, Self::Planned) => true,
            (Self::Creating, Self::Creating) => true,
            (Self::Running, Self::Running) => true,
            (Self::Updating, Self::Updating) => true,
            (Self::Destroying, Self::Destroying) => true,
            (Self::Destroyed, Self::Destroyed) => true,
            (Self::Failed(a), Self::Failed(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for ResourceState {}

// ── InfraResource ─────────────────────────────────────────────────────────────

/// A single managed infrastructure resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfraResource {
    pub id: String,
    pub resource_type: ResourceType,
    pub provider: String,
    pub name: String,
    pub tenant_id: String,
    pub state: ResourceState,
    /// Desired spec (what we want).
    pub spec: serde_json::Value,
    /// Actual observed state (from provider).
    pub actual: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// External ID assigned by the cloud provider.
    pub provider_id: Option<String>,
}

impl InfraResource {
    pub fn new(
        id: &str,
        resource_type: ResourceType,
        provider: &str,
        name: &str,
        tenant_id: &str,
        spec: serde_json::Value,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: id.to_string(),
            resource_type,
            provider: provider.to_string(),
            name: name.to_string(),
            tenant_id: tenant_id.to_string(),
            state: ResourceState::Planned,
            spec,
            actual: None,
            created_at: now,
            updated_at: now,
            provider_id: None,
        }
    }

    /// Returns true when the actual observed state diverges from the desired spec.
    /// Simplified: drift exists when `actual` is `None` or differs from `spec`.
    pub fn has_drift(&self) -> bool {
        match &self.actual {
            None => true,
            Some(actual) => actual != &self.spec,
        }
    }
}

// ── InfraState ────────────────────────────────────────────────────────────────

/// Aggregate state for a single tenant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfraState {
    pub tenant_id: String,
    pub resources: HashMap<String, InfraResource>,
    pub last_applied: Option<DateTime<Utc>>,
    pub schema_version: u32,
}

impl InfraState {
    pub fn new(tenant_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            resources: HashMap::new(),
            last_applied: None,
            schema_version: 1,
        }
    }

    pub fn resource_count(&self) -> usize {
        self.resources.len()
    }

    /// Returns resources whose actual state differs from desired spec.
    pub fn drifted_resources(&self) -> Vec<&InfraResource> {
        self.resources.values().filter(|r| r.has_drift()).collect()
    }

    /// Returns resources currently in the given state.
    pub fn resources_in_state(&self, state: &ResourceState) -> Vec<&InfraResource> {
        self.resources
            .values()
            .filter(|r| &r.state == state)
            .collect()
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }

    pub fn from_json(v: &serde_json::Value) -> Option<Self> {
        serde_json::from_value(v.clone()).ok()
    }
}

// ── StateManager ──────────────────────────────────────────────────────────────

/// Thread-safe manager for tenant infrastructure states.
pub struct StateManager {
    states: Arc<RwLock<HashMap<String, InfraState>>>,
}

impl StateManager {
    pub fn new() -> Self {
        Self {
            states: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Return the tenant's state, creating an empty one if it doesn't exist yet.
    pub async fn get_or_create(&self, tenant_id: &str) -> InfraState {
        {
            let guard = self.states.read().await;
            if let Some(state) = guard.get(tenant_id) {
                return state.clone();
            }
        }
        let mut guard = self.states.write().await;
        guard
            .entry(tenant_id.to_string())
            .or_insert_with(|| InfraState::new(tenant_id))
            .clone()
    }

    /// Insert or replace a resource in the tenant's state.
    pub async fn upsert_resource(&self, tenant_id: &str, resource: InfraResource) {
        let mut guard = self.states.write().await;
        let state = guard
            .entry(tenant_id.to_string())
            .or_insert_with(|| InfraState::new(tenant_id));
        state.resources.insert(resource.id.clone(), resource);
    }

    pub async fn get_resource(
        &self,
        tenant_id: &str,
        resource_id: &str,
    ) -> Option<InfraResource> {
        let guard = self.states.read().await;
        guard
            .get(tenant_id)
            .and_then(|s| s.resources.get(resource_id))
            .cloned()
    }

    /// Transition a resource to a new state.
    pub async fn update_resource_state(
        &self,
        tenant_id: &str,
        resource_id: &str,
        state: ResourceState,
    ) -> Result<(), String> {
        let mut guard = self.states.write().await;
        let infra_state = guard
            .get_mut(tenant_id)
            .ok_or_else(|| format!("tenant {tenant_id} not found"))?;
        let resource = infra_state
            .resources
            .get_mut(resource_id)
            .ok_or_else(|| format!("resource {resource_id} not found"))?;
        resource.state = state;
        resource.updated_at = Utc::now();
        Ok(())
    }

    /// Record the provider-observed actual state for a resource.
    pub async fn update_actual(
        &self,
        tenant_id: &str,
        resource_id: &str,
        actual: serde_json::Value,
    ) -> Result<(), String> {
        let mut guard = self.states.write().await;
        let infra_state = guard
            .get_mut(tenant_id)
            .ok_or_else(|| format!("tenant {tenant_id} not found"))?;
        let resource = infra_state
            .resources
            .get_mut(resource_id)
            .ok_or_else(|| format!("resource {resource_id} not found"))?;
        resource.actual = Some(actual);
        resource.updated_at = Utc::now();
        Ok(())
    }

    /// Return all drifted resources for the tenant.
    pub async fn detect_drift(&self, tenant_id: &str) -> Vec<InfraResource> {
        let guard = self.states.read().await;
        guard
            .get(tenant_id)
            .map(|s| {
                s.resources
                    .values()
                    .filter(|r| r.has_drift())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Serialise the tenant's full state to JSON for persistence / inspection.
    pub async fn snapshot(&self, tenant_id: &str) -> Option<serde_json::Value> {
        let guard = self.states.read().await;
        guard.get(tenant_id).map(|s| s.to_json())
    }
}

impl Default for StateManager {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_resource(id: &str, tenant: &str) -> InfraResource {
        InfraResource::new(
            id,
            ResourceType::Vm,
            "mock",
            "my-vm",
            tenant,
            serde_json::json!({ "cpu_cores": 2 }),
        )
    }

    #[tokio::test]
    async fn test_upsert_and_get_resource() {
        let mgr = StateManager::new();
        let res = make_resource("r1", "tenant-x");
        mgr.upsert_resource("tenant-x", res.clone()).await;

        let fetched = mgr.get_resource("tenant-x", "r1").await.unwrap();
        assert_eq!(fetched.id, "r1");
        assert_eq!(fetched.tenant_id, "tenant-x");
        assert_eq!(fetched.name, "my-vm");

        let state = mgr.get_or_create("tenant-x").await;
        assert_eq!(state.resource_count(), 1);
    }

    #[tokio::test]
    async fn test_detect_drift() {
        let mgr = StateManager::new();

        // Resource with no actual state → drifted
        let r1 = make_resource("r1", "t1");
        mgr.upsert_resource("t1", r1).await;

        // Resource with matching actual state → not drifted
        let mut r2 = make_resource("r2", "t1");
        r2.actual = Some(r2.spec.clone());
        mgr.upsert_resource("t1", r2).await;

        // Resource with different actual state → drifted
        let mut r3 = make_resource("r3", "t1");
        r3.actual = Some(serde_json::json!({ "cpu_cores": 99 }));
        mgr.upsert_resource("t1", r3).await;

        let drifted = mgr.detect_drift("t1").await;
        assert_eq!(drifted.len(), 2);
        let ids: Vec<&str> = drifted.iter().map(|r| r.id.as_str()).collect();
        assert!(ids.contains(&"r1"));
        assert!(ids.contains(&"r3"));
    }

    #[tokio::test]
    async fn test_snapshot_serialize_deserialize() {
        let mgr = StateManager::new();
        let res = make_resource("snap-r1", "tenant-snap");
        mgr.upsert_resource("tenant-snap", res).await;

        let snap = mgr.snapshot("tenant-snap").await.expect("snapshot present");
        let restored = InfraState::from_json(&snap).expect("deserializes");
        assert_eq!(restored.tenant_id, "tenant-snap");
        assert_eq!(restored.resource_count(), 1);
        assert!(restored.resources.contains_key("snap-r1"));
    }
>>>>>>> claude/great-sanderson
=======
>>>>>>> claude/interesting-khorana
}
