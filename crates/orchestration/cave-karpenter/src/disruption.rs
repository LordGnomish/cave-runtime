// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Disruption controller — consolidation / drift / expiration decisions
//! with Budget enforcement.
//!
//! Upstream reference (Karpenter v1.4.0):
//!   pkg/controllers/disruption/consolidation/*.go
//!   pkg/controllers/disruption/drift.go
//!   pkg/controllers/disruption/expiration.go
//!   pkg/controllers/disruption/orchestration/queue.go (budget arbiter)
//!
//! The upstream implementation reconciles against a live API server. The
//! Cave port keeps the decision logic pure — taking a snapshot of nodes
//! and pools and emitting a list of [`Decision`]s. The dispatch into a
//! real Kubernetes event queue is deferred to cave-cloud-controller-manager.

use crate::models::{Disruption, NodeClaim, NodePool};
use std::time::{Duration, SystemTime};

/// Why a node was flagged for disruption.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DisruptionReason {
    /// Underutilised compared to the pool's `consolidation_policy` threshold.
    Consolidation,
    /// Node was launched with a now-stale NodePool template hash.
    Drift,
    /// Node exceeded the `spec.expire_after` lifetime.
    Expiration,
    /// Node is permanently unhealthy and should be replaced.
    Unhealthy,
}

/// One disruption decision against a specific NodeClaim.
#[derive(Debug, Clone)]
pub struct Decision {
    pub claim_name: String,
    pub reason: DisruptionReason,
    pub message: String,
}

impl Decision {
    /// Apply the Budget array from a NodePool's `disruption` block, capping
    /// the candidate set to whatever the budget allows for the matching
    /// reason. Mirrors `orchestration/queue.go::Budget.Allowed`.
    ///
    /// `nodes` is parsed as either an integer cap or a percentage (`"10%"`).
    /// Percentage parsing intentionally rounds *down* — upstream behaviour.
    pub fn apply_budget(candidates: Vec<Decision>, disruption: &Disruption) -> Vec<Decision> {
        if disruption.budgets.is_empty() {
            return candidates;
        }
        let total = candidates.len();
        let mut out: Vec<Decision> = Vec::with_capacity(total);
        let mut consumed_by_reason: std::collections::BTreeMap<DisruptionReason, usize> =
            Default::default();

        for c in candidates {
            let cap = budget_cap_for(disruption, c.reason, total);
            let used = consumed_by_reason.entry(c.reason).or_insert(0);
            if *used < cap {
                *used += 1;
                out.push(c);
            }
        }
        out
    }
}

fn budget_cap_for(disruption: &Disruption, reason: DisruptionReason, total: usize) -> usize {
    let reason_str = match reason {
        DisruptionReason::Consolidation => "Underutilized",
        DisruptionReason::Drift => "Drifted",
        DisruptionReason::Expiration => "Expired",
        DisruptionReason::Unhealthy => "Unhealthy",
    };
    let mut cap = usize::MAX;
    for b in &disruption.budgets {
        let matches_reason = b.reasons.is_empty() || b.reasons.iter().any(|r| r == reason_str);
        if !matches_reason {
            continue;
        }
        if let Some(c) = parse_node_count(&b.nodes, total) {
            cap = cap.min(c);
        }
    }
    cap
}

fn parse_node_count(spec: &str, total: usize) -> Option<usize> {
    let s = spec.trim();
    if let Some(pct) = s.strip_suffix('%') {
        let n: f64 = pct.trim().parse().ok()?;
        Some(((n / 100.0) * total as f64).floor() as usize)
    } else {
        s.parse::<usize>().ok()
    }
}

/// Flag NodeClaims whose `utilization` is at or below `threshold`.
/// Mirrors `pkg/controllers/disruption/consolidation/underutilized.go`.
pub fn consolidation_candidates(claims: &[NodeClaim], threshold: f64) -> Vec<Decision> {
    claims
        .iter()
        .filter(|c| c.utilization <= threshold && !c.terminated)
        .map(|c| Decision {
            claim_name: c.name.clone(),
            reason: DisruptionReason::Consolidation,
            message: format!(
                "node utilization {:.2} <= threshold {:.2}",
                c.utilization, threshold
            ),
        })
        .collect()
}

