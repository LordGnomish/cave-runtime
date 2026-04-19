//! Scheduler models — nodes, resource capacity, scheduling decisions.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// A compute node in the cluster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub name: String,
    pub uid: Uuid,
    pub status: NodeStatus,
    pub capacity: ResourceCapacity,
    pub allocatable: ResourceCapacity,
    pub allocated: ResourceCapacity,
    pub labels: HashMap<String, String>,
    pub taints: Vec<Taint>,
    pub conditions: Vec<NodeCondition>,
    pub registered_at: DateTime<Utc>,
    pub last_heartbeat: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceCapacity {
    pub cpu_millicores: u64,
    pub memory_bytes: u64,
    pub pods: u64,
    pub ephemeral_storage_bytes: u64,
}

impl ResourceCapacity {
    pub fn has_room_for(&self, request: &ResourceRequest) -> bool {
        self.cpu_millicores >= request.cpu_millicores
            && self.memory_bytes >= request.memory_bytes
    }

    pub fn subtract(&mut self, request: &ResourceRequest) {
        self.cpu_millicores = self.cpu_millicores.saturating_sub(request.cpu_millicores);
        self.memory_bytes = self.memory_bytes.saturating_sub(request.memory_bytes);
        self.pods = self.pods.saturating_sub(1);
    }

    pub fn add(&mut self, request: &ResourceRequest) {
        self.cpu_millicores += request.cpu_millicores;
        self.memory_bytes += request.memory_bytes;
        self.pods += 1;
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceRequest {
    pub cpu_millicores: u64,
    pub memory_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeStatus {
    Ready,
    NotReady,
    Cordoned,
    Draining,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Taint {
    pub key: String,
    pub value: Option<String>,
    pub effect: TaintEffect,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaintEffect {
    NoSchedule,
    PreferNoSchedule,
    NoExecute,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeCondition {
    pub condition_type: String,
    pub status: String,
    pub last_heartbeat_time: DateTime<Utc>,
    pub reason: String,
}

/// A pod scheduling request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleRequest {
    pub pod_name: String,
    pub namespace: String,
    pub resources: ResourceRequest,
    pub node_selector: HashMap<String, String>,
    pub tolerations: Vec<Toleration>,
    pub affinity: Option<Affinity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Toleration {
    pub key: Option<String>,
    pub operator: String,
    pub value: Option<String>,
    pub effect: Option<TaintEffect>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Affinity {
    pub preferred_nodes: Vec<String>,
    pub required_labels: HashMap<String, String>,
}

/// Scheduling decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleResult {
    pub pod_name: String,
    pub namespace: String,
    pub node_name: Option<String>,
    pub reason: String,
    pub scored_nodes: Vec<ScoredNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredNode {
    pub name: String,
    pub score: u64,
    pub reasons: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resource_capacity_has_room() {
        let cap = ResourceCapacity { cpu_millicores: 4000, memory_bytes: 8_000_000_000, pods: 110, ephemeral_storage_bytes: 0 };
        let req = ResourceRequest { cpu_millicores: 500, memory_bytes: 1_000_000_000 };
        assert!(cap.has_room_for(&req));
    }

    #[test]
    fn test_resource_capacity_no_room() {
        let cap = ResourceCapacity { cpu_millicores: 100, memory_bytes: 500, pods: 1, ephemeral_storage_bytes: 0 };
        let req = ResourceRequest { cpu_millicores: 500, memory_bytes: 1000 };
        assert!(!cap.has_room_for(&req));
    }

    #[test]
    fn test_subtract_and_add() {
        let mut cap = ResourceCapacity { cpu_millicores: 4000, memory_bytes: 8000, pods: 10, ephemeral_storage_bytes: 0 };
        let req = ResourceRequest { cpu_millicores: 1000, memory_bytes: 2000 };
        cap.subtract(&req);
        assert_eq!(cap.cpu_millicores, 3000);
        assert_eq!(cap.pods, 9);
        cap.add(&req);
        assert_eq!(cap.cpu_millicores, 4000);
        assert_eq!(cap.pods, 10);
    }
}
