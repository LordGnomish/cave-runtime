//! CAVE Cluster — Kubernetes cluster lifecycle management.
//!
//! Replaces Rancher, Gardener, and Cluster API with a Rust-native control
//! plane. The CAVE runtime runs on bare metal; cave-cluster provisions and
//! manages Kubernetes clusters for tenants on top of it.
//!
//! ## Architecture
//! - Multi-tenant cluster isolation at the cluster level
//! - Provider support: Hetzner, Azure, AWS, bare metal (kubeadm)
//! - Cloud API calls delegated to cave-infra MCP bridge
//! - Encrypted kubeconfigs stored in cave-vault
//! - Tenant network isolation via Cilium NetworkPolicies
//!
//! ## Upstream Tracking
//! - Rancher:      <https://github.com/rancher/rancher>
//! - Gardener:     <https://github.com/gardener/gardener>
//! - Cluster API:  <https://github.com/kubernetes-sigs/cluster-api>

pub mod health;
pub mod models;
pub mod provisioner;
pub mod routes;
pub mod tenant;

use axum::Router;
use models::{Cluster, ClusterAddon, ClusterEvent, NodePool, Tenant};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// In-memory store for all cluster state.
///
/// In a production deployment this is backed by PostgreSQL via cave-db.
/// The `Arc<Mutex<ClusterStore>>` is cloned into every request handler.
pub struct ClusterStore {
    pub clusters: HashMap<Uuid, Cluster>,
    pub tenants: HashMap<Uuid, Tenant>,
    pub node_pools: HashMap<Uuid, NodePool>,
    pub events: Vec<ClusterEvent>,
    /// cluster_id → installed add-ons
    pub addons: HashMap<Uuid, Vec<ClusterAddon>>,
}

/// Module state shared across request handlers via `Arc<ClusterState>`.
pub struct ClusterState {
    pub store: Arc<Mutex<ClusterStore>>,
}

impl Default for ClusterState {
    fn default() -> Self {
        Self {
            store: Arc::new(Mutex::new(ClusterStore {
                clusters: HashMap::new(),
                tenants: HashMap::new(),
                node_pools: HashMap::new(),
                events: Vec::new(),
                addons: HashMap::new(),
            })),
        }
    }
}

/// Create the axum router for the cluster module.
pub fn router(state: Arc<ClusterState>) -> Router {
    routes::create_router(state)
}

/// Module name constant (used for logging and DB schema namespacing).
pub const MODULE_NAME: &str = "cluster";
pub mod cluster;
pub mod k8s_distro;
pub mod multi_cluster;
pub mod node;
pub mod tenant_ns;
pub mod upgrade;
pub use cluster::{
    Cluster, ClusterError, ClusterManager, ClusterProvider, ClusterSpec, ClusterState,
    KubernetesDistro,
};
pub use health::{
    ClusterHealthChecker, ClusterHealthReport, ComponentHealth, ComponentStatus,
    NodeResourceUsage,
};
pub use k8s_distro::{InstallConfig, InstallJob, InstallManager, InstallStatus};
pub use multi_cluster::{
    ClusterRegistration, FederatedOpStatus, FederatedOperation, MultiClusterManager,
    RegistrationStatus,
};
pub use node::{ClusterNode, NodeResources, NodeRole, NodeStatus};
pub use tenant_ns::{
    LimitRange, NamespaceProvisioner, NamespaceStatus, ResourceQuota, TenantNamespace,
};
pub use upgrade::{UpgradeManager, UpgradePlan, UpgradeStatus, UpgradeStrategy};
