// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cloud route controller — keeps the cloud's VPC route table in sync with
//! the set of `Node.spec.podCIDR` values.
//!
//! Mirrors `staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go`.
//! For each node CIDR, exactly one route must exist on the provider; stale
//! routes (whose name is not in the desired set) are deleted.
//!
//! The deeper API mirrors the upstream pieces upstream pulls in from
//! `pkg/controller/route`:
//! * **CIDR family** detection — IPv4 vs IPv6 routes go on separate route
//!   tables in some clouds, so the planner emits per-family operations.
//! * **Blackhole detection** — routes whose target node has been deleted.
//! * **CIDR validation** — only `/N` form, sanity-check octets/hex.
//! * **Plan vs reconcile** — `plan_routes` is a pure function returning
//!   `RoutePlan { creates, deletes }`, so the controller can be tested
//!   independently of any provider.

use crate::provider::RoutesIface;
use crate::types::{Cite, CloudError, Reconcile, TenantId};
use serde::{Deserialize, Serialize};

/// Build the canonical route name for `node_name`. Mirrors `routeNameFor` in
/// upstream — `<cluster-id>-<node>`. We accept the cluster prefix as a
/// parameter so multi-cluster tenants don't collide.
pub fn route_name_for(cluster: &str, node_name: &str) -> String {
    format!("{}-{}", cluster, node_name)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesiredRoute {
    pub node_name: String,
    pub pod_cidr: String,
}

// ─── CIDR helpers ────────────────────────────────────────────────────────────

/// IP family of a CIDR string. Mirrors `utilnet.IsIPv6CIDR` upstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CidrFamily {
    V4,
    V6,
}

/// Parse a CIDR string into its family, returning `None` if malformed.
/// Strict-form: `<addr>/<bits>` with `bits` ∈ `[0, 32]` (V4) or `[0, 128]` (V6).
pub fn cidr_family(s: &str) -> Option<CidrFamily> {
    let (addr, prefix) = s.split_once('/')?;
    let bits: u32 = prefix.parse().ok()?;
    if addr.contains(':') {
        let ok = bits <= 128 && addr.split(':').all(|seg| seg.is_empty() || seg.chars().all(|c| c.is_ascii_hexdigit()) && seg.len() <= 4);
        if ok {
            Some(CidrFamily::V6)
        } else {
            None
        }
    } else if addr.contains('.') {
        let parts: Vec<&str> = addr.split('.').collect();
        if parts.len() != 4 || bits > 32 {
            return None;
        }
        for p in parts {
            let n: u32 = p.parse().ok()?;
            if n > 255 {
                return None;
            }
        }
        Some(CidrFamily::V4)
    } else {
        None
    }
}

/// True iff `s` parses as a valid CIDR.
pub fn is_valid_cidr(s: &str) -> bool {
    cidr_family(s).is_some()
}

/// Split a list of routes into (V4, V6) pairs. Mirrors the per-family route
/// table writes upstream emits for AWS/GCP, where each family lives on its
/// own table.
pub fn split_by_family(desired: &[DesiredRoute]) -> (Vec<DesiredRoute>, Vec<DesiredRoute>) {
    let mut v4 = Vec::new();
    let mut v6 = Vec::new();
    for d in desired {
        match cidr_family(&d.pod_cidr) {
            Some(CidrFamily::V4) => v4.push(d.clone()),
            Some(CidrFamily::V6) => v6.push(d.clone()),
            None => {}
        }
    }
    (v4, v6)
}

// ─── Plan ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutePlan {
    pub creates: Vec<DesiredRoute>,
    pub deletes: Vec<String>,
}

impl RoutePlan {
    pub fn is_empty(&self) -> bool {
        self.creates.is_empty() && self.deletes.is_empty()
    }
    pub fn write_count(&self) -> u32 {
        (self.creates.len() + self.deletes.len()) as u32
    }
}

/// Compute the (creates, deletes) required to make `current` match `desired`.
/// Mirrors `reconcile` in upstream. Only entries with valid CIDRs are
/// scheduled for creation; malformed CIDRs are dropped silently.
pub fn diff(cluster: &str, desired: &[DesiredRoute], current: &[String]) -> (Vec<String>, Vec<String>) {
    let plan = plan_routes(cluster, desired, current);
    let creates = plan.creates.into_iter().map(|d| route_name_for(cluster, &d.node_name)).collect();
    (creates, plan.deletes)
}

