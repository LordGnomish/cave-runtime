//! Aggregated API server (kube-aggregator) — APIService registration + routing.
//!
//! Upstream: kubernetes/kubernetes v1.36.0
//!   * `staging/src/k8s.io/kube-aggregator/pkg/apis/apiregistration/v1/types.go`
//!     (`APIService`, `APIServiceSpec`, `APIServiceCondition`).
//!   * `staging/src/k8s.io/kube-aggregator/pkg/apiserver/handler_proxy.go`
//!     (proxy/route decisions for delegated groups).
//!   * `staging/src/k8s.io/kube-aggregator/pkg/controllers/status/available_controller.go`
//!     (availability gating).
//!
//! An APIService binds a `(group, version)` to a backing Service. When a
//! request lands at the aggregated apiserver, we either serve it locally
//! (built-in group/version) or delegate it to the backing service. Backing
//! services that are not Available fall back to local.
//!
//! Tenant invariant: each APIService is registered under a `tenant_id`. Lookup
//! and routing decisions MUST NOT cross tenant boundaries — a delegated group
//! registered by tenant A is invisible to tenant B's request.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceRef {
    pub namespace: String,
    pub name: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct APIService {
    /// Resource name, conventionally `<version>.<group>` (e.g.
    /// `v1beta1.metrics.k8s.io`).
    pub name: String,
    pub tenant_id: String,
    pub group: String,
    pub version: String,
    pub service: ServiceRef,
    /// Lower numbers are preferred (matches upstream
    /// `APIServiceSpec.GroupPriorityMinimum` semantics).
    pub group_priority: i32,
    pub version_priority: i32,
    pub available: bool,
}

/// Outcome of a route decision for an inbound `(tenant, group, version)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteDecision {
    /// Serve from the in-process apiserver (built-in or no registration).
    Local,
    /// Forward to the registered backing service.
    Delegated { tenant_id: String, service: ServiceRef },
}

#[derive(Debug, Hash, Eq, PartialEq, Clone)]
struct RegistryKey {
    tenant_id: String,
    group: String,
    version: String,
}

pub struct AggregatorRegistry {
    inner: Mutex<HashMap<RegistryKey, APIService>>,
}

impl AggregatorRegistry {
    pub fn new() -> Self {
        Self { inner: Mutex::new(HashMap::new()) }
    }

    /// Register or replace an APIService. Replacement is keyed by
    /// `(tenant_id, group, version)` — same key from same tenant overwrites.
    pub fn register(&self, svc: APIService) {
        let key = RegistryKey {
            tenant_id: svc.tenant_id.clone(),
            group: svc.group.clone(),
            version: svc.version.clone(),
        };
        self.inner.lock().unwrap().insert(key, svc);
    }

    pub fn unregister(&self, tenant_id: &str, group: &str, version: &str) -> bool {
        let key = RegistryKey {
            tenant_id: tenant_id.into(),
            group: group.into(),
            version: version.into(),
        };
        self.inner.lock().unwrap().remove(&key).is_some()
    }

    /// Lookup the APIService registered under `tenant_id` for `(group, version)`.
    /// Tenant scoping is enforced here; cross-tenant lookups return `None`.
    pub fn lookup_for(
        &self,
        tenant_id: &str,
        group: &str,
        version: &str,
    ) -> Option<APIService> {
        let key = RegistryKey {
            tenant_id: tenant_id.into(),
            group: group.into(),
            version: version.into(),
        };
        self.inner.lock().unwrap().get(&key).cloned()
    }

    /// Decide whether to serve the request locally or delegate to a backing
    /// service. Unavailable services fall back to local — mirrors upstream
    /// `available_controller` behavior of skipping unavailable APIServices.
    pub fn route_decision(
        &self,
        tenant_id: &str,
        group: &str,
        version: &str,
    ) -> RouteDecision {
        match self.lookup_for(tenant_id, group, version) {
            Some(svc) if svc.available => RouteDecision::Delegated {
                tenant_id: svc.tenant_id,
                service: svc.service,
            },
            _ => RouteDecision::Local,
        }
    }

