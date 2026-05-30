// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-karpenter: Karpenter node-autoscaler reimplementation (scaffold).
//!
//! Upstream: kubernetes-sigs/karpenter v1.12.0
//!
//! Modules:
//!   models               — NodePool / NodeClaim / NodeClass v1 CRD shapes
//!   store                — In-memory store (RwLock)
//!   scheduler            — first-match NodePool selection
//!   batcher              — pending-pod queue with scheduling-round window
//!   binpack              — first-fit-decreasing instance assignment with
//!                          topology spread + taint-intolerance + in-flight
//!                          reservation
//!   disruption           — consolidation / drift / expiration decisions
//!                          with Budget enforcement
//!   nodeclaim_lifecycle  — launch / drain / terminate over a
//!                          CloudProvider abstraction
//!   provider             — CloudProvider trait + StaticProvider +
//!                          Hetzner/Azure NodeClass envelopes
//!
//! 4-track status (honest):
//!   Backend   2/4 — scaffold + Phase 2 deep-port (disruption, lifecycle,
//!                   batcher, binpack, provider abstraction)
//!   Portal    0/4 — admin page Phase 3 alongside cave-ccm
//!   cavectl   0/4 — `cavectl karpenter` Phase 3
//!   Observ.   0/4 — alerts + dashboard Phase 3

pub mod batcher;
pub mod binpack;
pub mod budgets;
pub mod disruption;
pub mod drain;
pub mod labels;
pub mod models;
pub mod nodeclaim_lifecycle;
pub mod provider;
pub mod resources;
pub mod scheduler;
pub mod scheduling;
pub mod store;

pub use models::{
    Budget, Disruption, Limits, NodeClaim, NodeClaimSpec, NodeClaimStatus, NodeClaimTemplate,
    NodeClass, NodeClassRef, NodePool, Requirement, RequirementOperator, Taint,
};
pub use scheduler::{ScheduleOutcome, schedule_first_match};
pub use store::Store;

pub const MODULE_NAME: &str = "cave-karpenter";
pub const UPSTREAM_REPO: &str = "kubernetes-sigs/karpenter";
pub const UPSTREAM_VERSION: &str = "v1.12.0";

#[cfg(test)]
mod tests {
    use super::*;

    fn pool(name: &str, key: &str, op: RequirementOperator, vals: &[&str]) -> NodePool {
        let mut p = NodePool::default();
        p.name = name.to_string();
        p.template.spec.requirements.push(Requirement {
            key: key.to_string(),
            operator: op,
            values: vals.iter().map(|s| s.to_string()).collect(),
            min_values: None,
        });
        p
    }

    #[test]
    fn store_round_trips_pool() {
        let s = Store::new();
        let mut p = NodePool::default();
        p.name = "default".to_string();
        s.put_pool(p);
        assert_eq!(
            s.get_pool("default").map(|x| x.name),
            Some("default".to_string())
        );
        assert_eq!(s.list_pools().len(), 1);
        assert!(s.delete_pool("default"));
        assert!(s.list_pools().is_empty());
    }

    #[test]
    fn schedule_first_match_picks_in_operator_pool() {
        let p = pool(
            "gpu",
            "node.kubernetes.io/instance-type",
            RequirementOperator::In,
            &["g4dn.xlarge"],
        );
        let outcome = schedule_first_match(
            &[p],
            &[(
                "node.kubernetes.io/instance-type".to_string(),
                "g4dn.xlarge".to_string(),
            )],
        );
        match outcome {
            ScheduleOutcome::Provisioned { pool, claim } => {
                assert_eq!(pool, "gpu");
                assert_eq!(claim.name, "gpu-claim");
            }
            ScheduleOutcome::NoMatch { .. } => panic!("expected match"),
        }
    }

    #[test]
    fn schedule_first_match_skips_pool_with_not_in() {
        let p = pool(
            "default",
            "topology.kubernetes.io/zone",
            RequirementOperator::NotIn,
            &["us-east-1a"],
        );
        let outcome = schedule_first_match(
            &[p],
            &[(
                "topology.kubernetes.io/zone".to_string(),
                "us-east-1a".to_string(),
            )],
        );
        assert!(matches!(outcome, ScheduleOutcome::NoMatch { .. }));
    }

    #[test]
    fn schedule_no_pool_returns_no_match() {
        let outcome = schedule_first_match(&[], &[("any".to_string(), "x".to_string())]);
        assert!(matches!(outcome, ScheduleOutcome::NoMatch { .. }));
    }

    #[test]
    fn module_constants_exposed() {
        assert_eq!(MODULE_NAME, "cave-karpenter");
        assert_eq!(UPSTREAM_REPO, "kubernetes-sigs/karpenter");
        assert!(UPSTREAM_VERSION.starts_with('v'));
    }
}
