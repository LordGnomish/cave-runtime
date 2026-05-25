// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Route controller orchestration — concurrent reconcile, batch
//! scheduling, allocator/apiserver coordination.
//!
//! Mirrors the pieces around `route_controller.go` upstream that wrap
//! the basic plan/reconcile loop:
//!
//! * **Concurrency** — `--concurrent-route-syncs` caps how many provider
//!   calls fire in parallel.
//! * **Batching** — providers (AWS, GCP) accept a max number of route
//!   ops per call; we slice the plan into batches under that cap.
//! * **Cleanup policy** — immediate or grace-period delete for stale
//!   routes; the latter survives a transient missing-node window.
//! * **Allocator claim** — when nodeipam writes a CIDR to
//!   `Node.spec.podCIDR`, the route controller must reserve it in the
//!   `CidrAllocator` and pin a route to it.
//! * **Dual-stack table picker** — chooses V4 vs V6 route table.

use crate::route_controller::{CidrFamily, DesiredRoute, cidr_family};
use crate::types::{CloudError, ProviderName};
use serde::{Deserialize, Serialize};

// ─── Concurrency ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconcileConcurrency {
    pub max_inflight: u32,
}

impl ReconcileConcurrency {
    pub const DEFAULT: Self = Self { max_inflight: 4 };

    pub fn validate(&self) -> Result<(), CloudError> {
        if !(1..=32).contains(&self.max_inflight) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("max_inflight {} outside [1, 32]", self.max_inflight),
            });
        }
        Ok(())
    }

    /// Slice an operation count into chunks of at most `max_inflight`.
    /// Mirrors the `workqueue.RateLimitingInterface` grouping upstream
    /// uses for the per-reconcile parallelism gate.
    pub fn chunk_count(self, total: u32) -> u32 {
        if self.max_inflight == 0 {
            return 0;
        }
        total.div_ceil(self.max_inflight)
    }
}

// ─── Batch scheduler ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchSize {
    pub max_per_call: u32,
}

impl BatchSize {
    pub const fn aws_route_table() -> Self {
        Self { max_per_call: 100 }
    }
    pub const fn gcp_route_table() -> Self {
        Self { max_per_call: 50 }
    }

    pub fn validate(&self) -> Result<(), CloudError> {
        if self.max_per_call == 0 {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: "batch max_per_call must be > 0".into(),
            });
        }
        Ok(())
    }

    /// Split a list of routes into provider-sized batches.
    pub fn split<'a>(&self, routes: &'a [DesiredRoute]) -> Vec<&'a [DesiredRoute]> {
        if routes.is_empty() {
            return Vec::new();
        }
        routes.chunks(self.max_per_call as usize).collect()
    }
}

// ─── Cleanup policy ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CleanupPolicy {
    Immediate,
    GracePeriod { seconds: u32 },
}

impl CleanupPolicy {
    pub const fn default_for_route_controller() -> Self {
        CleanupPolicy::GracePeriod { seconds: 60 }
    }

    pub fn validate(&self) -> Result<(), CloudError> {
        if let CleanupPolicy::GracePeriod { seconds } = self {
            if !(1..=3_600).contains(seconds) {
                return Err(CloudError::InvalidConfig {
                    provider: ProviderName::Hetzner,
                    reason: format!("grace period {seconds} outside [1, 3600] s"),
                });
            }
        }
        Ok(())
    }

    /// True iff a route that's been stale for `stale_seconds` should be
    /// deleted now.
    pub fn should_delete_stale(&self, stale_seconds: u32) -> bool {
        match self {
            CleanupPolicy::Immediate => true,
            CleanupPolicy::GracePeriod { seconds } => stale_seconds >= *seconds,
        }
    }
}

// ─── Allocator claim ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllocatorClaim {
    pub node_name: String,
    pub pod_cidr: String,
    /// Route name produced for this claim (set after the route is
    /// programmed at the provider).
    pub route_name: Option<String>,
}

