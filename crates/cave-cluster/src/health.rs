//! Cluster health monitoring.

use crate::cluster::{Cluster, ClusterStatus};
use crate::error::ClusterResult;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ── Health check types ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentHealth {
    pub name: String,
    pub status: HealthStatus,
    pub message: Option<String>,
    pub last_checked: DateTime<Utc>,
}

/// Cluster health summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterHealth {
    pub cluster_name: String,
    pub overall: HealthStatus,
    pub components: Vec<ComponentHealth>,
    pub node_summary: NodeSummary,
    pub checked_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSummary {
    pub total: i32,
    pub ready: i32,
    pub not_ready: i32,
    pub unknown: i32,
}

impl NodeSummary {
    pub fn all_ready(count: i32) -> Self {
        Self { total: count, ready: count, not_ready: 0, unknown: 0 }
    }
}

// ── Health checker ────────────────────────────────────────────────────────────

/// Evaluate the health of a cluster (in-memory simulation).
pub fn check_cluster_health(
    cluster: &Cluster,
    node_count: i32,
) -> ClusterHealth {
    let overall = match cluster.status {
        ClusterStatus::Running => HealthStatus::Healthy,
        ClusterStatus::Upgrading | ClusterStatus::Scaling => HealthStatus::Degraded,
        ClusterStatus::Failed => HealthStatus::Unhealthy,
        _ => HealthStatus::Unknown,
    };

    let components = vec![
        ComponentHealth {
            name: "api-server".into(),
            status: if cluster.status == ClusterStatus::Running {
                HealthStatus::Healthy
            } else {
                HealthStatus::Degraded
            },
            message: None,
            last_checked: Utc::now(),
        },
        ComponentHealth {
            name: "etcd".into(),
            status: HealthStatus::Healthy,
            message: None,
            last_checked: Utc::now(),
        },
        ComponentHealth {
            name: "controller-manager".into(),
            status: if cluster.status == ClusterStatus::Running {
                HealthStatus::Healthy
            } else {
                HealthStatus::Degraded
            },
            message: None,
            last_checked: Utc::now(),
        },
        ComponentHealth {
            name: "scheduler".into(),
            status: HealthStatus::Healthy,
            message: None,
            last_checked: Utc::now(),
        },
        ComponentHealth {
            name: "coredns".into(),
            status: HealthStatus::Healthy,
            message: None,
            last_checked: Utc::now(),
        },
    ];

    ClusterHealth {
        cluster_name: cluster.spec.name.clone(),
        overall,
        components,
        node_summary: NodeSummary::all_ready(node_count),
        checked_at: Utc::now(),
    }
}

/// Control plane endpoint health check.
pub fn check_endpoint(endpoint: &str) -> EndpointHealth {
    // In a real impl, we'd make an HTTP request to /healthz
    EndpointHealth {
        endpoint: endpoint.to_string(),
        reachable: !endpoint.is_empty(),
        latency_ms: Some(5),
        status_code: Some(200),
        checked_at: Utc::now(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointHealth {
    pub endpoint: String,
    pub reachable: bool,
    pub latency_ms: Option<u64>,
    pub status_code: Option<u16>,
    pub checked_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::{Cluster, ClusterSpec, NetworkConfig};
    use std::collections::HashMap;

    fn running_cluster() -> Cluster {
        let spec = ClusterSpec {
            name: "healthy-cluster".into(),
            kubernetes_version: "1.30".into(),
            region: "eu-west-1".into(),
            network: NetworkConfig::default(),
            tags: HashMap::new(),
            enable_rbac: true,
            audit_logging: false,
        };
        let mut c = Cluster::new(spec, "alice".into());
        c.transition(ClusterStatus::Running);
        c
    }

    #[test]
    fn healthy_cluster_check() {
        let cluster = running_cluster();
        let health = check_cluster_health(&cluster, 3);
        assert_eq!(health.overall, HealthStatus::Healthy);
        assert_eq!(health.node_summary.ready, 3);
        assert!(health.components.iter().all(|c| c.status == HealthStatus::Healthy));
    }

    #[test]
    fn failed_cluster_is_unhealthy() {
        let mut cluster = running_cluster();
        cluster.fail("disk full".into());
        let health = check_cluster_health(&cluster, 3);
        assert_eq!(health.overall, HealthStatus::Unhealthy);
    }

    #[test]
    fn endpoint_check() {
        let h = check_endpoint("https://cluster.cave.internal:6443");
        assert!(h.reachable);
    }
}