/// Pure planner — same shape as upstream `reconcile`, returning a
/// `RoutePlan`. Skips desired routes with malformed CIDRs.
pub fn plan_routes(cluster: &str, desired: &[DesiredRoute], current: &[String]) -> RoutePlan {
    let valid: Vec<&DesiredRoute> = desired.iter().filter(|d| is_valid_cidr(&d.pod_cidr)).collect();
    let want_names: Vec<String> = valid
        .iter()
        .map(|d| route_name_for(cluster, &d.node_name))
        .collect();
    let creates: Vec<DesiredRoute> = valid
        .iter()
        .enumerate()
        .filter(|(i, _)| !current.contains(&want_names[*i]))
        .map(|(_, d)| (*d).clone())
        .collect();
    let deletes: Vec<String> = current
        .iter()
        .filter(|n| !want_names.contains(n))
        .cloned()
        .collect();
    RoutePlan { creates, deletes }
}

/// Detect blackhole routes — routes that exist on the cloud but no longer
/// have a desired node. Mirrors `findBlackholeRoutes` upstream.
pub fn detect_blackhole(current: &[String], live_route_names: &[String]) -> Vec<String> {
    current.iter().filter(|n| !live_route_names.contains(n)).cloned().collect()
}

/// Reject duplicate desired CIDRs — multiple nodes claiming the same CIDR
/// indicate an allocator bug. Mirrors the warning upstream emits in
/// `nodeipam`.
pub fn detect_cidr_collisions(desired: &[DesiredRoute]) -> Vec<String> {
    let mut seen: Vec<&str> = Vec::new();
    let mut dupes = Vec::new();
    for d in desired {
        if seen.contains(&d.pod_cidr.as_str()) {
            dupes.push(d.pod_cidr.clone());
        } else {
            seen.push(&d.pod_cidr);
        }
    }
    dupes
}

// ─── Reconcile ───────────────────────────────────────────────────────────────