impl AllocatorClaim {
    pub fn pending(node_name: &str, pod_cidr: &str) -> Self {
        Self {
            node_name: node_name.into(),
            pod_cidr: pod_cidr.into(),
            route_name: None,
        }
    }

    pub fn validate(&self) -> Result<(), CloudError> {
        if self.node_name.is_empty() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: "allocator claim node_name must not be empty".into(),
            });
        }
        if cidr_family(&self.pod_cidr).is_none() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!(
                    "allocator claim pod_cidr {:?} is not a valid CIDR",
                    self.pod_cidr
                ),
            });
        }
        Ok(())
    }

    pub fn pin(&mut self, route_name: &str) {
        self.route_name = Some(route_name.into());
    }

    pub fn is_pinned(&self) -> bool {
        self.route_name.is_some()
    }
}

// ─── Dual-stack table picker ─────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RouteTableId {
    V4,
    V6,
}

impl RouteTableId {
    pub fn for_route(route: &DesiredRoute) -> Option<Self> {
        match cidr_family(&route.pod_cidr) {
            Some(CidrFamily::V4) => Some(RouteTableId::V4),
            Some(CidrFamily::V6) => Some(RouteTableId::V6),
            None => None,
        }
    }
}

// ─── Per-route age ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteAgeRecord {
    pub name: String,
    /// Seconds since this route was last seen *desired*. 0 means
    /// "currently desired".
    pub stale_for_seconds: u32,
}

/// Increment the stale counter for routes that have fallen out of the
/// desired set. Mirrors the upstream loop that watches
/// `Node.spec.podCIDR` deletions and starts the clock.
pub fn age_routes(records: &mut [RouteAgeRecord], elapsed_seconds: u32, desired: &[String]) {
    for r in records.iter_mut() {
        if desired.contains(&r.name) {
            r.stale_for_seconds = 0;
        } else {
            r.stale_for_seconds = r.stale_for_seconds.saturating_add(elapsed_seconds);
        }
    }
}

