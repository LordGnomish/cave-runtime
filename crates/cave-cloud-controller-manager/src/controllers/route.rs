//! Cloud route controller — programs VPC routes for Pod CIDRs across nodes.
//!
//! Upstream: staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go::ReconcileRoutes

use crate::models::{CloudNode, CloudRoute};

/// Reconcile cloud routes — emit one CloudRoute per node with a non-empty
/// `pod_cidr`. Returns the canonical route table the controller would program.
pub fn reconcile_routes(nodes: &[CloudNode]) -> Vec<CloudRoute> {
    nodes
        .iter()
        .filter_map(|n| n.pod_cidr.as_ref().map(|cidr| CloudRoute {
            destination_cidr: cidr.clone(),
            target_node: n.name.clone(),
        }))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, cidr: Option<&str>) -> CloudNode {
        let mut n = CloudNode::new(name);
        n.pod_cidr = cidr.map(String::from);
        n
    }

    /// Port of `TestRouteControllerReconcile` — a route is emitted for each
    /// node that has a Pod CIDR; nodes without one are skipped.
    #[test]
    fn test_route_controller_reconcile() {
        let nodes = [
            node("a", Some("10.244.0.0/24")),
            node("b", Some("10.244.1.0/24")),
            node("c", None),
        ];
        let routes = reconcile_routes(&nodes);
        assert_eq!(routes.len(), 2);
        assert!(routes.contains(&CloudRoute {
            destination_cidr: "10.244.0.0/24".into(),
            target_node: "a".into(),
        }));
        assert!(routes.contains(&CloudRoute {
            destination_cidr: "10.244.1.0/24".into(),
            target_node: "b".into(),
        }));
    }
}
