// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Karpenter scheduler stub.
//!
//! Upstream reference: pkg/controllers/provisioning/scheduling/scheduler.go
//!
//! The real scheduler enumerates NodeClaim candidates by scoring NodePools
//! against pending Pod requirements. This stub provides only the trait surface
//! and a deterministic "first-match" implementation; production logic is
//! pending.

use crate::models::{NodeClaim, NodeClaimSpec, NodePool, Requirement, RequirementOperator};

/// Schedule decision: which NodePool produced a NodeClaim, or why none did.
#[derive(Debug, Clone)]
pub enum ScheduleOutcome {
    Provisioned { pool: String, claim: NodeClaim },
    NoMatch { reason: String },
}

/// Returns the first NodePool whose template requirements can satisfy `pod_reqs`.
/// `pod_reqs` are `(label_key, label_value)` pairs the pod requested via
/// `nodeSelector` / `requiredDuringSchedulingIgnoredDuringExecution`.
pub fn schedule_first_match(pools: &[NodePool], pod_reqs: &[(String, String)]) -> ScheduleOutcome {
    for pool in pools {
        if pool_satisfies(pool, pod_reqs) {
            let claim = NodeClaim {
                name: format!("{}-claim", pool.name),
                namespace: pool.namespace.clone(),
                spec: NodeClaimSpec {
                    requirements: pool.template.spec.requirements.clone(),
                    taints: pool.template.spec.taints.clone(),
                    startup_taints: pool.template.spec.startup_taints.clone(),
                    node_class_ref: pool.template.spec.node_class_ref.clone(),
                    expire_after: pool.template.spec.expire_after.clone(),
                    termination_grace_period: pool.template.spec.termination_grace_period.clone(),
                },
                status: None,
            };
            return ScheduleOutcome::Provisioned { pool: pool.name.clone(), claim };
        }
    }
    ScheduleOutcome::NoMatch { reason: "no NodePool satisfied pod requirements".to_string() }
}

fn pool_satisfies(pool: &NodePool, pod_reqs: &[(String, String)]) -> bool {
    for (k, v) in pod_reqs {
        if !requirement_satisfies(&pool.template.spec.requirements, k, v) {
            return false;
        }
    }
    true
}

fn requirement_satisfies(reqs: &[Requirement], key: &str, value: &str) -> bool {
    for r in reqs.iter().filter(|r| r.key == key) {
        match r.operator {
            RequirementOperator::In => return r.values.iter().any(|x| x == value),
            RequirementOperator::NotIn => return !r.values.iter().any(|x| x == value),
            RequirementOperator::Exists => return true,
            RequirementOperator::DoesNotExist => return false,
            RequirementOperator::Gt => {
                if let (Ok(want), Ok(threshold)) = (value.parse::<i64>(), r.values.first().map(|s| s.parse::<i64>()).unwrap_or(Ok(0))) {
                    return want > threshold;
                }
                return false;
            }
            RequirementOperator::Lt => {
                if let (Ok(want), Ok(threshold)) = (value.parse::<i64>(), r.values.first().map(|s| s.parse::<i64>()).unwrap_or(Ok(0))) {
                    return want < threshold;
                }
                return false;
            }
        }
    }
    // No requirement on this key → pool is permissive.
    true
}