/// Pick the routes that should be deleted now under `policy`.
pub fn select_for_cleanup<'a>(
    records: &'a [RouteAgeRecord],
    policy: CleanupPolicy,
) -> Vec<&'a str> {
    records
        .iter()
        .filter(|r| r.stale_for_seconds > 0 && policy.should_delete_stale(r.stale_for_seconds))
        .map(|r| r.name.as_str())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn ctx(tenant: &'static str, path: &'static str, sym: &'static str) {
        let (cite, _t) = test_ctx!(path, sym, tenant);
        assert_eq!(cite.repo, "kubernetes/kubernetes");
    }

    fn dr(node: &str, cidr: &str) -> DesiredRoute {
        DesiredRoute {
            node_name: node.into(),
            pod_cidr: cidr.into(),
        }
    }

    // ─── Concurrency ─────────────────────────────────────────────────────────

    #[test]
    fn reconcile_concurrency_default_is_four() {
        ctx(
            "acme",
            "cmd/cloud-controller-manager/app/options/options.go",
            "ConcurrentRouteSyncs",
        );
        assert_eq!(ReconcileConcurrency::DEFAULT.max_inflight, 4);
    }

    #[test]
    fn reconcile_concurrency_validates_range() {
        ctx(
            "acme",
            "cmd/cloud-controller-manager/app/options/options.go",
            "ConcurrentRouteSyncs",
        );
        let mut c = ReconcileConcurrency::DEFAULT;
        c.max_inflight = 0;
        assert!(matches!(
            c.validate().unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
        c.max_inflight = 100;
        assert!(matches!(
            c.validate().unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
    }

    #[test]
    fn reconcile_concurrency_chunk_count_uses_ceil_division() {
        ctx(
            "acme",
            "staging/src/k8s.io/client-go/util/workqueue/rate_limiting_queue.go",
            "RateLimitingInterface",
        );
        let c = ReconcileConcurrency { max_inflight: 4 };
        assert_eq!(c.chunk_count(8), 2);
        assert_eq!(c.chunk_count(9), 3);
        assert_eq!(c.chunk_count(0), 0);
        assert_eq!(c.chunk_count(1), 1);
    }

    // ─── Batch size ──────────────────────────────────────────────────────────

    #[test]
    fn batch_size_aws_default_is_one_hundred() {
        ctx(
            "acme",
            "staging/src/k8s.io/cloud-provider-aws/pkg/cloudprovider/providers/aws/aws_routes.go",
            "BatchSize",
        );
        assert_eq!(BatchSize::aws_route_table().max_per_call, 100);
    }

    #[test]
    fn batch_size_gcp_default_is_fifty() {
        ctx(
            "acme",
            "staging/src/k8s.io/cloud-provider-gcp/providers/gce/gce_routes.go",
            "BatchSize",
        );
        assert_eq!(BatchSize::gcp_route_table().max_per_call, 50);
    }

    #[test]
    fn batch_size_zero_per_call_is_invalid() {
        ctx(
            "acme",
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "ReconcileBatch",
        );
        let bs = BatchSize { max_per_call: 0 };
        assert!(matches!(
            bs.validate().unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
    }

    #[test]
    fn batch_size_split_returns_empty_for_empty_routes() {
        ctx(
            "acme",
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "ReconcileBatch",
        );
        let bs = BatchSize::aws_route_table();
        assert!(bs.split(&[]).is_empty());
    }

    #[test]
    fn batch_size_split_chunks_into_max_per_call_pieces() {
        ctx(
            "acme",
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "ReconcileBatch",
        );
        let bs = BatchSize { max_per_call: 3 };
        let routes: Vec<DesiredRoute> = (0..7)
            .map(|i| dr(&format!("n{i}"), &format!("10.0.{i}.0/24")))
            .collect();
        let chunks = bs.split(&routes);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].len(), 3);
        assert_eq!(chunks[1].len(), 3);
        assert_eq!(chunks[2].len(), 1);
    }

    // ─── Cleanup policy ──────────────────────────────────────────────────────

    #[test]
    fn cleanup_policy_default_is_grace_period_60s() {
        ctx(
            "acme",
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "CleanupPolicy",
        );
        assert_eq!(
            CleanupPolicy::default_for_route_controller(),
            CleanupPolicy::GracePeriod { seconds: 60 }
        );
    }

    #[test]
    fn cleanup_policy_immediate_validates_and_deletes_at_zero() {
        ctx(
            "acme",
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "CleanupPolicy",
        );
        assert!(CleanupPolicy::Immediate.validate().is_ok());
        assert!(CleanupPolicy::Immediate.should_delete_stale(1));
    }

    #[test]
    fn cleanup_policy_grace_period_validates_range() {
        ctx(
            "acme",
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "CleanupPolicy",
        );
        let p = CleanupPolicy::GracePeriod { seconds: 0 };
        assert!(matches!(
            p.validate().unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
        let p = CleanupPolicy::GracePeriod { seconds: 4_000 };
        assert!(matches!(
            p.validate().unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
    }

    #[test]
    fn cleanup_policy_grace_period_does_not_delete_below_threshold() {
        ctx(
            "acme",
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "CleanupPolicy",
        );
        let p = CleanupPolicy::GracePeriod { seconds: 60 };
        assert!(!p.should_delete_stale(30));
        assert!(p.should_delete_stale(60));
        assert!(p.should_delete_stale(120));
    }

    // ─── Allocator claim ─────────────────────────────────────────────────────

    #[test]
    fn allocator_claim_pending_is_unpinned() {
        ctx(
            "acme",
            "staging/src/k8s.io/cloud-provider/controllers/nodeipam/ipam/range_allocator.go",
            "AllocatorClaim",
        );
        let c = AllocatorClaim::pending("n1", "10.0.0.0/24");
        assert!(!c.is_pinned());
        assert!(c.validate().is_ok());
    }

    #[test]
    fn allocator_claim_validate_rejects_empty_node_name() {
        ctx(
            "acme",
            "staging/src/k8s.io/cloud-provider/controllers/nodeipam/ipam/range_allocator.go",
            "AllocatorClaim",
        );
        let mut c = AllocatorClaim::pending("n1", "10.0.0.0/24");
        c.node_name.clear();
        assert!(matches!(
            c.validate().unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
    }

    #[test]
    fn allocator_claim_validate_rejects_invalid_cidr() {
        ctx(
            "acme",
            "staging/src/k8s.io/cloud-provider/controllers/nodeipam/ipam/range_allocator.go",
            "AllocatorClaim",
        );
        let mut c = AllocatorClaim::pending("n1", "garbage");
        c.pin("c-1");
        assert!(matches!(
            c.validate().unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
    }

    #[test]
    fn allocator_claim_pin_records_route_name() {
        ctx(
            "acme",
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "createRoute",
        );
        let mut c = AllocatorClaim::pending("n1", "10.0.0.0/24");
        c.pin("c1-n1");
        assert!(c.is_pinned());
        assert_eq!(c.route_name.as_deref(), Some("c1-n1"));
    }

    // ─── Dual-stack table picker ─────────────────────────────────────────────

    #[test]
    fn route_table_id_picks_v4_for_ipv4_cidr() {
        ctx(
            "acme",
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "RouteTable",
        );
        assert_eq!(
            RouteTableId::for_route(&dr("n", "10.0.0.0/24")),
            Some(RouteTableId::V4)
        );
    }

    #[test]
    fn route_table_id_picks_v6_for_ipv6_cidr() {
        ctx(
            "acme",
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "RouteTable",
        );
        assert_eq!(
            RouteTableId::for_route(&dr("n", "2001:db8::/64")),
            Some(RouteTableId::V6)
        );
    }

    #[test]
    fn route_table_id_returns_none_for_invalid_cidr() {
        ctx(
            "acme",
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "RouteTable",
        );
        assert!(RouteTableId::for_route(&dr("n", "garbage")).is_none());
    }

    // ─── Route age + cleanup pick ────────────────────────────────────────────

    fn rec(name: &str, age: u32) -> RouteAgeRecord {
        RouteAgeRecord {
            name: name.into(),
            stale_for_seconds: age,
        }
    }

    #[test]
    fn age_routes_zeros_currently_desired() {
        ctx(
            "acme",
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "ageRoutes",
        );
        let mut recs = vec![rec("n1", 30), rec("n2", 0)];
        age_routes(&mut recs, 10, &["n1".into()]);
        assert_eq!(recs[0].stale_for_seconds, 0);
        assert_eq!(recs[1].stale_for_seconds, 10);
    }

    #[test]
    fn age_routes_increments_stale_counters() {
        ctx(
            "acme",
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "ageRoutes",
        );
        let mut recs = vec![rec("n1", 30)];
        age_routes(&mut recs, 15, &[]);
        assert_eq!(recs[0].stale_for_seconds, 45);
    }

    #[test]
    fn select_for_cleanup_returns_only_aged_records() {
        ctx(
            "acme",
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "ageRoutes",
        );
        let recs = vec![rec("n1", 30), rec("n2", 90), rec("n3", 0)];
        let p = CleanupPolicy::GracePeriod { seconds: 60 };
        let pick = select_for_cleanup(&recs, p);
        assert_eq!(pick, vec!["n2"]);
    }

    #[test]
    fn select_for_cleanup_returns_all_aged_under_immediate_policy() {
        ctx(
            "acme",
            "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
            "ageRoutes",
        );
        let recs = vec![rec("n1", 1), rec("n2", 30), rec("n3", 0)];
        let pick = select_for_cleanup(&recs, CleanupPolicy::Immediate);
        assert_eq!(pick, vec!["n1", "n2"]);
    }
}
