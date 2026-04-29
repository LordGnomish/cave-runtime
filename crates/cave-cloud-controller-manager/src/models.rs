//! Data models for the cloud controller manager.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudNode {
    pub name: String,
    pub provider_id: String,
    pub region: String,
    pub zone: String,
    pub instance_type: String,
    pub addresses: Vec<NodeAddress>,
    pub status: CloudNodeStatus,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloudNodeStatus {
    Pending,
    Ready,
    Terminating,
    Terminated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeAddress {
    pub kind: NodeAddressKind,
    pub address: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeAddressKind {
    InternalIP,
    ExternalIP,
    Hostname,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadBalancer {
    pub service_name: String,
    pub namespace: String,
    pub provider_lb_id: Option<String>,
    pub external_ip: Option<String>,
    pub backends: Vec<String>,
    pub status: LoadBalancerStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoadBalancerStatus {
    Pending,
    Provisioned,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudRoute {
    pub name: String,
    pub destination_cidr: String,
    pub next_hop_node: String,
    pub provider_route_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cloud_node_status_serialization() {
        let s = serde_json::to_string(&CloudNodeStatus::Ready).unwrap();
        assert_eq!(s, "\"Ready\"");
    }

    #[test]
    fn test_load_balancer_status_default_pending_is_serializable() {
        let lb = LoadBalancer {
            service_name: "web".into(), namespace: "default".into(),
            provider_lb_id: None, external_ip: None,
            backends: vec![], status: LoadBalancerStatus::Pending,
        };
        let s = serde_json::to_string(&lb).unwrap();
        assert!(s.contains("\"Pending\""));
    }

    #[test]
    fn test_node_address_kinds_distinct() {
        assert_ne!(NodeAddressKind::InternalIP, NodeAddressKind::ExternalIP);
    }
}