/// Mirrors `reconcileRoutes` in upstream.
pub fn reconcile<P: RoutesIface>(
    provider: &P,
    cluster: &str,
    desired: &[DesiredRoute],
    tenant: &TenantId,
) -> Result<Reconcile, CloudError> {
    provider.authorise(tenant, "Route", cluster)?;
    let current = provider.list_routes(tenant)?;
    let plan = plan_routes(cluster, desired, &current);
    for d in &plan.creates {
        let name = route_name_for(cluster, &d.node_name);
        provider.create_route(tenant, &name, &d.pod_cidr)?;
    }
    for n in &plan.deletes {
        provider.delete_route(tenant, n)?;
    }
    Ok(match (plan.creates.len() as u32, plan.deletes.len() as u32) {
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
    use crate::test_ctx;
    use crate::types::{ProviderName, TenantId};
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
                    tenant: TenantId::new(tenant).expect("test fixture"),
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

    fn dr(node: &str, cidr: &str) -> DesiredRoute {
        DesiredRoute { node_name: node.into(), pod_cidr: cidr.into() }
    }

    // ─── Existing v1 tests ───────────────────────────────────────────────────

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
        let desired = vec![dr("n1", "10.0.0.0/24"), dr("n2", "10.0.1.0/24")];
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
        let desired = vec![dr("n1", "10.0.0.0/24")];
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
        let desired = vec![dr("new", "10.0.0.0/24")];
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
        let desired = vec![dr("n1", "10.0.0.0/24")];
        assert_eq!(reconcile(&p, "acme", &desired, &tenant).unwrap(), Reconcile::NoOp);
    }

    // ─── CIDR validation ─────────────────────────────────────────────────────

    #[test]
    fn cidr_family_recognises_ipv4() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/utils/net/ipnet.go",
            "IsIPv6CIDR",
            "tenant-cidr-v4"
        );
        assert_eq!(cidr_family("10.0.0.0/8"), Some(CidrFamily::V4));
        assert_eq!(cidr_family("192.168.1.0/24"), Some(CidrFamily::V4));
        assert_eq!(cidr_family("0.0.0.0/0"), Some(CidrFamily::V4));
    }

    #[test]
    fn cidr_family_recognises_ipv6() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/utils/net/ipnet.go",
            "IsIPv6CIDR",
            "tenant-cidr-v6"
        );
        assert_eq!(cidr_family("2001:db8::/64"), Some(CidrFamily::V6));
        assert_eq!(cidr_family("fc00::/7"), Some(CidrFamily::V6));
    }

    #[test]
    fn cidr_family_rejects_garbage() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/utils/net/ipnet.go",
            "IsIPv6CIDR",
            "tenant-cidr-bad"
        );
        assert!(cidr_family("not-a-cidr").is_none());
        assert!(cidr_family("10.0.0.0").is_none());
        assert!(cidr_family("10.0.0.0/").is_none());
        assert!(cidr_family("10.0.0.0/33").is_none()); // out of range
        assert!(cidr_family("999.0.0.0/8").is_none()); // octet overflow
    }

    #[test]
    fn is_valid_cidr_is_a_thin_wrapper() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/utils/net/ipnet.go",
            "ParseCIDR",
            "tenant-cidr-valid"
        );
        assert!(is_valid_cidr("10.0.0.0/24"));
        assert!(!is_valid_cidr("garbage"));
    }

    #[test]
    fn split_by_family_partitions_routes() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "reconcile",
            "tenant-cidr-split"
        );
        let desired =
            vec![dr("n1", "10.0.0.0/24"), dr("n2", "2001:db8::/64"), dr("bad", "garbage")];
        let (v4, v6) = split_by_family(&desired);
        assert_eq!(v4.len(), 1);
        assert_eq!(v6.len(), 1);
        assert_eq!(v4[0].node_name, "n1");
        assert_eq!(v6[0].node_name, "n2");
    }

    // ─── plan_routes / RoutePlan ─────────────────────────────────────────────

    #[test]
    fn plan_routes_drops_routes_with_malformed_cidrs() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "reconcile",
            "tenant-plan-bad"
        );
        let desired = vec![dr("n1", "10.0.0.0/24"), dr("n2", "garbage")];
        let plan = plan_routes("c1", &desired, &[]);
        assert_eq!(plan.creates.len(), 1);
        assert_eq!(plan.creates[0].node_name, "n1");
    }

    #[test]
    fn plan_routes_returns_empty_for_steady_state() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "reconcile",
            "tenant-plan-stable"
        );
        let desired = vec![dr("n1", "10.0.0.0/24")];
        let plan = plan_routes("c1", &desired, &["c1-n1".into()]);
        assert!(plan.is_empty());
        assert_eq!(plan.write_count(), 0);
    }

    #[test]
    fn plan_routes_emits_creates_and_deletes_in_one_pass() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "reconcile",
            "tenant-plan-full"
        );
        let desired = vec![dr("a", "10.0.1.0/24"), dr("b", "10.0.2.0/24")];
        let plan = plan_routes("c1", &desired, &["c1-old1".into(), "c1-old2".into()]);
        assert_eq!(plan.creates.len(), 2);
        assert_eq!(plan.deletes.len(), 2);
        assert_eq!(plan.write_count(), 4);
    }

    #[test]
    fn plan_routes_carries_cidr_through_to_create_step() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "createRoute",
            "tenant-plan-cidr"
        );
        let desired = vec![dr("n1", "10.0.0.0/24")];
        let plan = plan_routes("c1", &desired, &[]);
        assert_eq!(plan.creates[0].pod_cidr, "10.0.0.0/24");
    }

    // ─── Blackhole detection ─────────────────────────────────────────────────

    #[test]
    fn detect_blackhole_finds_routes_without_a_live_node() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "findBlackholeRoutes",
            "tenant-blackhole"
        );
        let current = vec!["c1-n1".to_string(), "c1-deleted".to_string()];
        let live = vec!["c1-n1".to_string()];
        let bh = detect_blackhole(&current, &live);
        assert_eq!(bh, vec!["c1-deleted".to_string()]);
    }

    #[test]
    fn detect_blackhole_returns_empty_when_all_routes_have_nodes() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "findBlackholeRoutes",
            "tenant-blackhole-none"
        );
        let current = vec!["c1-n1".to_string(), "c1-n2".to_string()];
        let live = vec!["c1-n1".to_string(), "c1-n2".to_string()];
        assert!(detect_blackhole(&current, &live).is_empty());
    }

    // ─── CIDR collisions ─────────────────────────────────────────────────────

    #[test]
    fn detect_cidr_collisions_reports_dupes() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/nodeipam/ipam/range_allocator.go",
            "occupyCIDRs",
            "tenant-cidr-collide"
        );
        let desired = vec![dr("n1", "10.0.0.0/24"), dr("n2", "10.0.0.0/24")];
        let dupes = detect_cidr_collisions(&desired);
        assert_eq!(dupes, vec!["10.0.0.0/24".to_string()]);
    }

    #[test]
    fn detect_cidr_collisions_returns_empty_for_distinct_cidrs() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/nodeipam/ipam/range_allocator.go",
            "occupyCIDRs",
            "tenant-cidr-uniq"
        );
        let desired = vec![dr("n1", "10.0.0.0/24"), dr("n2", "10.0.1.0/24")];
        assert!(detect_cidr_collisions(&desired).is_empty());
    }

    // ─── Reconcile edge cases ────────────────────────────────────────────────

    #[test]
    fn reconcile_drops_invalid_cidr_silently() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "reconcileRoutes",
            "acme"
        );
        let p = StubRoutes::new("acme", vec![]);
        let desired = vec![dr("n1", "garbage"), dr("n2", "10.0.0.0/24")];
        let r = reconcile(&p, "acme", &desired, &tenant).unwrap();
        assert_eq!(r, Reconcile::Update(1));
        assert_eq!(p.table.borrow().clone(), vec!["acme-n2".to_string()]);
    }

    #[test]
    fn reconcile_handles_multi_route_creation_in_order() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "reconcileRoutes",
            "acme"
        );
        let p = StubRoutes::new("acme", vec![]);
        let desired = vec![dr("a", "10.0.0.0/24"), dr("b", "10.0.1.0/24"), dr("c", "10.0.2.0/24")];
        let r = reconcile(&p, "acme", &desired, &tenant).unwrap();
        assert_eq!(r, Reconcile::Update(3));
        assert_eq!(p.table.borrow().len(), 3);
    }

    #[test]
    fn reconcile_emits_delete_only_when_no_creates() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "reconcileRoutes",
            "acme"
        );
        let p = StubRoutes::new("acme", vec!["acme-old"]);
        let r = reconcile(&p, "acme", &[], &tenant).unwrap();
        assert_eq!(r, Reconcile::Delete(1));
        assert!(p.table.borrow().is_empty());
    }

    #[test]
    fn reconcile_returns_tenant_denied_for_wrong_caller() {
        let (_cite, attacker) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "reconcileRoutes",
            "tenant-attacker"
        );
        let p = StubRoutes::new("acme", vec![]);
        let err = reconcile(&p, "acme", &[dr("n1", "10.0.0.0/24")], &attacker).unwrap_err();
        assert!(matches!(err, CloudError::TenantDenied { .. }));
    }

    #[test]
    fn reconcile_is_idempotent_under_repeated_calls() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "reconcileRoutes",
            "acme"
        );
        let p = StubRoutes::new("acme", vec![]);
        let desired = vec![dr("a", "10.0.0.0/24")];
        reconcile(&p, "acme", &desired, &tenant).unwrap();
        let r = reconcile(&p, "acme", &desired, &tenant).unwrap();
        assert_eq!(r, Reconcile::NoOp);
    }

    #[test]
    fn reconcile_handles_dual_stack_routes() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "reconcileRoutes",
            "acme"
        );
        let p = StubRoutes::new("acme", vec![]);
        let desired = vec![dr("v4", "10.0.0.0/24"), dr("v6", "2001:db8::/64")];
        let r = reconcile(&p, "acme", &desired, &tenant).unwrap();
        assert_eq!(r, Reconcile::Update(2));
        assert_eq!(p.table.borrow().len(), 2);
    }

    // ─── RoutePlan helpers ───────────────────────────────────────────────────

    #[test]
    fn route_plan_is_empty_helper_matches_zero_writes() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "reconcile",
            "tenant-plan-empty"
        );
        let plan = RoutePlan { creates: vec![], deletes: vec![] };
        assert!(plan.is_empty());
        assert_eq!(plan.write_count(), 0);
    }

    #[test]
    fn route_plan_write_count_sums_creates_and_deletes() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "reconcile",
            "tenant-plan-count"
        );
        let plan = RoutePlan {
            creates: vec![dr("a", "10.0.0.0/24"), dr("b", "10.0.1.0/24")],
            deletes: vec!["c1-old".into()],
        };
        assert_eq!(plan.write_count(), 3);
        assert!(!plan.is_empty());
    }
}
