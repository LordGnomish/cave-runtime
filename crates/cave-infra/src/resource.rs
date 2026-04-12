//! Resource model — desired state, actual state, resource kinds.

use crate::error::{InfraError, InfraResult};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ── Resource kinds ───────────────────────────────────────────────��────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ResourceKind {
    Server,
    Network,
    Subnet,
    LoadBalancer,
    BlockStorage,
    ObjectStorage,
    Database,
    Cache,
    Dns,
    Firewall,
    IpAddress,
    SshKey,
    SecretRef,
    KubernetesCluster,
    Namespace,
    Custom(String),
}

impl ResourceKind {
    pub fn from_str(s: &str) -> Self {
        match s {
            "Server" => Self::Server,
            "Network" => Self::Network,
            "Subnet" => Self::Subnet,
            "LoadBalancer" => Self::LoadBalancer,
            "BlockStorage" => Self::BlockStorage,
            "ObjectStorage" => Self::ObjectStorage,
            "Database" => Self::Database,
            "Cache" => Self::Cache,
            "Dns" => Self::Dns,
            "Firewall" => Self::Firewall,
            "IpAddress" => Self::IpAddress,
            "SshKey" => Self::SshKey,
            "SecretRef" => Self::SecretRef,
            "KubernetesCluster" => Self::KubernetesCluster,
            "Namespace" => Self::Namespace,
            other => Self::Custom(other.to_string()),
        }
    }

    pub fn as_str(&self) -> String {
        match self {
            Self::Custom(s) => s.clone(),
            other => format!("{other:?}"),
        }
    }
}

// ── Resource status ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ResourceStatus {
    Pending,
    Creating,
    Running,
    Updating,
    Deleting,
    Deleted,
    Failed,
    Drifted,
}

// ── Resource ──────────────────────────────────────────────────────────────────

/// Desired state of a resource (what the user declared).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceSpec {
    pub kind: ResourceKind,
    pub name: String,
    pub provider: String,
    pub properties: HashMap<String, serde_json::Value>,
    pub depends_on: Vec<String>, // resource names
    pub tags: HashMap<String, String>,
}

/// Actual state of a resource (observed from the provider).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceState {
    pub id: Uuid,
    pub spec: ResourceSpec,
    pub status: ResourceStatus,
    pub provider_id: Option<String>,
    pub actual_properties: HashMap<String, serde_json::Value>,
    pub outputs: HashMap<String, serde_json::Value>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub version: u64,
}

impl ResourceState {
    pub fn new(spec: ResourceSpec) -> Self {
        Self {
            id: Uuid::new_v4(),
            spec,
            status: ResourceStatus::Pending,
            provider_id: None,
            actual_properties: HashMap::new(),
            outputs: HashMap::new(),
            error: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            version: 0,
        }
    }

    pub fn key(&self) -> String {
        format!("{}/{}", self.spec.kind.as_str(), self.spec.name)
    }

    pub fn transition(&mut self, status: ResourceStatus) {
        tracing::debug!(
            resource = %self.key(),
            from = ?self.status,
            to = ?status,
            "resource state transition"
        );
        self.status = status;
        self.version += 1;
        self.updated_at = Utc::now();
    }

    pub fn apply_actual(&mut self, actual: HashMap<String, serde_json::Value>, provider_id: Option<String>) {
        self.actual_properties = actual;
        self.provider_id = provider_id;
        self.transition(ResourceStatus::Running);
    }

    pub fn fail(&mut self, error: String) {
        self.error = Some(error);
        self.status = ResourceStatus::Failed;
        self.updated_at = Utc::now();
    }

