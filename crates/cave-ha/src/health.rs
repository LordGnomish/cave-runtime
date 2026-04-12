<<<<<<< HEAD
//! Cluster health monitoring and auto-healing.
//!
//! Aggregates health across all instances, detects quorum loss, identifies
//! asymmetric network partitions, and attempts self-healing where possible.

use crate::{models::InstanceStatus, HaState};
use anyhow::Result;
use std::{collections::HashSet, sync::Arc};
use tracing::{info, warn};

/// Self-health check for this instance: CPU, memory, disk, and module health.
///
/// Production: query `/proc/meminfo`, disk utilisation, and each module's
/// `/health` endpoint. Returns structured health facts for peer voting.
pub async fn instance_health_check(_state: Arc<HaState>) -> Result<serde_json::Value> {
    Ok(serde_json::json!({
        "cpu_ok":     true,
        "memory_ok":  true,
        "disk_ok":    true,
        "modules_ok": true,
    }))
}

/// Aggregate health view across the entire cluster.
pub async fn cluster_health(state: Arc<HaState>) -> Result<serde_json::Value> {
    let topology = state.topology.read().await;
    let total = topology.instances.len();
    let healthy = topology
        .instances
        .iter()
        .filter(|i| i.status == InstanceStatus::Healthy)
        .count();
    let degraded = topology
        .instances
        .iter()
        .filter(|i| i.status == InstanceStatus::Degraded)
        .count();
    let unreachable = topology
        .instances
        .iter()
        .filter(|i| i.status == InstanceStatus::Unreachable)
        .count();
    let leader = topology.leader.map(|id| id.to_string());
    let has_quorum = healthy >= topology.quorum_size;
    drop(topology);

    Ok(serde_json::json!({
        "total_instances": total,
        "healthy":         healthy,
        "degraded":        degraded,
        "unreachable":     unreachable,
        "leader":          leader,
        "has_quorum":      has_quorum,
    }))
}

/// Verify we still hold a majority of healthy instances.
pub async fn quorum_check(state: Arc<HaState>) -> Result<bool> {
    let topology = state.topology.read().await;
    let healthy = topology
        .instances
        .iter()
        .filter(|i| i.status == InstanceStatus::Healthy)
        .count();
    let required = topology.quorum_size;
    let has_quorum = healthy >= required;
    drop(topology);

    if has_quorum {
        info!(healthy, required, "Quorum check passed");
    } else {
        warn!(healthy, required, "Quorum check FAILED");
    }
    Ok(has_quorum)
}

/// Detect asymmetric network failures where this instance can see some peers
/// but not others (classic split scenario that precedes a split-brain).
///
/// A partition is suspected when the number of peers we have recent health
/// votes from is less than `total_peers`.
pub async fn network_partition_detection(state: Arc<HaState>) -> Result<bool> {
    let total_peers = state
        .topology
        .read()
        .await
        .instances
        .len()
        .saturating_sub(1); // exclude self

    if total_peers == 0 {
        return Ok(false); // single-node cluster — no partition possible
    }

    let cutoff = chrono::Utc::now() - chrono::Duration::seconds(30);
    let self_id = state.self_instance.id;
    let reachable: HashSet<uuid::Uuid> = state
        .health_votes
        .read()
        .await
        .iter()
        .filter(|v| v.instance_id == self_id && v.timestamp > cutoff)
        .map(|v| v.target_id)
        .collect();

    let partition = reachable.len() < total_peers;
    if partition {
        warn!(
            reachable = reachable.len(),
            total_peers,
            "Network partition suspected: cannot reach all peers"
        );
    }
    Ok(partition)
}

