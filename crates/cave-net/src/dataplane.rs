//! Data plane — pod IP allocation, service routing, network policy enforcement.
//!
//! On Linux: uses eBPF programs for kernel-level packet processing.
//! On other platforms: simulated routing tables for development.

use crate::models::*;
use chrono::Utc;
use dashmap::DashMap;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicU32, Ordering};
use uuid::Uuid;

/// Network data plane state.
pub struct NetState {
    /// Pod IP allocations.
    pub pods: DashMap<String, PodNetwork>,
    /// Service registry (ClusterIP routing).
    pub services: DashMap<String, ServiceEntry>,
    /// Network policies.
    pub policies: DashMap<String, NetworkPolicy>,
    /// Flow records (recent, ring buffer style).
    pub flows: DashMap<Uuid, FlowRecord>,
    /// Next pod IP counter (within 10.0.0.0/16 CIDR).
    ip_counter: AtomicU32,
    /// Pod CIDR.
    pub pod_cidr: String,
    /// Service CIDR.
    pub service_cidr: String,
}

impl NetState {
    pub fn new() -> Self {
        Self {
            pods: DashMap::new(),
            services: DashMap::new(),
            policies: DashMap::new(),
            flows: DashMap::new(),
            ip_counter: AtomicU32::new(1),
            pod_cidr: "10.0.0.0/16".into(),
            service_cidr: "10.96.0.0/12".into(),
        }
    }

    /// Allocate a pod IP from the CIDR.
    pub fn allocate_pod_ip(&self, pod_name: &str, namespace: &str, node_name: &str, labels: HashMap<String, String>) -> PodNetwork {
        let counter = self.ip_counter.fetch_add(1, Ordering::SeqCst);
        let third = (counter / 256) as u8;
        let fourth = (counter % 256) as u8;
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, third, fourth));

        let pn = PodNetwork {
            pod_name: pod_name.to_string(),
            namespace: namespace.to_string(),
            pod_ip: ip,
            node_name: node_name.to_string(),
            labels,
            created_at: Utc::now(),
        };

        let key = format!("{}/{}", namespace, pod_name);
        self.pods.insert(key, pn.clone());
        tracing::info!(pod = %pod_name, ip = %ip, "pod IP allocated");
        pn
    }

    /// Release a pod IP.
    pub fn release_pod_ip(&self, pod_name: &str, namespace: &str) {
        let key = format!("{}/{}", namespace, pod_name);
        self.pods.remove(&key);
    }

    /// Register a service with ClusterIP.
    pub fn register_service(&self, svc: ServiceEntry) {
        let key = format!("{}/{}", svc.namespace, svc.name);
        self.services.insert(key, svc);
    }

    /// Remove a service.
    pub fn remove_service(&self, name: &str, namespace: &str) {
        let key = format!("{}/{}", namespace, name);
        self.services.remove(&key);
    }

    /// Update service endpoints (when pods change).
    pub fn update_endpoints(&self, svc_name: &str, namespace: &str, endpoints: Vec<Endpoint>) {
        let key = format!("{}/{}", namespace, svc_name);
        if let Some(mut svc) = self.services.get_mut(&key) {
            svc.endpoints = endpoints;
        }
    }

    /// Apply a network policy.
    pub fn apply_policy(&self, policy: NetworkPolicy) {
        let key = format!("{}/{}", policy.namespace, policy.name);
        self.policies.insert(key, policy);
    }

    /// Remove a network policy.
    pub fn remove_policy(&self, name: &str, namespace: &str) {
        let key = format!("{}/{}", namespace, name);
        self.policies.remove(&key);
    }

    /// Check if traffic is allowed by network policies.
    pub fn check_policy(&self, _src_pod: &str, _src_ns: &str, dst_pod: &str, dst_ns: &str, _dst_port: u16) -> FlowVerdict {
        // If no policies in destination namespace, allow all (K8s default)
        let ns_policies: Vec<_> = self.policies.iter()
            .filter(|r| r.value().namespace == dst_ns)
            .map(|r| r.value().clone())
            .collect();

        if ns_policies.is_empty() {
            return FlowVerdict::Allowed;
        }

        // Check if any policy allows this traffic
        let dst_pod_labels = self.pods.get(&format!("{}/{}", dst_ns, dst_pod))
            .map(|p| p.labels.clone())
            .unwrap_or_default();

        for policy in &ns_policies {
            // Check if policy applies to destination pod
            let applies = policy.pod_selector.is_empty() ||
                policy.pod_selector.iter().all(|(k, v)| dst_pod_labels.get(k) == Some(v));

            if !applies { continue; }

            // If policy applies and has ingress rules, check them
            if policy.policy_types.contains(&PolicyType::Ingress) {
                if policy.ingress_rules.is_empty() {
                    // Empty ingress = deny all ingress
                    return FlowVerdict::Denied;
                }
                // Check if any ingress rule allows
                for rule in &policy.ingress_rules {
                    // Simplified: if rule has no from, allow from all
                    if rule.from.is_empty() {
                        return FlowVerdict::Allowed;
                    }
                }
            }
        }

        FlowVerdict::Allowed
    }

    /// Record a flow.
    pub fn record_flow(&self, flow: FlowRecord) {
        // Keep last 10000 flows
        if self.flows.len() > 10000 {
            if let Some(oldest) = self.flows.iter().next().map(|r| *r.key()) {
                self.flows.remove(&oldest);
            }
        }
        self.flows.insert(flow.id, flow);
    }
}