    /// Check whether actual state matches desired state.
    pub fn has_drift(&self) -> Vec<DriftItem> {
        let mut drifts = Vec::new();
        for (key, desired) in &self.spec.properties {
            match self.actual_properties.get(key) {
                None => drifts.push(DriftItem {
                    field: key.clone(),
                    desired: desired.clone(),
                    actual: serde_json::Value::Null,
                }),
                Some(actual) if actual != desired => drifts.push(DriftItem {
                    field: key.clone(),
                    desired: desired.clone(),
                    actual: actual.clone(),
                }),
                _ => {}
            }
        }
        drifts
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftItem {
    pub field: String,
    pub desired: serde_json::Value,
    pub actual: serde_json::Value,
}

// ── Resource store ────────────────────────────────────────────────────────────

pub struct ResourceStore {
    resources: DashMap<String, ResourceState>,
    /// Snapshot history for rollback: resource_key → ordered snapshots
    history: DashMap<String, Vec<ResourceState>>,
}

impl ResourceStore {
    pub fn new() -> Self {
        Self {
            resources: DashMap::new(),
            history: DashMap::new(),
        }
    }

    pub fn upsert(&self, state: ResourceState) -> String {
        let key = state.key();
        // Save to history before overwriting
        if let Some(existing) = self.resources.get(&key) {
            self.history.entry(key.clone()).or_default().push(existing.clone());
        }
        self.resources.insert(key.clone(), state);
        key
    }

    pub fn get(&self, key: &str) -> InfraResult<ResourceState> {
        self.resources
            .get(key)
            .map(|r| r.clone())
            .ok_or_else(|| {
                let parts: Vec<&str> = key.splitn(2, '/').collect();
                InfraError::NotFound {
                    kind: parts.first().copied().unwrap_or("Unknown").to_string(),
                    name: parts.get(1).copied().unwrap_or(key).to_string(),
                }
            })
    }

    pub fn get_by_name(&self, kind: &ResourceKind, name: &str) -> InfraResult<ResourceState> {
        let key = format!("{}/{name}", kind.as_str());
        self.get(&key)
    }

    pub fn list(&self) -> Vec<ResourceState> {
        self.resources.iter().map(|e| e.value().clone()).collect()
    }

    pub fn list_by_kind(&self, kind: &ResourceKind) -> Vec<ResourceState> {
        let prefix = format!("{}/", kind.as_str());
        self.resources
            .iter()
            .filter(|e| e.key().starts_with(&prefix))
            .map(|e| e.value().clone())
            .collect()
    }

    pub fn delete(&self, key: &str) -> InfraResult<()> {
        self.resources.remove(key).ok_or_else(|| {
            let parts: Vec<&str> = key.splitn(2, '/').collect();
            InfraError::NotFound {
                kind: parts.first().copied().unwrap_or("").to_string(),
                name: parts.get(1).copied().unwrap_or(key).to_string(),
            }
        })?;
        Ok(())
    }

    pub fn history(&self, key: &str) -> Vec<ResourceState> {
        self.history.get(key).map(|h| h.clone()).unwrap_or_default()
    }

    pub fn restore_previous(&self, key: &str) -> InfraResult<ResourceState> {
        let mut history = self
            .history
            .get_mut(key)
            .ok_or_else(|| InfraError::RollbackFailed(format!("no history for {key}")))?;
        let prev = history.pop().ok_or_else(|| {
            InfraError::RollbackFailed(format!("empty history for {key}"))
        })?;
        self.resources.insert(key.to_string(), prev.clone());
        Ok(prev)
    }

    pub fn drifted_resources(&self) -> Vec<(String, Vec<DriftItem>)> {
        self.resources
            .iter()
            .filter(|e| e.status == ResourceStatus::Running)
            .filter_map(|e| {
                let drifts = e.has_drift();
                if drifts.is_empty() {
                    None
                } else {
                    Some((e.key().clone(), drifts))
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server_spec(name: &str) -> ResourceSpec {
        let mut props = HashMap::new();
        props.insert("cpu".into(), serde_json::json!(4));
        props.insert("memory_gb".into(), serde_json::json!(16));
        props.insert("os".into(), serde_json::json!("ubuntu-22.04"));
        ResourceSpec {
            kind: ResourceKind::Server,
            name: name.to_string(),
            provider: "bare-metal".into(),
            properties: props,
            depends_on: vec![],
            tags: HashMap::new(),
        }
    }

    #[test]
    fn create_and_retrieve_resource() {
        let store = ResourceStore::new();
        let state = ResourceState::new(server_spec("web-01"));
        let key = store.upsert(state);
        let got = store.get(&key).unwrap();
        assert_eq!(got.spec.name, "web-01");
        assert_eq!(got.status, ResourceStatus::Pending);
    }

    #[test]
    fn apply_actual_transitions_to_running() {
        let store = ResourceStore::new();
        let mut state = ResourceState::new(server_spec("db-01"));
        let mut actual = HashMap::new();
        actual.insert("cpu".into(), serde_json::json!(4));
        actual.insert("memory_gb".into(), serde_json::json!(16));
        actual.insert("os".into(), serde_json::json!("ubuntu-22.04"));
        state.apply_actual(actual, Some("prov-id-123".into()));
        let key = store.upsert(state);
        let got = store.get(&key).unwrap();
        assert_eq!(got.status, ResourceStatus::Running);
        assert!(got.has_drift().is_empty());
    }

    #[test]
    fn drift_detected_when_actual_differs() {
        let store = ResourceStore::new();
        let mut state = ResourceState::new(server_spec("api-01"));
        let mut actual = HashMap::new();
        actual.insert("cpu".into(), serde_json::json!(2)); // desired: 4
        actual.insert("memory_gb".into(), serde_json::json!(16));
        actual.insert("os".into(), serde_json::json!("ubuntu-22.04"));
        state.apply_actual(actual, None);
        let key = store.upsert(state);
        let got = store.get(&key).unwrap();
        let drifts = got.has_drift();
        assert_eq!(drifts.len(), 1);
        assert_eq!(drifts[0].field, "cpu");
    }

    #[test]
    fn history_and_rollback() {
        let store = ResourceStore::new();
        let mut state = ResourceState::new(server_spec("lb-01"));
        state.transition(ResourceStatus::Running);
        let key = store.upsert(state.clone());

        // Update
        state.spec.properties.insert("cpu".into(), serde_json::json!(8));
        state.transition(ResourceStatus::Updating);
        store.upsert(state);

        // Rollback
        let prev = store.restore_previous(&key).unwrap();
        assert_eq!(prev.spec.properties["cpu"], serde_json::json!(4));
    }
}
