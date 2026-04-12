<<<<<<< HEAD
//! Cluster health checks and diagnostics.
//!
//! Probes the API server, etcd, and individual nodes. Results feed into
//! the health dashboard and drive automated remediation (cordon/drain).

use crate::models::*;
use chrono::Utc;
use tracing::{info, warn};
use uuid::Uuid;

/// Perform a full health check for a cluster.
///
/// Probes:
/// - API server `/healthz` endpoint
/// - etcd cluster health via metrics
/// - Node readiness via the Nodes API
/// - kube-system component pods
pub fn check_cluster_health(cluster: &Cluster) -> ClusterHealth {
    info!(cluster_id = %cluster.id, "Checking cluster health");

    // TODO: connect to cluster API server using kubeconfig from cave-vault
    // TODO: GET <endpoint>/healthz
    // TODO: GET <endpoint>/api/v1/nodes for readiness counts
    // TODO: check etcd via <endpoint>/metrics or etcd client

    let api_server_status = match &cluster.status {
        ClusterStatus::Running => HealthStatus::Healthy,
        ClusterStatus::Provisioning | ClusterStatus::Upgrading => HealthStatus::Degraded {
            reason: format!("Cluster is in {:?} state", cluster.status),
        },
        ClusterStatus::Deleting | ClusterStatus::Error { .. } => HealthStatus::Unreachable,
    };

    ClusterHealth {
        cluster_id: cluster.id,
        api_server_status,
        etcd_health: HealthStatus::Healthy, // TODO: real etcd probe
        node_readiness: NodeReadiness {
            total: 0,
            ready: 0,
            not_ready: 0,
            cordoned: 0,
        },
        component_statuses: component_status(cluster),
        checked_at: Utc::now(),
    }
}

/// Identify NotReady nodes and return their names for remediation.
///
/// Side-effects (TODO):
/// - Cordon nodes that have been NotReady > 5 minutes
/// - Drain cordoned nodes that have been NotReady > 15 minutes
pub fn detect_unhealthy_nodes(health: &ClusterHealth) -> Vec<String> {
    if health.node_readiness.not_ready == 0 {
        return vec![];
    }

    warn!(
        cluster_id = %health.cluster_id,
        not_ready = health.node_readiness.not_ready,
        "Unhealthy nodes detected"
    );

    // TODO: list NotReady nodes from cluster API
    // TODO: cordon nodes > 5 min NotReady
    // TODO: drain nodes > 15 min NotReady
    vec![]
}

/// Return the health status of kube-system components.
///
/// Checks: kube-controller-manager, kube-scheduler, kube-proxy,
/// coredns, and any installed CAVE add-ons.
pub fn component_status(cluster: &Cluster) -> Vec<ComponentStatus> {
    info!(cluster_id = %cluster.id, "Checking component status");

    // TODO: GET <endpoint>/api/v1/namespaces/kube-system/pods
    // TODO: check each pod's Ready condition

    vec![
        ComponentStatus {
            name: "kube-controller-manager".into(),
            healthy: true,
            message: None,
        },
        ComponentStatus {
            name: "kube-scheduler".into(),
            healthy: true,
            message: None,
        },
        ComponentStatus {
            name: "coredns".into(),
            healthy: true,
            message: None,
        },
    ]
}

/// Check how many days remain before cluster TLS certificates expire.
///
/// Returns `None` if no kubeconfig ref is available.
/// Returns `Some(days)` — negative means already expired.
///
/// A warning is emitted at ≤ 30 days; an error at ≤ 7 days.
pub fn certificate_expiry_check(kubeconfig_ref: &KubeconfigRef) -> Option<i64> {
    // TODO: fetch cert from cave-vault at kubeconfig_ref.vault_path
    // TODO: parse x509 certificate and compute days until NotAfter
    let days_remaining: i64 = 365; // TODO: real value from cert

    if days_remaining <= 7 {
        warn!(
            vault_path = %kubeconfig_ref.vault_path,
            days = days_remaining,
            "Cluster certificate expiry CRITICAL"
        );
    } else if days_remaining <= 30 {
        warn!(
            vault_path = %kubeconfig_ref.vault_path,
            days = days_remaining,
            "Cluster certificate expiry warning"
        );
    }

    Some(days_remaining)
}

