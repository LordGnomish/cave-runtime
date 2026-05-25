// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Kubelet models — pod status, node status, container state.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Pod managed by this kubelet instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedPod {
    pub uid: Uuid,
    pub name: String,
    pub namespace: String,
    pub containers: Vec<ManagedContainer>,
    pub status: PodPhase,
    pub assigned_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub node_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PodPhase {
    Pending,
    Running,
    Succeeded,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedContainer {
    pub name: String,
    pub image: String,
    pub container_id: Option<Uuid>,
    pub state: ContainerState,
    pub restart_count: u32,
    pub ready: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerState {
    Waiting {
        reason: String,
    },
    Running {
        started_at: DateTime<Utc>,
    },
    Terminated {
        exit_code: i32,
        reason: String,
        finished_at: DateTime<Utc>,
    },
}

/// Node status report sent to API server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeStatusReport {
    pub node_name: String,
    pub ready: bool,
    pub cpu_capacity_millicores: u64,
    pub memory_capacity_bytes: u64,
    pub cpu_used_millicores: u64,
    pub memory_used_bytes: u64,
    pub pod_count: u32,
    pub pod_capacity: u32,
    pub conditions: Vec<NodeCondition>,
    pub reported_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeCondition {
    pub condition_type: String,
    pub status: bool,
    pub message: String,
}

impl NodeStatusReport {
    pub fn healthy(node_name: &str) -> Self {
        Self {
            node_name: node_name.to_string(),
            ready: true,
            cpu_capacity_millicores: 8000,
            memory_capacity_bytes: 16_000_000_000,
            cpu_used_millicores: 0,
            memory_used_bytes: 0,
            pod_count: 0,
            pod_capacity: 110,
            conditions: vec![
                NodeCondition {
                    condition_type: "Ready".into(),
                    status: true,
                    message: "kubelet is ready".into(),
                },
                NodeCondition {
                    condition_type: "MemoryPressure".into(),
                    status: false,
                    message: "".into(),
                },
                NodeCondition {
                    condition_type: "DiskPressure".into(),
                    status: false,
                    message: "".into(),
                },
                NodeCondition {
                    condition_type: "PIDPressure".into(),
                    status: false,
                    message: "".into(),
                },
            ],
            reported_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_healthy_node_status() {
        let s = NodeStatusReport::healthy("node1");
        assert!(s.ready);
        assert_eq!(s.conditions.len(), 4);
    }

    #[test]
    fn test_pod_phase_serialization() {
        let p = PodPhase::Running;
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(json, "\"Running\"");
    }

    #[test]
    fn test_container_state() {
        let s = ContainerState::Waiting {
            reason: "PullImage".into(),
        };
        assert_eq!(
            s,
            ContainerState::Waiting {
                reason: "PullImage".into()
            }
        );
    }
}
