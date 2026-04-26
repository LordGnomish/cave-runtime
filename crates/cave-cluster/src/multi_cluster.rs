use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

// ── Registration ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegistrationStatus {
    Pending,
    Connected,
    Disconnected,
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterRegistration {
    pub id: Uuid,
    pub cluster_id: Uuid,
    pub cluster_name: String,
    pub api_endpoint: String,
    pub status: RegistrationStatus,
    pub registered_at: chrono::DateTime<chrono::Utc>,
    pub last_seen: Option<chrono::DateTime<chrono::Utc>>,
    pub labels: HashMap<String, String>,
}

impl ClusterRegistration {
    pub fn new(cluster_id: Uuid, cluster_name: &str, api_endpoint: &str) -> Self {
        Self {
            id: Uuid::new_v4(),
            cluster_id,
            cluster_name: cluster_name.to_string(),
            api_endpoint: api_endpoint.to_string(),
            status: RegistrationStatus::Pending,
            registered_at: Utc::now(),
            last_seen: None,
            labels: HashMap::new(),
        }
    }
}

// ── Federated operation ───────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FederatedOpStatus {
    Pending,
    Running,
    Completed,
    PartialFailure,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederatedOperation {
    pub id: Uuid,
    pub operation_type: String,
    pub target_clusters: Vec<Uuid>,
    pub payload: serde_json::Value,
    pub results: HashMap<Uuid, serde_json::Value>,
    pub status: FederatedOpStatus,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl FederatedOperation {
    pub fn new(
        operation_type: &str,
        target_clusters: Vec<Uuid>,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            operation_type: operation_type.to_string(),
            target_clusters,
            payload,
            results: HashMap::new(),
            status: FederatedOpStatus::Pending,
            created_at: Utc::now(),
        }
    }
}

// ── Manager ───────────────────────────────────────────────────────────────────

pub struct MultiClusterManager {
    /// Keyed by `cluster_id` (not registration id).
    registrations: Arc<RwLock<HashMap<Uuid, ClusterRegistration>>>,
    operations: Arc<RwLock<Vec<FederatedOperation>>>,
}