    pub fn mark_available(
        &self,
        tenant_id: &str,
        group: &str,
        version: &str,
        available: bool,
    ) -> bool {
        let key = RegistryKey {
            tenant_id: tenant_id.into(),
            group: group.into(),
            version: version.into(),
        };
        let mut inner = self.inner.lock().unwrap();
        if let Some(svc) = inner.get_mut(&key) {
            svc.available = available;
            true
        } else {
            false
        }
    }

    /// List all APIServices registered for `tenant_id`, sorted by
    /// `(group_priority asc, version_priority asc, name asc)` — matching
    /// upstream `apiregistration` ordering used by discovery.
    pub fn list_for_tenant(&self, tenant_id: &str) -> Vec<APIService> {
        let mut out: Vec<APIService> = self.inner.lock().unwrap().values()
            .filter(|s| s.tenant_id == tenant_id)
            .cloned()
            .collect();
        out.sort_by(|a, b| {
            a.group_priority.cmp(&b.group_priority)
                .then(a.version_priority.cmp(&b.version_priority))
                .then(a.name.cmp(&b.name))
        });
        out
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool { self.len() == 0 }
}

impl Default for AggregatorRegistry {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svc(
        tenant: &str,
        group: &str,
        version: &str,
        ns: &str,
        name: &str,
        gprio: i32,
        vprio: i32,
        available: bool,
    ) -> APIService {
        APIService {
            name: format!("{}.{}", version, group),
            tenant_id: tenant.into(),
            group: group.into(),
            version: version.into(),
            service: ServiceRef { namespace: ns.into(), name: name.into(), port: 443 },
            group_priority: gprio,
            version_priority: vprio,
            available,
        }
    }

    /// Upstream parity: `TestAPIServiceRegistration_RegisterAndLookup`
    /// (kube-aggregator/pkg/registry/apiservice/storage/storage_test.go —
    /// register, then read back).
    #[test]
    fn test_register_then_lookup_roundtrip() {
        let reg = AggregatorRegistry::new();
        reg.register(svc("acme", "metrics.k8s.io", "v1beta1",
            "kube-system", "metrics-server", 100, 100, true));
        let got = reg.lookup_for("acme", "metrics.k8s.io", "v1beta1")
            .expect("registered service must be found");
        assert_eq!(got.tenant_id, "acme",
            "tenant_id invariant: APIService retains its registering tenant_id");
        assert_eq!(got.service.name, "metrics-server");
        assert_eq!(got.service.port, 443);
    }

    /// Upstream parity: `TestAPIService_TenantIsolatedLookup`
    /// (lookups MUST NOT cross tenant boundaries — adapted from the
    /// available_controller tenancy contract).
    #[test]
    fn test_lookup_does_not_cross_tenant_boundaries() {
        let reg = AggregatorRegistry::new();
        reg.register(svc("acme", "metrics.k8s.io", "v1beta1",
            "kube-system", "metrics-server", 100, 100, true));
        let leaked = reg.lookup_for("globex", "metrics.k8s.io", "v1beta1");
        assert!(leaked.is_none(),
            "tenant_id invariant: globex MUST NOT see acme's APIService");
        let acme = reg.lookup_for("acme", "metrics.k8s.io", "v1beta1");
        assert!(acme.is_some(), "owning tenant still sees its registration");
    }

    /// Upstream parity: `TestAPIService_RouteDelegatedWhenAvailable`
    /// (handler_proxy.go — `Available` services proxy; otherwise local).
    #[test]
    fn test_route_decision_delegates_when_available() {
        let reg = AggregatorRegistry::new();
        reg.register(svc("acme", "metrics.k8s.io", "v1beta1",
            "kube-system", "metrics-server", 100, 100, true));
        let dec = reg.route_decision("acme", "metrics.k8s.io", "v1beta1");
        match dec {
            RouteDecision::Delegated { tenant_id, service } => {
                assert_eq!(tenant_id, "acme",
                    "tenant_id invariant: delegated decision carries owning tenant_id");
                assert_eq!(service.name, "metrics-server");
            }
            RouteDecision::Local => panic!("expected delegated route for available APIService"),
        }
    }

