use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::node::ClusterNode;

// ── Provider / Distro / State ────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClusterProvider {
    BareMetal,
    HetznerCloud,
    AzureVM,
    Aws,
    Gcp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClusterState {
    Provisioning,
    Running,
    Degraded,
    Upgrading,
    Destroying,
    Destroyed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KubernetesDistro {
    K3s,
    Rke2,
    Kubeadm,
}

// ── Spec & Cluster ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterSpec {
    pub name: String,
    pub provider: ClusterProvider,
    pub distro: KubernetesDistro,
    pub kubernetes_version: String,
    pub control_plane_count: usize,
    pub worker_count: usize,
    pub region: String,
    pub tenant_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cluster {
    pub id: Uuid,
    pub spec: ClusterSpec,
    pub state: ClusterState,
    /// Node IDs belonging to this cluster.
    pub nodes: Vec<Uuid>,
    pub api_endpoint: Option<String>,
    /// Base64-encoded kubeconfig.
    pub kubeconfig: Option<String>,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
    pub created_by: Uuid,
    pub labels: HashMap<String, String>,
    pub annotations: HashMap<String, String>,
}

impl Cluster {
    pub fn new(spec: ClusterSpec, created_by: Uuid) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            spec,
            state: ClusterState::Provisioning,
            nodes: Vec::new(),
            api_endpoint: None,
            kubeconfig: None,
            created_at: now,
            updated_at: now,
            created_by,
            labels: HashMap::new(),
            annotations: HashMap::new(),
        }
    }

    pub fn is_ready(&self) -> bool {
        self.state == ClusterState::Running
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
}

// ── Error ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, thiserror::Error)]
pub enum ClusterError {
    #[error("Cluster not found: {0}")]
    NotFound(Uuid),
    #[error("Cluster already exists: {0}")]
    AlreadyExists(String),
    #[error("Invalid state transition from {from:?} to {to:?}")]
    InvalidStateTransition { from: ClusterState, to: ClusterState },
    #[error("Cluster is not ready")]
    NotReady,
    #[error("{0}")]
    Internal(String),
}

// ── Manager ──────────────────────────────────────────────────────────────────

pub struct ClusterManager {
    clusters: Arc<RwLock<HashMap<Uuid, Cluster>>>,
    nodes: Arc<RwLock<HashMap<Uuid, ClusterNode>>>,
}