impl MultiClusterManager {
    pub fn new() -> Self {
        Self {
            registrations: Arc::new(RwLock::new(HashMap::new())),
            operations: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Register a cluster. Returns the registration's own id.
    pub async fn register(&self, reg: ClusterRegistration) -> Uuid {
        let reg_id = reg.id;
        let cluster_id = reg.cluster_id;
        let mut guard = self.registrations.write().await;
        guard.insert(cluster_id, reg);
        tracing::info!(cluster_id = %cluster_id, "cluster registered in multi-cluster manager");
        reg_id
    }

    pub async fn deregister(&self, cluster_id: Uuid) -> Result<(), String> {
        let mut guard = self.registrations.write().await;
        guard
            .remove(&cluster_id)
            .ok_or_else(|| format!("cluster {cluster_id} not registered"))?;
        Ok(())
    }

    pub async fn get_registration(&self, cluster_id: Uuid) -> Option<ClusterRegistration> {
        let guard = self.registrations.read().await;
        guard.get(&cluster_id).cloned()
    }

    pub async fn list_connected(&self) -> Vec<ClusterRegistration> {
        let guard = self.registrations.read().await;
        guard
            .values()
            .filter(|r| r.status == RegistrationStatus::Connected)
            .cloned()
            .collect()
    }

    pub async fn update_status(&self, cluster_id: Uuid, status: RegistrationStatus) {
        let mut guard = self.registrations.write().await;
        if let Some(reg) = guard.get_mut(&cluster_id) {
            reg.status = status;
            reg.last_seen = Some(Utc::now());
        }
    }

    /// Submit a federated operation. Returns the operation id.
    pub async fn submit_federated_op(&self, op: FederatedOperation) -> Uuid {
        let id = op.id;
        let mut guard = self.operations.write().await;
        guard.push(op);
        id
    }

    /// Record the result of one cluster completing the federated op.
    pub async fn complete_federated_op(
        &self,
        op_id: Uuid,
        cluster_id: Uuid,
        result: serde_json::Value,
    ) -> Result<(), String> {
        let mut guard = self.operations.write().await;
        let op = guard
            .iter_mut()
            .find(|o| o.id == op_id)
            .ok_or_else(|| format!("operation {op_id} not found"))?;

        op.results.insert(cluster_id, result);

        // Update overall status.
        let expected = op.target_clusters.len();
        let received = op.results.len();
        if received >= expected {
            op.status = FederatedOpStatus::Completed;
        } else {
            op.status = FederatedOpStatus::Running;
        }
        Ok(())
    }

    pub async fn get_federated_op(&self, op_id: Uuid) -> Option<FederatedOperation> {
        let guard = self.operations.read().await;
        guard.iter().find(|o| o.id == op_id).cloned()
    }
}

impl Default for MultiClusterManager {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(cluster_id: Uuid) -> ClusterRegistration {
        let mut r =
            ClusterRegistration::new(cluster_id, "prod-cluster", "https://10.0.0.1:6443");
        r.status = RegistrationStatus::Connected;
        r
    }

    #[tokio::test]
    async fn test_register_cluster() {
        let mgr = MultiClusterManager::new();
        let cluster_id = Uuid::new_v4();
        let reg_id = mgr.register(reg(cluster_id)).await;
        assert_ne!(reg_id, Uuid::nil());
        let stored = mgr.get_registration(cluster_id).await;
        assert!(stored.is_some());
    }

    #[tokio::test]
    async fn test_list_connected_only_connected() {
        let mgr = MultiClusterManager::new();

        let c1 = Uuid::new_v4();
        let c2 = Uuid::new_v4();

        mgr.register(reg(c1)).await; // Connected
        let mut disconnected = reg(c2);
        disconnected.status = RegistrationStatus::Disconnected;
        mgr.register(disconnected).await;

        let connected = mgr.list_connected().await;
        assert_eq!(connected.len(), 1);
        assert_eq!(connected[0].cluster_id, c1);
    }

    #[tokio::test]
    async fn test_deregister() {
        let mgr = MultiClusterManager::new();
        let cluster_id = Uuid::new_v4();
        mgr.register(reg(cluster_id)).await;
        mgr.deregister(cluster_id).await.unwrap();
        assert!(mgr.get_registration(cluster_id).await.is_none());
    }

    #[tokio::test]
    async fn test_federated_op_submit_and_complete() {
        let mgr = MultiClusterManager::new();
        let c1 = Uuid::new_v4();
        let c2 = Uuid::new_v4();

        let op = FederatedOperation::new(
            "deploy",
            vec![c1, c2],
            serde_json::json!({"image": "nginx:latest"}),
        );
        let op_id = mgr.submit_federated_op(op).await;

        mgr.complete_federated_op(op_id, c1, serde_json::json!({"ok": true}))
            .await
            .unwrap();
        mgr.complete_federated_op(op_id, c2, serde_json::json!({"ok": true}))
            .await
            .unwrap();

        let stored = mgr.get_federated_op(op_id).await.unwrap();
        assert_eq!(stored.status, FederatedOpStatus::Completed);
        assert_eq!(stored.results.len(), 2);
    }

    #[tokio::test]
    async fn test_update_status() {
        let mgr = MultiClusterManager::new();
        let cluster_id = Uuid::new_v4();
        mgr.register(reg(cluster_id)).await;
        mgr.update_status(cluster_id, RegistrationStatus::Disconnected).await;
        let stored = mgr.get_registration(cluster_id).await.unwrap();
        assert_eq!(stored.status, RegistrationStatus::Disconnected);
        assert!(stored.last_seen.is_some());
    }
}
