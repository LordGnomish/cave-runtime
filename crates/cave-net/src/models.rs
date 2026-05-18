// SPDX-License-Identifier: AGPL-3.0-or-later
//! Networking models — pods, services, endpoints, network policies.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;
use uuid::Uuid;

/// Pod network identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodNetwork {
    /// The name of the pod.
    pub pod_name: String,
    /// The Kubernetes namespace the pod belongs to.
    pub namespace: String,
    /// The IP address assigned to the pod.
    pub pod_ip: IpAddr,
    /// The name of the node hosting the pod.
    pub node_name: String,
    /// Key-value labels attached to the pod.
    pub labels: HashMap<String, String>,
    /// The timestamp when the pod was created.
    pub created_at: DateTime<Utc>,
}

/// Service (ClusterIP, NodePort, LoadBalancer).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceEntry {
    /// The name of the service.
    pub name: String,
    /// The Kubernetes namespace the service belongs to.
    pub namespace: String,
    /// The ClusterIP assigned to the service.
    pub cluster_ip: IpAddr,
    /// The type of the service (e.g., ClusterIP, NodePort).
    pub service_type: ServiceType,
    /// The list of ports exposed by the service.
    pub ports: Vec<ServicePort>,
    /// Label selector used to identify backend pods.
    pub selector: HashMap<String, String>,
    /// The current list of backend endpoints.
    pub endpoints: Vec<Endpoint>,
    /// The timestamp when the service was created.
    pub created_at: DateTime<Utc>,
}

/// The type of Kubernetes service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceType {
    /// Internal service accessible only within the cluster.
    ClusterIP,
    /// Service accessible via a port on each node.
    NodePort,
    /// Service exposed via an external load balancer.
    LoadBalancer,
    /// Service that resolves to an external DNS name.
    ExternalName,
}

/// A port configuration for a service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServicePort {
    /// Optional name for the port.
    pub name: Option<String>,
    /// The port number on the service.
    pub port: u16,
    /// The port number on the target pod.
    pub target_port: u16,
    /// The network protocol (TCP or UDP).
    pub protocol: Protocol,
    /// The node port if the service type is NodePort.
    pub node_port: Option<u16>,
}

/// The network protocol used by a port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Protocol {
    /// Transmission Control Protocol.
    TCP,
    /// User Datagram Protocol.
    UDP,
}

/// Backend endpoint for a service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Endpoint {
    /// The IP address of the backend.
    pub ip: IpAddr,
    /// The port number of the backend.
    pub port: u16,
    /// The name of the pod hosting this endpoint.
    pub pod_name: String,
    /// Whether the endpoint is ready to receive traffic.
    pub ready: bool,
}

/// Network policy — controls ingress/egress traffic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkPolicy {
    /// The name of the network policy.
    pub name: String,
    /// The Kubernetes namespace the policy applies to.
    pub namespace: String,
    /// Selector identifying the pods this policy applies to.
    pub pod_selector: HashMap<String, String>,
    /// The types of traffic controlled by this policy.
    pub policy_types: Vec<PolicyType>,
    /// Rules governing incoming traffic.
    pub ingress_rules: Vec<IngressRule>,
    /// Rules governing outgoing traffic.
    pub egress_rules: Vec<EgressRule>,
    /// The timestamp when the policy was created.
    pub created_at: DateTime<Utc>,
}

/// The type of network policy rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyType {
    /// Policy controls ingress traffic.
    Ingress,
    /// Policy controls egress traffic.
    Egress,
}

/// An ingress rule defining allowed incoming traffic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngressRule {
    /// List of peers allowed to send traffic.
    pub from: Vec<PeerSelector>,
    /// List of ports allowed for ingress.
    pub ports: Vec<PolicyPort>,
}

/// An egress rule defining allowed outgoing traffic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EgressRule {
    /// List of peers allowed to receive traffic.
    pub to: Vec<PeerSelector>,
    /// List of ports allowed for egress.
    pub ports: Vec<PolicyPort>,
}

/// Selector for identifying peers in network policies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerSelector {
    /// Optional selector for matching pods.
    pub pod_selector: Option<HashMap<String, String>>,
    /// Optional selector for matching namespaces.
    pub namespace_selector: Option<HashMap<String, String>>,
    /// Optional IP block for CIDR-based matching.
    pub ip_block: Option<IpBlock>,
}

