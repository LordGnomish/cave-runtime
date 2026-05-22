// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Role & Status ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeRole {
    ControlPlane,
    Worker,
    Etcd,
    ControlPlaneWorker,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    Pending,
    Joining,
    Ready,
    NotReady,
    Draining,
    Drained,
    Removing,
    Removed,
}

// ── Resources ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeResources {
    pub cpu_cores: u32,
    pub memory_gb: u32,
    pub disk_gb: u32,
    pub gpu_count: u32,
}

impl Default for NodeResources {
    fn default() -> Self {
        Self { cpu_cores: 2, memory_gb: 4, disk_gb: 50, gpu_count: 0 }
    }
}

// ── Node ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterNode {
    pub id: Uuid,
    pub cluster_id: Uuid,
    pub hostname: String,
    pub ip_address: String,
    pub role: NodeRole,
    pub status: NodeStatus,
    pub kubernetes_version: String,
    pub resources: NodeResources,
    pub labels: HashMap<String, String>,
    pub taints: Vec<String>,
    pub joined_at: Option<DateTime<Utc>>,
    pub last_heartbeat: Option<DateTime<Utc>>,
}

impl ClusterNode {
    pub fn new(
        cluster_id: Uuid,
        hostname: &str,
        ip: &str,
        role: NodeRole,
        resources: NodeResources,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            cluster_id,
            hostname: hostname.to_string(),
            ip_address: ip.to_string(),
            role,
            status: NodeStatus::Pending,
            kubernetes_version: String::new(),
            resources,
            labels: HashMap::new(),
            taints: Vec::new(),
            joined_at: None,
            last_heartbeat: None,
        }
    }

    /// A node is schedulable when it is `Ready` and has no taints.
    pub fn is_schedulable(&self) -> bool {
        self.status == NodeStatus::Ready && self.taints.is_empty()
    }

    /// Returns `true` for roles that include control-plane responsibilities.
    pub fn is_control_plane(&self) -> bool {
        matches!(self.role, NodeRole::ControlPlane | NodeRole::ControlPlaneWorker)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn resources() -> NodeResources {
        NodeResources { cpu_cores: 8, memory_gb: 16, disk_gb: 200, gpu_count: 0 }
    }

    #[test]
    fn test_node_creation_defaults() {
        let cid = Uuid::new_v4();
        let node = ClusterNode::new(cid, "node-1", "192.168.1.10", NodeRole::Worker, resources());
        assert_eq!(node.status, NodeStatus::Pending);
        assert_eq!(node.cluster_id, cid);
        assert!(node.taints.is_empty());
        assert!(node.joined_at.is_none());
    }

    #[test]
    fn test_is_schedulable_ready_no_taints() {
        let cid = Uuid::new_v4();
        let mut node =
            ClusterNode::new(cid, "node-1", "192.168.1.10", NodeRole::Worker, resources());
        node.status = NodeStatus::Ready;
        assert!(node.is_schedulable());
    }

    #[test]
    fn test_is_schedulable_false_with_taint() {
        let cid = Uuid::new_v4();
        let mut node =
            ClusterNode::new(cid, "node-1", "192.168.1.10", NodeRole::Worker, resources());
        node.status = NodeStatus::Ready;
        node.taints.push("NoSchedule:dedicated=gpu".to_string());
        assert!(!node.is_schedulable());
    }

    #[test]
    fn test_is_schedulable_false_not_ready() {
        let cid = Uuid::new_v4();
        let node =
            ClusterNode::new(cid, "node-1", "192.168.1.10", NodeRole::Worker, resources());
        // status is Pending by default
        assert!(!node.is_schedulable());
    }

    #[test]
    fn test_is_control_plane_true() {
        let cid = Uuid::new_v4();
        let cp = ClusterNode::new(cid, "cp-1", "10.0.0.1", NodeRole::ControlPlane, resources());
        assert!(cp.is_control_plane());

        let cpw =
            ClusterNode::new(cid, "cpw-1", "10.0.0.2", NodeRole::ControlPlaneWorker, resources());
        assert!(cpw.is_control_plane());
    }

    #[test]
    fn test_is_control_plane_false_for_worker() {
        let cid = Uuid::new_v4();
        let worker =
            ClusterNode::new(cid, "worker-1", "10.0.0.3", NodeRole::Worker, resources());
        assert!(!worker.is_control_plane());
    }
}