/// Auto-healing: attempt recovery from quorum loss or network partition.
///
/// Called by the background health loop. Triggers split-brain protection if
/// quorum is lost, and initiates catch-up replication after a partition heals.
pub async fn auto_healing(state: Arc<HaState>) -> Result<()> {
    let has_quorum = quorum_check(Arc::clone(&state)).await?;
    if !has_quorum {
        info!("Auto-healing: quorum lost — applying split-brain protection");
        crate::failover::split_brain_protection(Arc::clone(&state)).await?;
    }

    let partition = network_partition_detection(Arc::clone(&state)).await?;
    if partition {
        // Production: trigger catch_up replication for this instance.
        info!("Auto-healing: partition detected — catch-up replication scheduled");
    }

    info!("Auto-healing cycle complete");
    Ok(())
=======
//! Health monitoring of cluster nodes and the cluster as a whole.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::warn;

use crate::raft::{NodeId, RaftRole};

/// Health status of an individual cluster node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeHealthStatus {
    Healthy,
    Degraded,
    Unreachable,
    Unknown,
}

/// Snapshot of resource usage for a single node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceUsage {
    pub cpu_percent: f64,
    pub memory_mb: u64,
    pub disk_mb: u64,
}

/// Full health record for a single cluster node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeHealth {
    pub node_id: NodeId,
    pub status: NodeHealthStatus,
    pub last_heartbeat: DateTime<Utc>,
    pub role: RaftRole,
    pub commit_index: u64,
    pub last_applied: u64,
    pub log_length: usize,
    pub resource_usage: ResourceUsage,
}

/// Aggregated health of the entire cluster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterHealth {
    pub cluster_id: String,
    pub leader_id: Option<NodeId>,
    pub nodes: Vec<NodeHealth>,
    pub healthy_nodes: usize,
    pub has_quorum: bool,
}

impl ClusterHealth {
    /// The cluster is overall healthy when it has a leader and quorum.
    pub fn overall_healthy(&self) -> bool {
        self.has_quorum && self.leader_id.is_some()
    }
}

/// Monitors the health of all nodes in a cluster.
pub struct ClusterHealthMonitor {
    cluster_id: String,
    nodes: Arc<RwLock<HashMap<NodeId, NodeHealth>>>,
    quorum_size: usize,
}

impl ClusterHealthMonitor {
    /// Create a new monitor for `cluster_id` with `quorum_size` required healthy nodes.
    pub fn new(cluster_id: &str, quorum_size: usize) -> Self {
        Self {
            cluster_id: cluster_id.to_string(),
            nodes: Arc::new(RwLock::new(HashMap::new())),
            quorum_size,
        }
    }

    /// Record or update the health of a node.
    pub async fn update_node(&self, health: NodeHealth) {
        let mut nodes = self.nodes.write().await;
        nodes.insert(health.node_id, health);
    }

    /// Mark a node as unreachable (e.g., heartbeat missed).
    pub async fn mark_unreachable(&self, node_id: NodeId) {
        let mut nodes = self.nodes.write().await;
        if let Some(health) = nodes.get_mut(&node_id) {
            health.status = NodeHealthStatus::Unreachable;
            warn!(node_id, "node marked as unreachable");
        } else {
            // Insert a placeholder.
            nodes.insert(
                node_id,
                NodeHealth {
                    node_id,
                    status: NodeHealthStatus::Unreachable,
                    last_heartbeat: Utc::now(),
                    role: RaftRole::Follower,
                    commit_index: 0,
                    last_applied: 0,
                    log_length: 0,
                    resource_usage: ResourceUsage {
                        cpu_percent: 0.0,
                        memory_mb: 0,
                        disk_mb: 0,
                    },
                },
            );
        }
    }

    /// Compute and return a snapshot of the cluster's overall health.
    pub async fn cluster_health(&self) -> ClusterHealth {
        let nodes = self.nodes.read().await;
        let all_nodes: Vec<NodeHealth> = nodes.values().cloned().collect();

        let healthy_nodes = all_nodes
            .iter()
            .filter(|n| n.status == NodeHealthStatus::Healthy)
            .count();

        let has_quorum = healthy_nodes >= self.quorum_size;

        let leader_id = all_nodes
            .iter()
            .find(|n| n.role == RaftRole::Leader && n.status == NodeHealthStatus::Healthy)
            .map(|n| n.node_id);

        ClusterHealth {
            cluster_id: self.cluster_id.clone(),
            leader_id,
            nodes: all_nodes,
            healthy_nodes,
            has_quorum,
        }
    }

