// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Port of emptiness consolidation + the consolidationPolicy gate from
// pkg/controllers/disruption/emptiness.go and consolidation.go in
// kubernetes-sigs/karpenter v1.12.1 (sha ed490e8).
//
// Upstream distinguishes two consolidation reasons:
//   * "empty"        — a node with no reschedulable pods (highest confidence)
//   * "underutilized"— a node below the utilization threshold
// and gates them on Disruption.ConsolidationPolicy:
//   * WhenEmpty                 → only empty nodes are consolidated
//   * WhenEmptyOrUnderutilized  → both (the default)
//
// cave models per-node occupancy as NodeClaim.utilization (0.0 == empty).

use cave_karpenter::disruption::{
    consolidation_decisions, empty_candidates, Decision, DisruptionReason,
    CONSOLIDATION_POLICY_WHEN_EMPTY, CONSOLIDATION_POLICY_WHEN_EMPTY_OR_UNDERUTILIZED,
};
use cave_karpenter::models::{Budget, Disruption, NodeClaim};

fn claim(name: &str, utilization: f64) -> NodeClaim {
    let mut c = NodeClaim::default();
    c.name = name.into();
    c.utilization = utilization;
    c
}

// ---- constants ---------------------------------------------------------------

#[test]
fn consolidation_policy_constants() {
    assert_eq!(CONSOLIDATION_POLICY_WHEN_EMPTY, "WhenEmpty");
    assert_eq!(
        CONSOLIDATION_POLICY_WHEN_EMPTY_OR_UNDERUTILIZED,
        "WhenEmptyOrUnderutilized"
    );
}

// ---- empty_candidates --------------------------------------------------------

#[test]
fn empty_candidates_flag_zero_utilization() {
    let claims = vec![claim("idle", 0.0), claim("busy", 0.4)];
    let out = empty_candidates(&claims);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].claim_name, "idle");
    assert_eq!(out[0].reason, DisruptionReason::Empty);
}

#[test]
fn empty_candidates_skip_terminated() {
    let mut c = claim("idle", 0.0);
    c.terminated = true;
    assert!(empty_candidates(&[c]).is_empty());
}

// ---- Empty reason participates in budget arbitration -------------------------

#[test]
fn empty_reason_obeys_its_budget() {
    let claims = vec![claim("a", 0.0), claim("b", 0.0), claim("c", 0.0)];
    let candidates = empty_candidates(&claims);
    assert_eq!(candidates.len(), 3);

    let disruption = Disruption {
        consolidation_policy: None,
        consolidate_after: None,
        budgets: vec![Budget {
            nodes: "1".into(),
            schedule: None,
            duration: None,
            reasons: vec!["Empty".into()],
        }],
    };
    let allowed = Decision::apply_budget(candidates, &disruption);
    assert_eq!(allowed.len(), 1, "Empty budget of 1 caps 3 empties to 1");
}

// ---- consolidation_decisions: policy gate ------------------------------------

#[test]
fn policy_when_empty_only_consolidates_empties() {
    let claims = vec![claim("empty", 0.0), claim("under", 0.3)];
    let disruption = Disruption {
        consolidation_policy: Some(CONSOLIDATION_POLICY_WHEN_EMPTY.into()),
        consolidate_after: None,
        budgets: vec![],
    };
    let out = consolidation_decisions(&claims, &disruption, 0.5);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].claim_name, "empty");
    assert_eq!(out[0].reason, DisruptionReason::Empty);
}

#[test]
fn policy_when_empty_or_underutilized_consolidates_both() {
    let claims = vec![claim("empty", 0.0), claim("under", 0.3), claim("busy", 0.9)];
    let disruption = Disruption {
        consolidation_policy: Some(CONSOLIDATION_POLICY_WHEN_EMPTY_OR_UNDERUTILIZED.into()),
        consolidate_after: None,
        budgets: vec![],
    };
    let out = consolidation_decisions(&claims, &disruption, 0.5);
    let by_name: std::collections::BTreeMap<&str, DisruptionReason> =
        out.iter().map(|d| (d.claim_name.as_str(), d.reason)).collect();
    assert_eq!(by_name.get("empty"), Some(&DisruptionReason::Empty));
    assert_eq!(by_name.get("under"), Some(&DisruptionReason::Consolidation));
    assert_eq!(by_name.get("busy"), None, "0.9 > threshold 0.5 is not a candidate");
}

#[test]
fn default_policy_is_empty_or_underutilized() {
    // A nil ConsolidationPolicy defaults to WhenEmptyOrUnderutilized upstream.
    let claims = vec![claim("empty", 0.0), claim("under", 0.3)];
    let disruption = Disruption {
        consolidation_policy: None,
        consolidate_after: None,
        budgets: vec![],
    };
    let out = consolidation_decisions(&claims, &disruption, 0.5);
    assert_eq!(out.len(), 2);
}