impl Default for NetState {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allocate_pod_ip() {
        let state = NetState::new();
        let pn = state.allocate_pod_ip("nginx", "default", "node1", HashMap::new());
        assert_eq!(pn.pod_ip.to_string(), "10.0.0.1");

        let pn2 = state.allocate_pod_ip("api", "default", "node1", HashMap::new());
        assert_eq!(pn2.pod_ip.to_string(), "10.0.0.2");
    }

    #[test]
    fn test_release_pod_ip() {
        let state = NetState::new();
        state.allocate_pod_ip("temp", "ns", "node1", HashMap::new());
        assert_eq!(state.pods.len(), 1);
        state.release_pod_ip("temp", "ns");
        assert_eq!(state.pods.len(), 0);
    }

    #[test]
    fn test_service_registration() {
        let state = NetState::new();
        state.register_service(ServiceEntry {
            name: "api".into(), namespace: "default".into(),
            cluster_ip: IpAddr::V4(Ipv4Addr::new(10, 96, 0, 1)),
            service_type: ServiceType::ClusterIP,
            ports: vec![], selector: HashMap::new(), endpoints: vec![],
            created_at: Utc::now(),
        });
        assert_eq!(state.services.len(), 1);
    }

    #[test]
    fn test_default_allow_no_policies() {
        let state = NetState::new();
        let verdict = state.check_policy("src", "ns1", "dst", "ns2", 80);
        assert_eq!(verdict, FlowVerdict::Allowed);
    }

    #[test]
    fn test_default_deny_with_empty_ingress() {
        let state = NetState::new();
        state.allocate_pod_ip("dst", "prod", "node1", HashMap::new());
        state.apply_policy(NetworkPolicy {
            name: "deny-all".into(), namespace: "prod".into(),
            pod_selector: HashMap::new(),
            policy_types: vec![PolicyType::Ingress],
            ingress_rules: vec![],
            egress_rules: vec![],
            created_at: Utc::now(),
        });
        let verdict = state.check_policy("src", "other", "dst", "prod", 80);
        assert_eq!(verdict, FlowVerdict::Denied);
    }

    #[test]
    fn test_flow_recording() {
        let state = NetState::new();
        state.record_flow(FlowRecord {
            id: Uuid::new_v4(), timestamp: Utc::now(),
            source_ip: IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            source_pod: Some("src".into()),
            destination_ip: IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)),
            destination_pod: Some("dst".into()),
            destination_port: 80, protocol: Protocol::TCP,
            verdict: FlowVerdict::Allowed, bytes: 1500,
            direction: FlowDirection::Forward,
        });
        assert_eq!(state.flows.len(), 1);
    }
}