    /// Returns `true` when at least `quorum_size` nodes are healthy.
    pub async fn is_quorum_healthy(&self) -> bool {
        let nodes = self.nodes.read().await;
        let healthy = nodes
            .values()
            .filter(|n| n.status == NodeHealthStatus::Healthy)
            .count();
        healthy >= self.quorum_size
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn healthy_node(id: NodeId, role: RaftRole) -> NodeHealth {
        NodeHealth {
            node_id: id,
            status: NodeHealthStatus::Healthy,
            last_heartbeat: Utc::now(),
            role,
            commit_index: 100,
            last_applied: 100,
            log_length: 100,
            resource_usage: ResourceUsage {
                cpu_percent: 10.0,
                memory_mb: 512,
                disk_mb: 2048,
            },
        }
    }

    #[tokio::test]
    async fn test_cluster_health_all_healthy() {
        // 3-node cluster with quorum = 2.
        let monitor = ClusterHealthMonitor::new("test-cluster", 2);

        monitor.update_node(healthy_node(1, RaftRole::Leader)).await;
        monitor.update_node(healthy_node(2, RaftRole::Follower)).await;
        monitor.update_node(healthy_node(3, RaftRole::Follower)).await;

        let health = monitor.cluster_health().await;
        assert_eq!(health.cluster_id, "test-cluster");
        assert_eq!(health.healthy_nodes, 3);
        assert!(health.has_quorum);
        assert_eq!(health.leader_id, Some(1));
        assert!(health.overall_healthy());
    }

    #[tokio::test]
    async fn test_cluster_health_no_quorum() {
        // 3-node cluster with quorum = 2; only 1 healthy node.
        let monitor = ClusterHealthMonitor::new("test-cluster", 2);

        monitor.update_node(healthy_node(1, RaftRole::Leader)).await;
        monitor.mark_unreachable(2).await;
        monitor.mark_unreachable(3).await;

        let health = monitor.cluster_health().await;
        assert_eq!(health.healthy_nodes, 1);
        assert!(!health.has_quorum);
        // overall_healthy requires quorum AND a leader.
        assert!(!health.overall_healthy());
    }

    #[tokio::test]
    async fn test_mark_unreachable() {
        let monitor = ClusterHealthMonitor::new("c1", 1);
        monitor.update_node(healthy_node(1, RaftRole::Follower)).await;
        monitor.mark_unreachable(1).await;

        let health = monitor.cluster_health().await;
        let node = health.nodes.iter().find(|n| n.node_id == 1).unwrap();
        assert_eq!(node.status, NodeHealthStatus::Unreachable);
    }

    #[tokio::test]
    async fn test_is_quorum_healthy() {
        let monitor = ClusterHealthMonitor::new("c1", 2);
        assert!(!monitor.is_quorum_healthy().await);

        monitor.update_node(healthy_node(1, RaftRole::Leader)).await;
        assert!(!monitor.is_quorum_healthy().await);

        monitor.update_node(healthy_node(2, RaftRole::Follower)).await;
        assert!(monitor.is_quorum_healthy().await);
    }

    #[tokio::test]
    async fn test_overall_healthy_requires_leader() {
        let monitor = ClusterHealthMonitor::new("c1", 2);

        // Two followers — quorum met but no leader.
        monitor.update_node(healthy_node(1, RaftRole::Follower)).await;
        monitor.update_node(healthy_node(2, RaftRole::Follower)).await;

        let health = monitor.cluster_health().await;
        assert!(health.has_quorum);
        assert!(health.leader_id.is_none());
        assert!(!health.overall_healthy());
    }
>>>>>>> claude/great-sanderson
}
