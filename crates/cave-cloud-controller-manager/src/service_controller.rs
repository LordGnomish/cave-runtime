//! Cloud service controller — drives `Service.spec.type == LoadBalancer`
//! through a per-cloud `LoadBalancer` provider.
//!
//! Mirrors `staging/src/k8s.io/cloud-provider/controllers/service/controller.go`.
//! The in-tree `service` controller in `cave-controller-manager` decides what
//! to do; this controller is the per-cloud executor that asks the provider
//! to actually allocate or release the external IP.

use crate::provider::LoadBalancerIface;
use crate::types::{Cite, CloudError, Reconcile, TenantId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceObservation {
    pub service_name: String,
    pub namespace: String,
    pub external_ip: Option<String>,
    pub deletion_pending: bool,
}

/// Mirrors `syncLoadBalancerIfNeeded` upstream.
pub fn reconcile<P: LoadBalancerIface>(
    provider: &P,
    obs: &ServiceObservation,
    tenant: &TenantId,
) -> Result<Reconcile, CloudError> {
    provider.authorise(tenant, "Service", &obs.service_name)?;
    if obs.deletion_pending {
        if obs.external_ip.is_some() {
            provider.delete_lb(tenant, &obs.service_name)?;
            return Ok(Reconcile::Delete(1));
        }
        return Ok(Reconcile::NoOp);
    }
    if obs.external_ip.is_none() {
        let _ip = provider.ensure_lb(tenant, &obs.service_name)?;
        return Ok(Reconcile::AllocateIp(1));
    }
    Ok(Reconcile::NoOp)
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::k8s(
    "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
    "Controller",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{CloudConfig, CloudProvider};
    use crate::types::{ProviderName, TenantId};
    use crate::test_ctx;
    use std::cell::Cell;

    /// Minimal in-memory LB provider for unit tests.
    struct StubLb {
        cfg: CloudConfig,
        ensured: Cell<u32>,
        deleted: Cell<u32>,
    }
    impl StubLb {
        fn new(tenant: &str) -> Self {
            Self {
                cfg: CloudConfig {
                    tenant: TenantId::new(tenant),
                    provider: ProviderName::Hetzner,
                    region: "fsn1".into(),
                    credential_ref: "vault://kv/hcloud".into(),
                },
                ensured: Cell::new(0),
                deleted: Cell::new(0),
            }
        }
    }
    impl CloudProvider for StubLb {
        fn name(&self) -> ProviderName {
            self.cfg.provider
        }
        fn config(&self) -> &CloudConfig {
            &self.cfg
        }
    }
    impl LoadBalancerIface for StubLb {
        fn ensure_lb(&self, _t: &TenantId, _svc: &str) -> Result<String, CloudError> {
            self.ensured.set(self.ensured.get() + 1);
            Ok("203.0.113.7".into())
        }
        fn delete_lb(&self, _t: &TenantId, _svc: &str) -> Result<(), CloudError> {
            self.deleted.set(self.deleted.get() + 1);
            Ok(())
        }
    }

    fn obs(name: &str, ip: Option<&str>, deletion: bool) -> ServiceObservation {
        ServiceObservation {
            service_name: name.into(),
            namespace: "default".into(),
            external_ip: ip.map(|s| s.to_string()),
            deletion_pending: deletion,
        }
    }

    #[test]
    fn missing_external_ip_calls_ensure_lb() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "syncLoadBalancerIfNeeded",
            "acme"
        );
        let p = StubLb::new("acme");
        let r = reconcile(&p, &obs("web", None, false), &tenant).unwrap();
        assert_eq!(r, Reconcile::AllocateIp(1));
        assert_eq!(p.ensured.get(), 1);
    }

    #[test]
    fn pre_allocated_ip_is_a_no_op() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "syncLoadBalancerIfNeeded",
            "acme"
        );
        let p = StubLb::new("acme");
        let r = reconcile(&p, &obs("web", Some("203.0.113.1"), false), &tenant).unwrap();
        assert_eq!(r, Reconcile::NoOp);
        assert_eq!(p.ensured.get(), 0);
    }

    #[test]
    fn deletion_with_ip_calls_delete_lb() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "ensureLoadBalancerDeleted",
            "acme"
        );
        let p = StubLb::new("acme");
        let r = reconcile(&p, &obs("web", Some("203.0.113.1"), true), &tenant).unwrap();
        assert_eq!(r, Reconcile::Delete(1));
        assert_eq!(p.deleted.get(), 1);
    }

    #[test]
    fn deletion_without_ip_is_a_no_op() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "ensureLoadBalancerDeleted",
            "acme"
        );
        let p = StubLb::new("acme");
        let r = reconcile(&p, &obs("web", None, true), &tenant).unwrap();
        assert_eq!(r, Reconcile::NoOp);
        assert_eq!(p.deleted.get(), 0);
    }

    #[test]
    fn cross_tenant_call_is_refused_before_provider_is_invoked() {
        let (_cite, attacker) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "Controller",
            "tenant-attacker"
        );
        let p = StubLb::new("acme");
        let err = reconcile(&p, &obs("web", None, false), &attacker).unwrap_err();
        assert!(matches!(err, CloudError::TenantDenied { .. }));
        assert_eq!(p.ensured.get(), 0);
    }
}
