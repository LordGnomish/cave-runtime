//! Cloud controllers — node, service, route reconcile loops.
//!
//! Each controller corresponds to an upstream
//! `staging/src/k8s.io/cloud-provider/controllers/<kind>/` package.

use crate::models::*;
use crate::state::CloudControllerState;

// ── NodeController ───────────────────────────────────────────────────────────
// Mirrors staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go.
//
// Periodically calls the cloud provider to synchronize node addresses,
// labels, and termination state.

/// Reconcile a single node against the cloud provider's view.
/// Returns true if state changed.
pub fn reconcile_node(state: &CloudControllerState, name: &str) -> bool {
    let cloud_view = match state.provider.describe_node(name) {
        Some(n) => n,
        None => {
            // Provider says node is gone — mark Terminated locally.
            if let Some(mut n) = state.nodes.get_mut(name) {
                if n.status != CloudNodeStatus::Terminated {
                    n.status = CloudNodeStatus::Terminated;
                    return true;
                }
            }
            return false;
        }
    };
    state.nodes.insert(name.into(), cloud_view);
    true
}

// ── ServiceController ────────────────────────────────────────────────────────
// Mirrors staging/src/k8s.io/cloud-provider/controllers/service/controller.go.
//
// For each Service of type=LoadBalancer, ensures the cloud provider has
// the right LB provisioned for the current set of backend pod IPs.

pub fn reconcile_service_load_balancer(
    state: &CloudControllerState,
    namespace: &str,
    service_name: &str,
    backends: Vec<String>,
) -> LoadBalancerStatus {
    let status = state.provider.ensure_load_balancer(service_name, namespace, &backends);
    let lb = LoadBalancer {
        service_name: service_name.into(),
        namespace: namespace.into(),
        provider_lb_id: Some(format!("lb-{}-{}", namespace, service_name)),
        external_ip: if matches!(status, LoadBalancerStatus::Provisioned) {
            Some(format!("203.0.113.{}", namespace.len() + service_name.len()))
        } else {
            None
        },
        backends,
        status: status.clone(),
    };
    state.load_balancers.insert((namespace.into(), service_name.into()), lb);
    status
}

pub fn delete_service_load_balancer(state: &CloudControllerState, namespace: &str, service_name: &str) -> bool {
    state.provider.delete_load_balancer(service_name, namespace);
    state.load_balancers.remove(&(namespace.into(), service_name.into())).is_some()
}

// ── RouteController ──────────────────────────────────────────────────────────
// Mirrors staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go.
//
// Synchronizes pod-CIDR → node routes in the cloud provider's route table.

pub fn reconcile_route(state: &CloudControllerState, route: CloudRoute) -> bool {
    let ok = state.provider.create_route(&route);
    if ok {
        state.routes.insert(route.name.clone(), route);
    }
    ok
}

pub fn remove_route(state: &CloudControllerState, name: &str) -> bool {
    let _ = state.provider.delete_route(name);
    state.routes.remove(name).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::MockProvider;
    use chrono::Utc;
    use std::sync::Arc;

    fn fresh_state() -> CloudControllerState {
        CloudControllerState::with_provider(Arc::new(MockProvider::new()))
    }

    #[test]
    fn test_reconcile_node_known_to_provider_is_synced() {
        // Mirrors TestNodeControllerSync.
        let mock = Arc::new(MockProvider::new());
        mock.seed_node(CloudNode {
            name: "n1".into(), provider_id: "p:///n1".into(),
            region: "us-east-1".into(), zone: "us-east-1a".into(),
            instance_type: "t3".into(), addresses: vec![],
            status: CloudNodeStatus::Ready, created_at: Utc::now(),
        });
        let state = CloudControllerState::with_provider(mock);
        assert!(reconcile_node(&state, "n1"));
        assert_eq!(state.nodes.get("n1").unwrap().status, CloudNodeStatus::Ready);
    }

    #[test]
    fn test_reconcile_node_missing_in_provider_marks_terminated() {
        // Mirrors TestNodeControllerHandlesGone.
        let state = fresh_state();
        state.nodes.insert("gone".into(), CloudNode {
            name: "gone".into(), provider_id: "p:///gone".into(),
            region: "x".into(), zone: "y".into(), instance_type: "t".into(),
            addresses: vec![], status: CloudNodeStatus::Ready,
            created_at: Utc::now(),
        });
        assert!(reconcile_node(&state, "gone"));
        assert_eq!(state.nodes.get("gone").unwrap().status, CloudNodeStatus::Terminated);
    }

    #[test]
    fn test_service_load_balancer_with_backends_is_provisioned() {
        // Mirrors TestServiceControllerEnsureLB.
        let state = fresh_state();
        let r = reconcile_service_load_balancer(&state, "default", "web", vec!["10.0.0.1".into()]);
        assert_eq!(r, LoadBalancerStatus::Provisioned);
        let lb = state.load_balancers.get(&("default".into(), "web".into())).unwrap();
        assert!(lb.external_ip.is_some());
    }

    #[test]
    fn test_service_load_balancer_no_backends_is_pending() {
        let state = fresh_state();
        let r = reconcile_service_load_balancer(&state, "default", "web", vec![]);
        assert_eq!(r, LoadBalancerStatus::Pending);
        assert!(state.load_balancers.get(&("default".into(), "web".into())).unwrap().external_ip.is_none());
    }

    #[test]
    fn test_delete_service_load_balancer_removes_state() {
        let state = fresh_state();
        reconcile_service_load_balancer(&state, "default", "web", vec!["10.0.0.1".into()]);
        assert!(delete_service_load_balancer(&state, "default", "web"));
        assert!(state.load_balancers.get(&("default".into(), "web".into())).is_none());
    }

    #[test]
    fn test_delete_service_load_balancer_unknown_returns_false() {
        let state = fresh_state();
        assert!(!delete_service_load_balancer(&state, "default", "ghost"));
    }

    #[test]
    fn test_reconcile_route_creates_in_provider_and_state() {
        // Mirrors TestRouteControllerCreate.
        let state = fresh_state();
        let route = CloudRoute {
            name: "pod-net-1".into(), destination_cidr: "10.244.0.0/24".into(),
            next_hop_node: "n1".into(), provider_route_id: None,
        };
        assert!(reconcile_route(&state, route.clone()));
        assert!(state.routes.contains_key("pod-net-1"));
    }

    #[test]
    fn test_remove_route_clears_state() {
        let state = fresh_state();
        reconcile_route(&state, CloudRoute {
            name: "r".into(), destination_cidr: "10.0.0.0/16".into(),
            next_hop_node: "n".into(), provider_route_id: None,
        });
        assert!(remove_route(&state, "r"));
        assert!(!state.routes.contains_key("r"));
    }
}