/// Synthesize a `ClusterHealth` for a cluster that cannot be reached.
pub fn unreachable_health(cluster_id: Uuid, reason: &str) -> ClusterHealth {
    ClusterHealth {
        cluster_id,
        api_server_status: HealthStatus::Unreachable,
        etcd_health: HealthStatus::Unreachable,
        node_readiness: NodeReadiness {
            total: 0,
            ready: 0,
            not_ready: 0,
            cordoned: 0,
        },
        component_statuses: vec![ComponentStatus {
            name: "api-server".into(),
            healthy: false,
            message: Some(reason.into()),
        }],
        checked_at: Utc::now(),
=======
use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::cluster::Cluster;
use crate::node::{ClusterNode, NodeStatus};

// ── Component health ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComponentStatus {
    Healthy,
    Degraded,
    Unknown,
    NotFound,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentHealth {
    /// Component name, e.g. `"api-server"`, `"etcd"`, `"scheduler"`.
    pub name: String,
    pub status: ComponentStatus,
    pub message: Option<String>,
    pub last_checked: chrono::DateTime<chrono::Utc>,
}

impl ComponentHealth {
    fn healthy(name: &str) -> Self {
        Self {
            name: name.to_string(),
            status: ComponentStatus::Healthy,
            message: None,
            last_checked: Utc::now(),
        }
    }
}

// ── Resource usage ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeResourceUsage {
    pub node_id: Uuid,
    pub cpu_percent: f64,
    pub memory_percent: f64,
    pub disk_percent: f64,
    pub pod_count: u32,
    pub recorded_at: chrono::DateTime<chrono::Utc>,
}

// ── Health report ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterHealthReport {
    pub cluster_id: Uuid,
    pub overall_status: ComponentStatus,
    pub control_plane_components: Vec<ComponentHealth>,
    pub node_statuses: HashMap<Uuid, NodeStatus>,
    pub total_nodes: usize,
    pub ready_nodes: usize,
    pub resource_usage: Vec<NodeResourceUsage>,
    pub checked_at: chrono::DateTime<chrono::Utc>,
}

impl ClusterHealthReport {
    pub fn is_fully_healthy(&self) -> bool {
        self.overall_status == ComponentStatus::Healthy && self.ready_nodes == self.total_nodes
    }

    pub fn ready_percentage(&self) -> f64 {
        if self.total_nodes == 0 {
            0.0
        } else {
            self.ready_nodes as f64 / self.total_nodes as f64 * 100.0
        }
    }
}

// ── Checker ───────────────────────────────────────────────────────────────────

pub struct ClusterHealthChecker {
    reports: Arc<RwLock<HashMap<Uuid, ClusterHealthReport>>>,
}

impl ClusterHealthChecker {
    pub fn new() -> Self {
        Self { reports: Arc::new(RwLock::new(HashMap::new())) }
    }

    pub async fn update_report(&self, report: ClusterHealthReport) {
        let mut guard = self.reports.write().await;
        guard.insert(report.cluster_id, report);
    }

    pub async fn get_report(&self, cluster_id: Uuid) -> Option<ClusterHealthReport> {
        let guard = self.reports.read().await;
        guard.get(&cluster_id).cloned()
    }

    pub async fn record_node_usage(&self, cluster_id: Uuid, usage: NodeResourceUsage) {
        let mut guard = self.reports.write().await;
        if let Some(report) = guard.get_mut(&cluster_id) {
            // Replace existing usage record for the node or append.
            if let Some(pos) = report.resource_usage.iter().position(|u| u.node_id == usage.node_id) {
                report.resource_usage[pos] = usage;
            } else {
                report.resource_usage.push(usage);
            }
        }
    }