/// An IP block defined by CIDR notation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpBlock {
    /// The CIDR range (e.g., "10.0.0.0/24").
    pub cidr: String,
    /// List of IP ranges to exclude from the CIDR.
    pub except: Vec<String>,
}

/// A port definition for network policy rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyPort {
    /// The port number.
    pub port: u16,
    /// The protocol (TCP or UDP).
    pub protocol: Protocol,
}

/// CIDR allocation for pod IPs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CidrAllocation {
    /// The CIDR block allocated.
    pub cidr: String,
    /// The gateway IP for the subnet.
    pub gateway: IpAddr,
    /// List of allocated IP addresses and their associated pod names.
    pub allocated: Vec<(IpAddr, String)>,
    /// The number of available IP addresses in the block.
    pub available: u32,
}

/// Network flow record (Hubble-style).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowRecord {
    /// Unique identifier for the flow.
    pub id: Uuid,
    /// Timestamp when the flow was observed.
    pub timestamp: DateTime<Utc>,
    /// Source IP address.
    pub source_ip: IpAddr,
    /// Optional source pod name.
    pub source_pod: Option<String>,
    /// Destination IP address.
    pub destination_ip: IpAddr,
    /// Optional destination pod name.
    pub destination_pod: Option<String>,
    /// Destination port number.
    pub destination_port: u16,
    /// Network protocol of the flow.
    pub protocol: Protocol,
    /// Verdict on the flow (Allowed, Denied, Dropped).
    pub verdict: FlowVerdict,
    /// Number of bytes in the flow.
    pub bytes: u64,
    /// Direction of the flow.
    pub direction: FlowDirection,
}

/// Verdict for a network flow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlowVerdict {
    /// Traffic was allowed.
    Allowed,
    /// Traffic was denied by policy.
    Denied,
    /// Traffic was dropped (e.g., by kube-proxy or CNI).
    Dropped,
}

/// Direction of a network flow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FlowDirection {
    /// Traffic entering the node/pod.
    Ingress,
    /// Traffic leaving the node/pod.
    Egress,
    /// Traffic forwarded between nodes/pods.
    Forward,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_pod_network() {
        let pn = PodNetwork {
            pod_name: "nginx".into(),
            namespace: "default".into(),
            pod_ip: IpAddr::V4(Ipv4Addr::new(10, 0, 1, 5)),
            node_name: "node1".into(),
            labels: HashMap::new(),
            created_at: Utc::now(),
        };
        assert_eq!(pn.pod_ip.to_string(), "10.0.1.5");
    }

    #[test]
    fn test_service_entry() {
        let svc = ServiceEntry {
            name: "api".into(),
            namespace: "default".into(),
            cluster_ip: IpAddr::V4(Ipv4Addr::new(10, 96, 0, 100)),
            service_type: ServiceType::ClusterIP,
            ports: vec![ServicePort { name: Some("http".into()), port: 80, target_port: 8080, protocol: Protocol::TCP, node_port: None }],
            selector: HashMap::from([("app".into(), "api".into())]),
            endpoints: vec![],
            created_at: Utc::now(),
        };
        assert_eq!(svc.service_type, ServiceType::ClusterIP);
    }

    #[test]
    fn test_network_policy_default_deny() {
        let policy = NetworkPolicy {
            name: "default-deny".into(),
            namespace: "prod".into(),
            pod_selector: HashMap::new(),
            policy_types: vec![PolicyType::Ingress, PolicyType::Egress],
            ingress_rules: vec![],
            egress_rules: vec![],
            created_at: Utc::now(),
        };
        assert!(policy.ingress_rules.is_empty());
        assert!(policy.egress_rules.is_empty());
    }

    #[test]
    fn test_flow_record() {
        let flow = FlowRecord {
            id: Uuid::new_v4(),
            timestamp: Utc::now(),
            source_ip: IpAddr::V4(Ipv4Addr::new(10, 0, 1, 5)),
            source_pod: Some("nginx".into()),
            destination_ip: IpAddr::V4(Ipv4Addr::new(10, 0, 2, 10)),
            destination_pod: Some("api".into()),
            destination_port: 8080,
            protocol: Protocol::TCP,
            verdict: FlowVerdict::Allowed,
            bytes: 1500,
            direction: FlowDirection::Forward,
        };
        assert_eq!(flow.verdict, FlowVerdict::Allowed);
    }
}
