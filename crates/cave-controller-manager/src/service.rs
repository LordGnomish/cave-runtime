//! Service controller — provisions external load balancers for
//! `Service.spec.type == "LoadBalancer"` and finalises cleanup on delete.
//!
//! Upstream: [`pkg/controller/service`]. The full controller drives a
//! per-cloud `cloudprovider.LoadBalancer` interface; this scaffold isolates
//! the type-dispatch and finaliser logic.

use crate::types::{Cite, ControllerError, Reconcile, TenantId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceType {
    ClusterIP,
    NodePort,
    LoadBalancer,
    ExternalName,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceSpec {
    pub name: String,
    pub namespace: String,
    pub service_type: ServiceType,
    pub deletion_pending: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServiceStatus {
    pub lb_provisioned: bool,
    pub finalizer_present: bool,
}

/// Returns true iff the service type warrants this controller acting at all.
/// Mirrors `wantsLoadBalancer` in `pkg/controller/service/controller.go`.
pub fn wants_load_balancer(spec: &ServiceSpec) -> bool {
    spec.service_type == ServiceType::LoadBalancer
}

/// Mirrors `syncService` — the controller adds/removes the cloud finaliser
/// and provisions/destroys the LB.
pub fn reconcile(
    spec: &ServiceSpec,
    status: &ServiceStatus,
    _tenant: &TenantId,
) -> Result<Reconcile, ControllerError> {
    if !wants_load_balancer(spec) {
        return Ok(Reconcile::NoOp);
    }
    if spec.deletion_pending {
        // We must tear down the LB *before* removing the finaliser.
        if status.lb_provisioned {
            return Ok(Reconcile::Delete(1));
        }
        if status.finalizer_present {
            return Ok(Reconcile::Update(0));
        }
        return Ok(Reconcile::NoOp);
    }
    if !status.lb_provisioned {
        return Ok(Reconcile::Create(1));
    }
    Ok(Reconcile::NoOp)
}

/// Stub: cloud-provider-specific LB attribute reconciliation. Not implemented.
pub fn sync_lb_attributes(_spec: &ServiceSpec) -> Result<Reconcile, ControllerError> {
    unimplemented!("cloud LB attribute sync — see cloudprovider.LoadBalancer.UpdateLoadBalancer")
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new("pkg/controller/service/controller.go", "Controller");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn svc(t: ServiceType, deletion: bool) -> ServiceSpec {
        ServiceSpec {
            name: "frontend".into(),
            namespace: "default".into(),
            service_type: t,
            deletion_pending: deletion,
        }
    }

    #[test]
    fn cluster_ip_service_is_a_no_op() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/service/controller.go",
            "wantsLoadBalancer",
            "tenant-svc-clusterip"
        );
        let s = svc(ServiceType::ClusterIP, false);
        assert!(!wants_load_balancer(&s));
        assert_eq!(reconcile(&s, &ServiceStatus::default(), &tenant).unwrap(), Reconcile::NoOp);
    }

    #[test]
    fn missing_lb_triggers_create() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/service/controller.go",
            "ensureLoadBalancer",
            "tenant-svc-create-lb"
        );
        let s = svc(ServiceType::LoadBalancer, false);
        assert_eq!(reconcile(&s, &ServiceStatus::default(), &tenant).unwrap(), Reconcile::Create(1));
    }

    #[test]
    fn deletion_destroys_lb_before_finalizer_removal() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/service/controller.go",
            "syncLoadBalancerIfNeeded",
            "tenant-svc-delete-lb"
        );
        let s = svc(ServiceType::LoadBalancer, true);
        let st = ServiceStatus { lb_provisioned: true, finalizer_present: true };
        assert_eq!(reconcile(&s, &st, &tenant).unwrap(), Reconcile::Delete(1));
    }

    #[test]
    fn finalizer_removal_emits_update_after_lb_gone() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/service/controller.go",
            "removeFinalizer",
            "tenant-svc-finalizer"
        );
        let s = svc(ServiceType::LoadBalancer, true);
        let st = ServiceStatus { lb_provisioned: false, finalizer_present: true };
        assert_eq!(reconcile(&s, &st, &tenant).unwrap(), Reconcile::Update(0));
    }
}