/// Flag NodeClaims whose `template_hash` differs from their NodePool's
/// current `template_hash`. Mirrors `disruption/drift.go::isNodeClaimDrifted`.
pub fn drift_candidates(claims: &[NodeClaim], pools: &[NodePool]) -> Vec<Decision> {
    let mut out = Vec::new();
    for c in claims {
        if c.terminated {
            continue;
        }
        let Some(pool_name) = c.pool_name.as_ref() else {
            continue;
        };
        let Some(pool) = pools.iter().find(|p| &p.name == pool_name) else {
            continue;
        };
        match (pool.template_hash.as_ref(), c.template_hash.as_ref()) {
            (Some(want), Some(have)) if want != have => out.push(Decision {
                claim_name: c.name.clone(),
                reason: DisruptionReason::Drift,
                message: format!("template hash {have} != pool hash {want}"),
            }),
            _ => {}
        }
    }
    out
}

/// Flag NodeClaims whose `created_at + expire_after` is in the past.
/// Mirrors `disruption/expiration.go::ShouldDisrupt`.
pub fn expiration_candidates(claims: &[NodeClaim], now: SystemTime) -> Vec<Decision> {
    let mut out = Vec::new();
    for c in claims {
        if c.terminated {
            continue;
        }
        let Some(created) = c.created_at else {
            continue;
        };
        let Some(expire_after) = c.spec.expire_after.as_ref() else {
            continue;
        };
        let Ok(dur) = parse_duration(expire_after) else {
            continue;
        };
        if let Ok(elapsed) = now.duration_since(created)
            && elapsed >= dur
        {
            out.push(Decision {
                claim_name: c.name.clone(),
                reason: DisruptionReason::Expiration,
                message: format!("age {:?} >= expire_after {expire_after}", elapsed),
            });
        }
    }
    out
}

/// Parse a "15s" / "5m" / "2h" / "1d" duration string. Mirrors the limited
/// subset Karpenter accepts in `NodeClaimSpec.ExpireAfter`.
pub fn parse_duration(spec: &str) -> Result<Duration, String> {
    let s = spec.trim();
    let (num, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let n: u64 = num.parse().map_err(|e| format!("bad number {num}: {e}"))?;
    let mul = match unit {
        "s" | "" => 1,
        "m" => 60,
        "h" => 60 * 60,
        "d" => 60 * 60 * 24,
        other => return Err(format!("unknown unit {other}")),
    };
    Ok(Duration::from_secs(n * mul))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Budget;

    #[test]
    fn parse_duration_accepts_supported_units() {
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
        assert_eq!(parse_duration("2h").unwrap(), Duration::from_secs(7200));
        assert_eq!(parse_duration("1d").unwrap(), Duration::from_secs(86400));
    }

    #[test]
    fn parse_duration_rejects_unknown_unit() {
        assert!(parse_duration("5y").is_err());
        assert!(parse_duration("abc").is_err());
    }

    #[test]
    fn budget_percentage_rounds_down() {
        let d = Disruption {
            consolidation_policy: None,
            consolidate_after: None,
            budgets: vec![Budget {
                nodes: "33%".into(),
                schedule: None,
                duration: None,
                reasons: vec![],
            }],
        };
        // 10 candidates, 33% → 3 allowed
        let candidates: Vec<Decision> = (0..10)
            .map(|i| Decision {
                claim_name: format!("n{i}"),
                reason: DisruptionReason::Consolidation,
                message: String::new(),
            })
            .collect();
        let allowed = Decision::apply_budget(candidates, &d);
        assert_eq!(allowed.len(), 3);
    }

    #[test]
    fn budget_with_no_matching_reason_does_not_restrict() {
        let d = Disruption {
            consolidation_policy: None,
            consolidate_after: None,
            budgets: vec![Budget {
                nodes: "0".into(),
                schedule: None,
                duration: None,
                reasons: vec!["Drifted".into()],
            }],
        };
        let candidates = vec![Decision {
            claim_name: "n".into(),
            reason: DisruptionReason::Consolidation,
            message: String::new(),
        }];
        let allowed = Decision::apply_budget(candidates, &d);
        assert_eq!(allowed.len(), 1);
    }
}
