//! Cloud route controller — keeps the cloud's VPC route table in sync with
//! the set of `Node.spec.podCIDR` values.
//!
//! Mirrors `staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go`.
//! For each node CIDR, exactly one route must exist on the provider; stale
//! routes (whose name is not in the desired set) are deleted.

use crate::provider::RoutesIface;
use crate::types::{Cite, CloudError, Reconcile, TenantId};
use serde::{Deserialize, Serialize};

/// Build the canonical route name for `node_name`. Mirrors `routeNameFor` in
/// upstream — `<cluster-id>-<node>`. We accept the cluster prefix as a
/// parameter so multi-cluster tenants don't collide.
pub fn route_name_for(cluster: &str, node_name: &str) -> String {
    format!("{}-{}", cluster, node_name)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesiredRoute {
    pub node_name: String,
    pub pod_cidr: String,
}

/// Compute the (creates, deletes) required to make `current` match `desired`.
/// Mirrors `reconcile` in upstream.
pub fn diff(cluster: &str, desired: &[DesiredRoute], current: &[String]) -> (Vec<String>, Vec<String>) {
    let want_names: Vec<String> = desired
        .iter()
        .map(|d| route_name_for(cluster, &d.node_name))
        .collect();
    let creates: Vec<String> = want_names
        .iter()
        .filter(|n| !current.contains(n))
        .cloned()
        .collect();
    let deletes: Vec<String> = current
        .iter()
        .filter(|n| !want_names.contains(n))
        .cloned()
        .collect();
    (creates, deletes)
}

/// Mirrors `reconcileRoutes` in upstream.
pub fn reconcile<P: RoutesIface>(
    provider: &P,
    cluster: &str,
    desired: &[DesiredRoute],
    tenant: &TenantId,
) -> Result<Reconcile, CloudError> {
    provider.authorise(tenant, "Route", cluster)?;
    let current = provider.list_routes(tenant)?;
    let (creates, deletes) = diff(cluster, desired, &current);
    for c in &creates {
        let cidr = desired
            .iter()
            .find(|d| route_name_for(cluster, &d.node_name) == *c)
            .map(|d| d.pod_cidr.as_str())
            .unwrap_or("0.0.0.0/0");
        provider.create_route(tenant, c, cidr)?;
    }
    for d in &deletes {
        provider.delete_route(tenant, d)?;
    }
    Ok(match (creates.len() as u32, deletes.len() as u32) {
        (0, 0) => Reconcile::NoOp,
        (n, 0) => Reconcile::Update(n),
        (0, m) => Reconcile::Delete(m),
        (n, m) => Reconcile::Update(n + m),
    })
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::k8s(
    "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
    "RouteController",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{CloudConfig, CloudProvider};
    use crate::types::{ProviderName, TenantId};
    use crate::test_ctx;
    use std::cell::RefCell;

    /// In-memory provider. `RefCell` is fine — single-threaded tests only.
    struct StubRoutes {
        cfg: CloudConfig,
        table: RefCell<Vec<String>>,
    }
    impl StubRoutes {
        fn new(tenant: &str, initial: Vec<&str>) -> Self {
            Self {
                cfg: CloudConfig {
                    tenant: TenantId::new(tenant),
                    provider: ProviderName::Hetzner,
                    region: "fsn1".into(),
                    credential_ref: "vault://kv/hcloud".into(),
                },
                table: RefCell::new(initial.into_iter().map(String::from).collect()),
            }
        }
    }
    impl CloudProvider for StubRoutes {
        fn name(&self) -> ProviderName {
            self.cfg.provider
        }
        fn config(&self) -> &CloudConfig {
            &self.cfg
        }
    }
    impl RoutesIface for StubRoutes {
        fn list_routes(&self, _t: &TenantId) -> Result<Vec<String>, CloudError> {
            Ok(self.table.borrow().clone())
        }
        fn create_route(&self, _t: &TenantId, name: &str, _cidr: &str) -> Result<(), CloudError> {
            self.table.borrow_mut().push(name.into());
            Ok(())
        }
        fn delete_route(&self, _t: &TenantId, name: &str) -> Result<(), CloudError> {
            self.table.borrow_mut().retain(|n| n != name);
            Ok(())
        }
    }

    #[test]
    fn route_name_uses_cluster_prefix() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "routeNameFor",
            "tenant-route-name"
        );
        let _ = tenant;
        assert_eq!(route_name_for("prod", "node-1"), "prod-node-1");
    }

    #[test]
    fn diff_returns_missing_routes_as_creates() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "reconcile",
            "tenant-route-diff"
        );
        let _ = tenant;
        let desired = vec![
            DesiredRoute { node_name: "n1".into(), pod_cidr: "10.0.0.0/24".into() },
            DesiredRoute { node_name: "n2".into(), pod_cidr: "10.0.1.0/24".into() },
        ];
        let (c, d) = diff("c1", &desired, &vec!["c1-n1".into()]);
        assert_eq!(c, vec!["c1-n2".to_string()]);
        assert_eq!(d, Vec::<String>::new());
    }

    #[test]
    fn diff_returns_stale_routes_as_deletes() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "reconcile",
            "tenant-route-stale"
        );
        let _ = tenant;
        let desired = vec![DesiredRoute { node_name: "n1".into(), pod_cidr: "10.0.0.0/24".into() }];
        let (c, d) = diff("c1", &desired, &vec!["c1-n1".into(), "c1-old".into()]);
        assert!(c.is_empty());
        assert_eq!(d, vec!["c1-old".to_string()]);
    }

    #[test]
    fn reconcile_creates_then_deletes_to_converge() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "reconcileRoutes",
            "acme"
        );
        let p = StubRoutes::new("acme", vec!["acme-old"]);
        let desired = vec![DesiredRoute { node_name: "new".into(), pod_cidr: "10.0.0.0/24".into() }];
        let r = reconcile(&p, "acme", &desired, &tenant).unwrap();
        assert_eq!(r, Reconcile::Update(2)); // 1 create + 1 delete
        assert_eq!(p.table.borrow().clone(), vec!["acme-new".to_string()]);
    }

    #[test]
    fn reconcile_is_a_no_op_when_table_already_matches() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "reconcileRoutes",
            "acme"
        );
        let p = StubRoutes::new("acme", vec!["acme-n1"]);
        let desired = vec![DesiredRoute { node_name: "n1".into(), pod_cidr: "10.0.0.0/24".into() }];
        assert_eq!(reconcile(&p, "acme", &desired, &tenant).unwrap(), Reconcile::NoOp);
    }
}
