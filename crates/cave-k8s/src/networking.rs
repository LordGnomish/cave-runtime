// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Networking facade — Service, Endpoint, EndpointSlice, NetworkPolicy.
//!
//! Bridges `cave-apiserver` (resource definitions) and
//! `cave-kube-proxy` (datapath synthesis).  Provides EndpointSlice
//! derivation from a Pod selector and exposes the topology / sessionAffinity
//! knobs that the umbrella tracks at the control-plane layer.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceType {
    ClusterIP,
    NodePort,
    LoadBalancer,
    ExternalName,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IpFamily {
    Ipv4,
    Ipv6,
    DualStack,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServicePort {
    pub name: String,
    pub port: u16,
    pub target_port: u16,
    pub node_port: Option<u16>,
    /// TCP / UDP / SCTP.
    pub protocol: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceSpec {
    pub kind: ServiceType,
    pub selector: std::collections::BTreeMap<String, String>,
    pub ports: Vec<ServicePort>,
    pub cluster_ip: String,
    pub ip_family: IpFamily,
    pub session_affinity: SessionAffinity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionAffinity {
    None,
    ClientIP,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    pub addresses: Vec<String>,
    pub node_name: Option<String>,
    pub ready: bool,
    pub serving: bool,
    pub terminating: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointSlice {
    pub name: String,
    pub namespace: String,
    pub service: String,
    pub address_type: IpFamily,
    pub endpoints: Vec<Endpoint>,
    pub ports: Vec<ServicePort>,
}

impl EndpointSlice {
    pub fn ready_count(&self) -> usize {
        self.endpoints.iter().filter(|e| e.ready).count()
    }
    pub fn serving_count(&self) -> usize {
        self.endpoints.iter().filter(|e| e.serving).count()
    }
}

/// Derive EndpointSlice list from a Service selector + Pod set.  Caller
/// passes the matching Pods as `(name, node, ip, ready)` tuples.  Mirrors
/// the algorithm in `pkg/controller/endpointslice`.
pub fn derive_slices(
    namespace: &str,
    service: &str,
    family: IpFamily,
    ports: Vec<ServicePort>,
    pods: &[(String, String, String, bool)],
    max_per_slice: usize,
) -> Vec<EndpointSlice> {
    let max_per_slice = max_per_slice.max(1);
    pods.chunks(max_per_slice)
        .enumerate()
        .map(|(i, chunk)| {
            let endpoints = chunk
                .iter()
                .map(|(_name, node, ip, ready)| Endpoint {
                    addresses: vec![ip.clone()],
                    node_name: Some(node.clone()),
                    ready: *ready,
                    serving: *ready,
                    terminating: false,
                })
                .collect();
            EndpointSlice {
                name: format!("{}-{}", service, i),
                namespace: namespace.to_string(),
                service: service.to_string(),
                address_type: family,
                endpoints,
                ports: ports.clone(),
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkPolicyDirection {
    Ingress,
    Egress,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkPolicyRule {
    pub direction: NetworkPolicyDirection,
    pub from_pod_selector: Option<std::collections::BTreeMap<String, String>>,
    pub ports: Vec<u16>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_count_excludes_unready() {
        let s = EndpointSlice {
            name: "x".into(),
            namespace: "default".into(),
            service: "x".into(),
            address_type: IpFamily::Ipv4,
            endpoints: vec![
                Endpoint {
                    addresses: vec!["10.0.0.1".into()],
                    node_name: Some("n1".into()),
                    ready: true,
                    serving: true,
                    terminating: false,
                },
                Endpoint {
                    addresses: vec!["10.0.0.2".into()],
                    node_name: Some("n1".into()),
                    ready: false,
                    serving: false,
                    terminating: false,
                },
            ],
            ports: vec![],
        };
        assert_eq!(s.ready_count(), 1);
        assert_eq!(s.serving_count(), 1);
    }

    #[test]
    fn derive_chunks_pods() {
        let pods = (0..5)
            .map(|i| (format!("p{}", i), "n1".into(), format!("10.0.0.{}", i), true))
            .collect::<Vec<_>>();
        let slices = derive_slices(
            "default",
            "svc",
            IpFamily::Ipv4,
            vec![],
            &pods,
            2,
        );
        // 5 pods, max=2 -> 3 slices (2,2,1)
        assert_eq!(slices.len(), 3);
        assert_eq!(slices[0].endpoints.len(), 2);
        assert_eq!(slices[2].endpoints.len(), 1);
    }

    #[test]
    fn derive_handles_empty_pods() {
        let slices = derive_slices(
            "default",
            "svc",
            IpFamily::Ipv4,
            vec![],
            &[],
            10,
        );
        assert!(slices.is_empty());
    }

    #[test]
    fn service_type_roundtrip() {
        for t in [
            ServiceType::ClusterIP,
            ServiceType::NodePort,
            ServiceType::LoadBalancer,
            ServiceType::ExternalName,
        ] {
            let s = serde_json::to_string(&t).unwrap();
            let back: ServiceType = serde_json::from_str(&s).unwrap();
            assert_eq!(back, t);
        }
    }

    #[test]
    fn dualstack_family_distinguishable() {
        assert_ne!(IpFamily::Ipv4, IpFamily::DualStack);
        assert_ne!(IpFamily::Ipv6, IpFamily::DualStack);
    }

    #[test]
    fn npolicy_rule_supports_egress() {
        let r = NetworkPolicyRule {
            direction: NetworkPolicyDirection::Egress,
            from_pod_selector: None,
            ports: vec![53],
        };
        assert_eq!(r.direction, NetworkPolicyDirection::Egress);
    }
}
