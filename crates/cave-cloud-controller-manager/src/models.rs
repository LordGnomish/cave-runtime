//! Shared types for the cloud controllers.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Per-controller metadata + status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudControllerInfo {
    pub name: String,
    pub state: CloudControllerState,
    pub last_sync: Option<DateTime<Utc>>,
    pub reconcile_count: u64,
    pub error_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloudControllerState {
    Stopped,
    Running,
    Failed,
}

impl CloudControllerInfo {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            state: CloudControllerState::Stopped,
            last_sync: None,
            reconcile_count: 0,
            error_count: 0,
        }
    }
}

/// Provider-supplied metadata about an instance backing a Node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceMetadata {
    pub provider_id: String,
    pub instance_type: String,
    pub zone: String,
    pub region: String,
    pub addresses: Vec<NodeAddress>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeAddress {
    pub kind: String, // "InternalIP" | "ExternalIP" | "Hostname"
    pub value: String,
}

/// A node that the cloud controllers are responsible for.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudNode {
    pub name: String,
    pub provider_id: Option<String>,
    pub initialized: bool,
    pub pod_cidr: Option<String>,
    pub addresses: Vec<NodeAddress>,
    pub instance_type: Option<String>,
    pub zone: Option<String>,
}

impl CloudNode {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            provider_id: None,
            initialized: false,
            pod_cidr: None,
            addresses: vec![],
            instance_type: None,
            zone: None,
        }
    }
}

/// A Service that the LB controller is responsible for.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LbService {
    pub name: String,
    pub namespace: String,
    pub service_type: String, // "ClusterIP" | "NodePort" | "LoadBalancer"
    pub external_ip: Option<String>,
}

/// A VPC route entry the route controller programs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudRoute {
    pub destination_cidr: String,
    pub target_node: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cloud_controller_info_starts_stopped() {
        let info = CloudControllerInfo::new("node");
        assert_eq!(info.state, CloudControllerState::Stopped);
        assert_eq!(info.reconcile_count, 0);
    }

    #[test]
    fn test_cloud_node_new_uninitialized() {
        let n = CloudNode::new("worker-1");
        assert_eq!(n.name, "worker-1");
        assert!(!n.initialized);
        assert!(n.pod_cidr.is_none());
    }
}
