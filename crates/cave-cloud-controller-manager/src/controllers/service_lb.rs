//! Service LoadBalancer controller — provisions / tears down cloud LBs for
//! Services of type LoadBalancer.
//!
//! Upstream: staging/src/k8s.io/cloud-provider/controllers/service/controller.go::EnsureLoadBalancer

use crate::models::LbService;

/// Outcome of one reconcile pass over a Service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LbAction {
    /// Provision an LB and surface its IP back as `external_ip`.
    Ensure { external_ip: String },
    /// Tear the LB down (Service no longer needs it).
    Delete,
    /// No action needed (already in steady state, or not type=LoadBalancer).
    NoOp,
}

/// Reconcile a Service against the cloud LB. Returns the action the
/// controller would take. The caller is responsible for actually invoking the
/// cloud API and persisting the result.
pub fn reconcile_service_lb(svc: &LbService) -> LbAction {
    if svc.service_type != "LoadBalancer" {
        return if svc.external_ip.is_some() {
            // Service downgraded from LB → tear the LB down.
            LbAction::Delete
        } else {
            LbAction::NoOp
        };
    }
    match svc.external_ip {
        None => LbAction::Ensure {
            external_ip: synth_lb_ip(&svc.namespace, &svc.name),
        },
        Some(_) => LbAction::NoOp,
    }
}

fn synth_lb_ip(ns: &str, name: &str) -> String {
    // Deterministic placeholder — real provider implementations call the cloud
    // API and surface its allocation.
    format!("lb://{}/{}", ns, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svc(t: &str, external: Option<&str>) -> LbService {
        LbService {
            name: "api".into(),
            namespace: "default".into(),
            service_type: t.into(),
            external_ip: external.map(String::from),
        }
    }

    /// Port of `TestServiceLoadBalancerEnsure` — a fresh LB Service receives
    /// an external_ip allocation; an already-allocated one is left as-is.
    #[test]
    fn test_service_load_balancer_ensure() {
        let fresh = svc("LoadBalancer", None);
        let action = reconcile_service_lb(&fresh);
        assert!(matches!(action, LbAction::Ensure { .. }));
        let already = svc("LoadBalancer", Some("203.0.113.10"));
        assert_eq!(reconcile_service_lb(&already), LbAction::NoOp);
    }

    /// Port of `TestServiceLoadBalancerDelete` — when a Service that previously
    /// held an external_ip is no longer type=LoadBalancer, the LB is deleted.
    #[test]
    fn test_service_load_balancer_delete() {
        let demoted = svc("ClusterIP", Some("203.0.113.20"));
        assert_eq!(reconcile_service_lb(&demoted), LbAction::Delete);
        let plain = svc("ClusterIP", None);
        assert_eq!(reconcile_service_lb(&plain), LbAction::NoOp);
    }
}