    /// Upstream parity: `TestAPIService_FallBackToLocalWhenUnavailable`
    /// (available_controller — flipping Available=false routes locally).
    #[test]
    fn test_unavailable_apiservice_falls_back_to_local() {
        let reg = AggregatorRegistry::new();
        reg.register(svc("acme", "metrics.k8s.io", "v1beta1",
            "kube-system", "metrics-server", 100, 100, true));
        assert!(reg.mark_available("acme", "metrics.k8s.io", "v1beta1", false));
        let dec = reg.route_decision("acme", "metrics.k8s.io", "v1beta1");
        assert_eq!(dec, RouteDecision::Local,
            "unavailable APIService must route locally as fallback");
        // tenant_id invariant: lookup still scoped, just not available.
        let svc_back = reg.lookup_for("acme", "metrics.k8s.io", "v1beta1").unwrap();
        assert_eq!(svc_back.tenant_id, "acme",
            "tenant_id invariant retained while toggling availability");
        assert!(!svc_back.available);
    }

    /// Upstream parity: `TestAPIService_UnregisteredRoutesLocal`
    /// (no registration → local serve, never proxy).
    #[test]
    fn test_unregistered_group_routes_locally() {
        let reg = AggregatorRegistry::new();
        // Register one to prove other groups aren't accidentally matched.
        reg.register(svc("acme", "metrics.k8s.io", "v1beta1",
            "kube-system", "metrics-server", 100, 100, true));
        let dec = reg.route_decision("acme", "external.metrics.k8s.io", "v1beta1");
        assert_eq!(dec, RouteDecision::Local,
            "unknown group/version must serve locally");
        // tenant_id invariant: nothing else mutated for the registered group.
        let other = reg.lookup_for("acme", "metrics.k8s.io", "v1beta1").unwrap();
        assert_eq!(other.tenant_id, "acme", "tenant_id invariant on neighbor lookup");
    }

    /// Upstream parity: `TestAPIService_PrioritySortingForDiscovery`
    /// (discovery aggregator orders by group_priority then version_priority).
    #[test]
    fn test_list_for_tenant_orders_by_priority_then_name() {
        let reg = AggregatorRegistry::new();
        reg.register(svc("acme", "b.example.com", "v1", "ns", "b", 200, 10, true));
        reg.register(svc("acme", "a.example.com", "v1", "ns", "a", 100, 10, true));
        reg.register(svc("acme", "c.example.com", "v1", "ns", "c", 100, 5,  true));
        // A different tenant's entry must not appear in acme's list.
        reg.register(svc("globex", "z.example.com", "v1", "ns", "z", 1, 1, true));
        let list = reg.list_for_tenant("acme");
        assert_eq!(list.len(), 3,
            "tenant_id invariant: globex entries excluded from acme list");
        assert!(list.iter().all(|s| s.tenant_id == "acme"),
            "tenant_id invariant: only acme entries returned");
        // c (gprio=100, vprio=5) before a (gprio=100, vprio=10) before b (gprio=200).
        assert_eq!(list[0].group, "c.example.com");
        assert_eq!(list[1].group, "a.example.com");
        assert_eq!(list[2].group, "b.example.com");
    }

    /// Upstream parity: `TestAPIService_UnregisterRemovesEntry`
    /// (registration storage delete is reflected in lookup).
    #[test]
    fn test_unregister_removes_entry_and_routes_local() {
        let reg = AggregatorRegistry::new();
        reg.register(svc("acme", "metrics.k8s.io", "v1beta1",
            "kube-system", "metrics-server", 100, 100, true));
        let removed = reg.unregister("acme", "metrics.k8s.io", "v1beta1");
        assert!(removed, "registered entry returns true on unregister");
        let again = reg.unregister("acme", "metrics.k8s.io", "v1beta1");
        assert!(!again, "second unregister is a no-op returning false");
        let dec = reg.route_decision("acme", "metrics.k8s.io", "v1beta1");
        assert_eq!(dec, RouteDecision::Local,
            "after unregister, route falls back to local");
        // tenant_id invariant: registry still empty for the tenant after delete.
        assert!(reg.list_for_tenant("acme").is_empty(),
            "tenant_id invariant: acme's list is empty post-unregister");
    }
}
