//! Cloud-provider trait — abstracts AWS/GCP/Azure differences.
//!
//! Cave ships a `MockProvider` for tests. Real provider impls will live in
//! sibling crates (`cave-cloud-aws`, `cave-cloud-gcp`, etc.).

use crate::models::*;
use std::sync::Mutex;

pub trait CloudProvider: Send + Sync {
    /// Fetch live cloud-side metadata for a node.
    fn describe_node(&self, name: &str) -> Option<CloudNode>;
    /// Provision (or refresh) a load balancer for a service.
    fn ensure_load_balancer(&self, service_name: &str, namespace: &str, backends: &[String]) -> LoadBalancerStatus;
    /// Tear down a load balancer.
    fn delete_load_balancer(&self, service_name: &str, namespace: &str) -> bool;
    /// Add a cluster-managed route in the cloud route table.
    fn create_route(&self, route: &CloudRoute) -> bool;
    /// Remove a cluster-managed route.
    fn delete_route(&self, name: &str) -> bool;
}

/// Test-only provider that records calls and lets each test seed responses.
pub struct MockProvider {
    pub nodes: Mutex<Vec<CloudNode>>,
    pub created_lbs: Mutex<Vec<(String, String)>>,
    pub deleted_lbs: Mutex<Vec<(String, String)>>,
    pub created_routes: Mutex<Vec<CloudRoute>>,
    pub deleted_routes: Mutex<Vec<String>>,
}

impl MockProvider {
    pub fn new() -> Self {
        Self {
            nodes: Mutex::new(Vec::new()),
            created_lbs: Mutex::new(Vec::new()),
            deleted_lbs: Mutex::new(Vec::new()),
            created_routes: Mutex::new(Vec::new()),
            deleted_routes: Mutex::new(Vec::new()),
        }
    }

    pub fn seed_node(&self, n: CloudNode) {
        self.nodes.lock().unwrap().push(n);
    }
}

impl Default for MockProvider {
    fn default() -> Self { Self::new() }
}

impl CloudProvider for MockProvider {
    fn describe_node(&self, name: &str) -> Option<CloudNode> {
        self.nodes.lock().unwrap().iter().find(|n| n.name == name).cloned()
    }
    fn ensure_load_balancer(&self, service_name: &str, namespace: &str, backends: &[String]) -> LoadBalancerStatus {
        self.created_lbs.lock().unwrap().push((namespace.into(), service_name.into()));
        if backends.is_empty() {
            LoadBalancerStatus::Pending
        } else {
            LoadBalancerStatus::Provisioned
        }
    }
    fn delete_load_balancer(&self, service_name: &str, namespace: &str) -> bool {
        self.deleted_lbs.lock().unwrap().push((namespace.into(), service_name.into()));
        true
    }
    fn create_route(&self, route: &CloudRoute) -> bool {
        self.created_routes.lock().unwrap().push(route.clone());
        true
    }
    fn delete_route(&self, name: &str) -> bool {
        self.deleted_routes.lock().unwrap().push(name.into());
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_mock_provider_describe_node_known() {
        let p = MockProvider::new();
        p.seed_node(CloudNode {
            name: "n1".into(), provider_id: "aws:///us-east-1a/i-1".into(),
            region: "us-east-1".into(), zone: "us-east-1a".into(),
            instance_type: "t3.medium".into(), addresses: vec![],
            status: CloudNodeStatus::Ready, created_at: Utc::now(),
        });
        assert!(p.describe_node("n1").is_some());
        assert!(p.describe_node("missing").is_none());
    }

    #[test]
    fn test_mock_provider_ensure_lb_with_backends_is_provisioned() {
        let p = MockProvider::new();
        let r = p.ensure_load_balancer("web", "default", &["10.0.0.1".into()]);
        assert_eq!(r, LoadBalancerStatus::Provisioned);
    }

    #[test]
    fn test_mock_provider_ensure_lb_no_backends_is_pending() {
        let p = MockProvider::new();
        let r = p.ensure_load_balancer("web", "default", &[]);
        assert_eq!(r, LoadBalancerStatus::Pending);
    }

    #[test]
    fn test_mock_provider_route_lifecycle() {
        let p = MockProvider::new();
        let route = CloudRoute {
            name: "pod-net".into(), destination_cidr: "10.244.0.0/24".into(),
            next_hop_node: "n1".into(), provider_route_id: None,
        };
        assert!(p.create_route(&route));
        assert_eq!(p.created_routes.lock().unwrap().len(), 1);
        assert!(p.delete_route("pod-net"));
        assert_eq!(p.deleted_routes.lock().unwrap().len(), 1);
    }
}