    /// Build a `ClusterHealthReport` by inspecting the given cluster and its nodes.
    pub async fn check_cluster(
        &self,
        cluster: &Cluster,
        nodes: &[ClusterNode],
    ) -> ClusterHealthReport {
        let now = Utc::now();

        // Control-plane components — simulated as healthy when cluster is Running.
        let component_names = ["api-server", "etcd", "scheduler", "controller-manager"];
        let control_plane_components: Vec<ComponentHealth> =
            component_names.iter().map(|n| ComponentHealth::healthy(n)).collect();

        let node_statuses: HashMap<Uuid, NodeStatus> =
            nodes.iter().map(|n| (n.id, n.status.clone())).collect();

        let total_nodes = nodes.len();
        let ready_nodes = nodes.iter().filter(|n| n.status == NodeStatus::Ready).count();

        let overall_status = if ready_nodes == total_nodes && total_nodes > 0 {
            ComponentStatus::Healthy
        } else if ready_nodes > 0 {
            ComponentStatus::Degraded
        } else {
            ComponentStatus::Unknown
        };

        let report = ClusterHealthReport {
            cluster_id: cluster.id,
            overall_status,
            control_plane_components,
            node_statuses,
            total_nodes,
            ready_nodes,
            resource_usage: Vec::new(),
            checked_at: now,
        };

        self.update_report(report.clone()).await;
        report
    }
}

impl Default for ClusterHealthChecker {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::{Cluster, ClusterProvider, ClusterSpec, ClusterState, KubernetesDistro};
    use crate::node::{NodeResources, NodeRole};

    fn make_cluster() -> Cluster {
        let spec = ClusterSpec {
            name: "test".to_string(),
            provider: ClusterProvider::BareMetal,
            distro: KubernetesDistro::K3s,
            kubernetes_version: "v1.29.0".to_string(),
            control_plane_count: 1,
            worker_count: 2,
            region: "eu".to_string(),
            tenant_id: "t1".to_string(),
        };
        let mut c = Cluster::new(spec, Uuid::new_v4());
        c.state = ClusterState::Running;
        c
    }

    fn ready_node(cluster_id: Uuid) -> ClusterNode {
        let mut n = ClusterNode::new(
            cluster_id,
            "node",
            "10.0.0.1",
            NodeRole::Worker,
            NodeResources::default(),
        );
        n.status = NodeStatus::Ready;
        n
    }

    fn not_ready_node(cluster_id: Uuid) -> ClusterNode {
        ClusterNode::new(
            cluster_id,
            "bad-node",
            "10.0.0.2",
            NodeRole::Worker,
            NodeResources::default(),
        )
        // status stays Pending / NotReady
    }

    #[tokio::test]
    async fn test_healthy_cluster_report() {
        let checker = ClusterHealthChecker::new();
        let cluster = make_cluster();
        let nodes = vec![ready_node(cluster.id), ready_node(cluster.id)];
        let report = checker.check_cluster(&cluster, &nodes).await;

        assert_eq!(report.overall_status, ComponentStatus::Healthy);
        assert!(report.is_fully_healthy());
        assert_eq!(report.total_nodes, 2);
        assert_eq!(report.ready_nodes, 2);
        assert_eq!(report.ready_percentage(), 100.0);
    }

    #[tokio::test]
    async fn test_degraded_when_nodes_not_ready() {
        let checker = ClusterHealthChecker::new();
        let cluster = make_cluster();
        let nodes = vec![ready_node(cluster.id), not_ready_node(cluster.id)];
        let report = checker.check_cluster(&cluster, &nodes).await;

        assert_eq!(report.overall_status, ComponentStatus::Degraded);
        assert!(!report.is_fully_healthy());
        assert_eq!(report.ready_nodes, 1);
    }

    #[tokio::test]
    async fn test_ready_percentage_calculation() {
        let checker = ClusterHealthChecker::new();
        let cluster = make_cluster();
        let nodes = vec![
            ready_node(cluster.id),
            ready_node(cluster.id),
            not_ready_node(cluster.id),
            not_ready_node(cluster.id),
        ];
        let report = checker.check_cluster(&cluster, &nodes).await;
        assert!((report.ready_percentage() - 50.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_ready_percentage_no_nodes() {
        let checker = ClusterHealthChecker::new();
        let cluster = make_cluster();
        let report = checker.check_cluster(&cluster, &[]).await;
        assert_eq!(report.ready_percentage(), 0.0);
    }

    #[tokio::test]
    async fn test_get_report_after_check() {
        let checker = ClusterHealthChecker::new();
        let cluster = make_cluster();
        let nodes = vec![ready_node(cluster.id)];
        checker.check_cluster(&cluster, &nodes).await;

        let stored = checker.get_report(cluster.id).await;
        assert!(stored.is_some());
        assert_eq!(stored.unwrap().cluster_id, cluster.id);
>>>>>>> claude/great-sanderson
    }
}