impl ClusterManager {
    pub fn new() -> Self {
        Self {
            clusters: Arc::new(RwLock::new(HashMap::new())),
            nodes: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Provision a new cluster.
    ///
    /// Simulates async provisioning: the cluster starts in `Provisioning` state and
    /// is immediately transitioned to `Running` once written (no actual I/O).
    pub async fn provision(
        &self,
        spec: ClusterSpec,
        created_by: Uuid,
    ) -> Result<Cluster, ClusterError> {
        // Check for duplicate name within the tenant.
        {
            let guard = self.clusters.read().await;
            let exists = guard
                .values()
                .any(|c| c.spec.name == spec.name && c.spec.tenant_id == spec.tenant_id);
            if exists {
                return Err(ClusterError::AlreadyExists(spec.name.clone()));
            }
        }

        let mut cluster = Cluster::new(spec, created_by);
        tracing::info!(cluster_id = %cluster.id, name = %cluster.spec.name, "provisioning cluster");

        // Simulate provisioning completion.
        cluster.state = ClusterState::Running;
        cluster.updated_at = Utc::now();

        let mut guard = self.clusters.write().await;
        guard.insert(cluster.id, cluster.clone());
        Ok(cluster)
    }

    pub async fn get(&self, id: Uuid) -> Result<Cluster, ClusterError> {
        let guard = self.clusters.read().await;
        guard.get(&id).cloned().ok_or(ClusterError::NotFound(id))
    }

    pub async fn get_by_name(&self, name: &str) -> Option<Cluster> {
        let guard = self.clusters.read().await;
        guard.values().find(|c| c.spec.name == name).cloned()
    }

    pub async fn list(&self, tenant_id: &str) -> Vec<Cluster> {
        let guard = self.clusters.read().await;
        guard
            .values()
            .filter(|c| c.spec.tenant_id == tenant_id)
            .cloned()
            .collect()
    }

    pub async fn join_node(
        &self,
        cluster_id: Uuid,
        node: ClusterNode,
    ) -> Result<(), ClusterError> {
        let mut clusters = self.clusters.write().await;
        let cluster = clusters
            .get_mut(&cluster_id)
            .ok_or(ClusterError::NotFound(cluster_id))?;

        if cluster.state == ClusterState::Destroyed
            || cluster.state == ClusterState::Destroying
        {
            return Err(ClusterError::InvalidStateTransition {
                from: cluster.state.clone(),
                to: ClusterState::Running,
            });
        }

        let node_id = node.id;
        cluster.nodes.push(node_id);
        cluster.updated_at = Utc::now();

        let mut nodes = self.nodes.write().await;
        nodes.insert(node_id, node);

        tracing::info!(cluster_id = %cluster_id, node_id = %node_id, "node joined cluster");
        Ok(())
    }

    pub async fn remove_node(
        &self,
        cluster_id: Uuid,
        node_id: Uuid,
    ) -> Result<(), ClusterError> {
        let mut clusters = self.clusters.write().await;
        let cluster = clusters
            .get_mut(&cluster_id)
            .ok_or(ClusterError::NotFound(cluster_id))?;

        cluster.nodes.retain(|id| *id != node_id);
        cluster.updated_at = Utc::now();

        let mut nodes = self.nodes.write().await;
        nodes.remove(&node_id);

        tracing::info!(cluster_id = %cluster_id, node_id = %node_id, "node removed from cluster");
        Ok(())
    }

    pub async fn destroy(&self, cluster_id: Uuid) -> Result<(), ClusterError> {
        let mut guard = self.clusters.write().await;
        let cluster = guard
            .get_mut(&cluster_id)
            .ok_or(ClusterError::NotFound(cluster_id))?;

        if cluster.state == ClusterState::Destroyed {
            return Err(ClusterError::InvalidStateTransition {
                from: ClusterState::Destroyed,
                to: ClusterState::Destroying,
            });
        }

        cluster.state = ClusterState::Destroyed;
        cluster.updated_at = Utc::now();
        tracing::info!(cluster_id = %cluster_id, "cluster destroyed");
        Ok(())
    }

    pub async fn update_state(
        &self,
        cluster_id: Uuid,
        state: ClusterState,
    ) -> Result<(), ClusterError> {
        let mut guard = self.clusters.write().await;
        let cluster = guard
            .get_mut(&cluster_id)
            .ok_or(ClusterError::NotFound(cluster_id))?;
        cluster.state = state;
        cluster.updated_at = Utc::now();
        Ok(())
    }

    pub async fn set_api_endpoint(
        &self,
        cluster_id: Uuid,
        endpoint: String,
    ) -> Result<(), ClusterError> {
        let mut guard = self.clusters.write().await;
        let cluster = guard
            .get_mut(&cluster_id)
            .ok_or(ClusterError::NotFound(cluster_id))?;
        cluster.api_endpoint = Some(endpoint);
        cluster.updated_at = Utc::now();
        Ok(())
    }
}

impl Default for ClusterManager {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{NodeResources, NodeRole};

    fn sample_spec(name: &str) -> ClusterSpec {
        ClusterSpec {
            name: name.to_string(),
            provider: ClusterProvider::BareMetal,
            distro: KubernetesDistro::K3s,
            kubernetes_version: "v1.29.0".to_string(),
            control_plane_count: 1,
            worker_count: 2,
            region: "eu-west-1".to_string(),
            tenant_id: "tenant-abc".to_string(),
        }
    }

    fn sample_resources() -> NodeResources {
        NodeResources { cpu_cores: 4, memory_gb: 8, disk_gb: 100, gpu_count: 0 }
    }

    #[tokio::test]
    async fn test_provision_creates_running_cluster() {
        let mgr = ClusterManager::new();
        let owner = Uuid::new_v4();
        let cluster = mgr.provision(sample_spec("prod"), owner).await.unwrap();
        assert_eq!(cluster.state, ClusterState::Running);
        assert!(cluster.is_ready());
    }

    #[tokio::test]
    async fn test_provision_duplicate_name_error() {
        let mgr = ClusterManager::new();
        let owner = Uuid::new_v4();
        mgr.provision(sample_spec("prod"), owner).await.unwrap();
        let err = mgr.provision(sample_spec("prod"), owner).await.unwrap_err();
        assert!(matches!(err, ClusterError::AlreadyExists(_)));
    }

    #[tokio::test]
    async fn test_join_node_adds_node() {
        let mgr = ClusterManager::new();
        let owner = Uuid::new_v4();
        let cluster = mgr.provision(sample_spec("prod"), owner).await.unwrap();

        let node = ClusterNode::new(
            cluster.id,
            "worker-1",
            "10.0.0.2",
            NodeRole::Worker,
            sample_resources(),
        );
        let node_id = node.id;
        mgr.join_node(cluster.id, node).await.unwrap();

        let updated = mgr.get(cluster.id).await.unwrap();
        assert_eq!(updated.node_count(), 1);
        assert!(updated.nodes.contains(&node_id));
    }

    #[tokio::test]
    async fn test_remove_node_removes_it() {
        let mgr = ClusterManager::new();
        let owner = Uuid::new_v4();
        let cluster = mgr.provision(sample_spec("prod"), owner).await.unwrap();

        let node = ClusterNode::new(
            cluster.id,
            "worker-1",
            "10.0.0.2",
            NodeRole::Worker,
            sample_resources(),
        );
        let node_id = node.id;
        mgr.join_node(cluster.id, node).await.unwrap();
        mgr.remove_node(cluster.id, node_id).await.unwrap();

        let updated = mgr.get(cluster.id).await.unwrap();
        assert_eq!(updated.node_count(), 0);
    }

    #[tokio::test]
    async fn test_destroy_sets_destroyed_state() {
        let mgr = ClusterManager::new();
        let owner = Uuid::new_v4();
        let cluster = mgr.provision(sample_spec("prod"), owner).await.unwrap();
        mgr.destroy(cluster.id).await.unwrap();

        let updated = mgr.get(cluster.id).await.unwrap();
        assert_eq!(updated.state, ClusterState::Destroyed);
    }

    #[tokio::test]
    async fn test_list_filters_by_tenant() {
        let mgr = ClusterManager::new();
        let owner = Uuid::new_v4();

        let mut spec_a = sample_spec("a");
        spec_a.tenant_id = "tenant-A".to_string();
        let mut spec_b = sample_spec("b");
        spec_b.tenant_id = "tenant-B".to_string();

        mgr.provision(spec_a, owner).await.unwrap();
        mgr.provision(spec_b, owner).await.unwrap();

        let tenant_a_clusters = mgr.list("tenant-A").await;
        assert_eq!(tenant_a_clusters.len(), 1);
        assert_eq!(tenant_a_clusters[0].spec.tenant_id, "tenant-A");
    }

    #[tokio::test]
    async fn test_set_api_endpoint() {
        let mgr = ClusterManager::new();
        let owner = Uuid::new_v4();
        let cluster = mgr.provision(sample_spec("prod"), owner).await.unwrap();
        mgr.set_api_endpoint(cluster.id, "https://10.0.0.1:6443".to_string())
            .await
            .unwrap();
        let updated = mgr.get(cluster.id).await.unwrap();
        assert_eq!(updated.api_endpoint.as_deref(), Some("https://10.0.0.1:6443"));
    }
}
