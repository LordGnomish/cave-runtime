//! Networking models — pods, services, endpoints, network policies.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;
use uuid::Uuid;

/// Pod network identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodNetwork {
    pub pod_name: String,
    pub namespace: String,
    pub pod_ip: IpAddr,
    pub node_name: String,
    pub labels: HashMap<String, String>,
    pub created_at: DateTime<Utc>,
}

/// Service (ClusterIP, NodePort, LoadBalancer).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceEntry {
    pub name: String,
    pub namespace: String,
    pub cluster_ip: IpAddr,
    pub service_type: ServiceType,
    pub ports: Vec<ServicePort>,
    pub selector: HashMap<String, String>,
    pub endpoints: Vec<Endpoint>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceType {
    ClusterIP,
    NodePort,
    LoadBalancer,
    ExternalName,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServicePort {
    pub name: Option<String>,
    pub port: u16,
    pub target_port: u16,
    pub protocol: Protocol,
    pub node_port: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Protocol {
    TCP,
    UDP,
}

/// Backend endpoint for a service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Endpoint {
    pub ip: IpAddr,
    pub port: u16,
    pub pod_name: String,
    pub ready: bool,
}

/// Network policy — controls ingress/egress traffic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkPolicy {
    pub name: String,
    pub namespace: String,
    pub pod_selector: HashMap<String, String>,
    pub policy_types: Vec<PolicyType>,
    pub ingress_rules: Vec<IngressRule>,
    pub egress_rules: Vec<EgressRule>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyType {
    Ingress,
    Egress,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngressRule {
    pub from: Vec<PeerSelector>,
    pub ports: Vec<PolicyPort>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EgressRule {
    pub to: Vec<PeerSelector>,
    pub ports: Vec<PolicyPort>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerSelector {
    pub pod_selector: Option<HashMap<String, String>>,
    pub namespace_selector: Option<HashMap<String, String>>,
    pub ip_block: Option<IpBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpBlock {
    pub cidr: String,
    pub except: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyPort {
    pub port: u16,
    pub protocol: Protocol,
}

/// CIDR allocation for pod IPs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CidrAllocation {
    pub cidr: String,
    pub gateway: IpAddr,
    pub allocated: Vec<(IpAddr, String)>,
    pub available: u32,
}

/// Network flow record (Hubble-style).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowRecord {
    pub id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub source_ip: IpAddr,
    pub source_pod: Option<String>,
    pub destination_ip: IpAddr,
    pub destination_pod: Option<String>,
    pub destination_port: u16,
    pub protocol: Protocol,
    pub verdict: FlowVerdict,
    pub bytes: u64,
    pub direction: FlowDirection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlowVerdict {
    Allowed,
    Denied,
    Dropped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FlowDirection {
    Ingress,
    Egress,
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
