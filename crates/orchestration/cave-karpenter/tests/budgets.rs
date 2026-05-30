// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Port of the disruption-budget AllowedDisruptions math from
// pkg/apis/v1/nodepool.go (kubernetes-sigs/karpenter v1.12.1, sha ed490e8):
//   * GetIntStrFromValue / GetScaledValueFromIntOrPercent (k8s intstr
//     round-up percentage semantics)
//   * Budget.GetAllowedDisruptions (no-schedule active path)
//   * NodePool.GetAllowedDisruptionsByReason (min across reason-matched
//     budgets, MaxInt32 when unbounded)
//   * MustGetAllowedDisruptions (fail-closed to 0 on error)
//
// The cron-schedule IsActive window (Schedule + Duration) depends on a cron
// parser and is scope-cut this cycle; only no-schedule (always-active)
// budgets are exercised here.

use cave_karpenter::budgets::{
    budget_allowed_disruptions, must_get_allowed_disruptions, nodepool_allowed_disruptions_by_reason,
    scaled_value_from_int_or_percent, UNBOUNDED_DISRUPTIONS,
};
use cave_karpenter::models::Budget;

fn budget(nodes: &str, reasons: &[&str]) -> Budget {
    Budget {
        nodes: nodes.to_string(),
        schedule: None,
        duration: None,
        reasons: reasons.iter().map(|s| s.to_string()).collect(),
    }
}

// ---- GetScaledValueFromIntOrPercent ------------------------------------------

#[test]
fn scaled_int_passes_through_ignoring_total() {
    assert_eq!(scaled_value_from_int_or_percent("10", 100, true).unwrap(), 10);
    assert_eq!(scaled_value_from_int_or_percent("3", 0, true).unwrap(), 3);
}

#[test]
fn scaled_percent_rounds_up() {
    // doc example: 5% of 10 nodes rounds up to 1 (rather than blocking all)
    assert_eq!(scaled_value_from_int_or_percent("5%", 10, true).unwrap(), 1);
    assert_eq!(scaled_value_from_int_or_percent("5%", 100, true).unwrap(), 5);
    assert_eq!(scaled_value_from_int_or_percent("10%", 100, true).unwrap(), 10);
    assert_eq!(scaled_value_from_int_or_percent("100%", 7, true).unwrap(), 7);
}

#[test]
fn scaled_percent_rounds_down_when_not_round_up() {
    assert_eq!(scaled_value_from_int_or_percent("5%", 10, false).unwrap(), 0);
    assert_eq!(scaled_value_from_int_or_percent("19%", 10, false).unwrap(), 1);
}

#[test]
fn scaled_zero_percent_is_zero() {
    assert_eq!(scaled_value_from_int_or_percent("0%", 50, true).unwrap(), 0);
}

#[test]
fn scaled_invalid_spec_errors() {
    assert!(scaled_value_from_int_or_percent("abc", 10, true).is_err());
    assert!(scaled_value_from_int_or_percent("abc%", 10, true).is_err());
    assert!(scaled_value_from_int_or_percent("-5%", 10, true).is_err());
}

// ---- Budget.GetAllowedDisruptions (no-schedule => active) --------------------

#[test]
fn budget_no_schedule_is_active_and_scales() {
    assert_eq!(budget_allowed_disruptions(&budget("3", &[]), 100).unwrap(), 3);
    // 20% of 100 nodes = 20 (reasons are ignored by the single-budget calc)
    assert_eq!(budget_allowed_disruptions(&budget("20%", &[]), 100).unwrap(), 20);
}

#[test]
fn budget_invalid_nodes_spec_errors() {
    assert!(budget_allowed_disruptions(&budget("not-a-number", &[]), 10).is_err());
}

// ---- NodePool.GetAllowedDisruptionsByReason ----------------------------------

#[test]
fn by_reason_no_budgets_is_unbounded() {
    assert_eq!(
        nodepool_allowed_disruptions_by_reason(&[], 10, "Underutilized").unwrap(),
        UNBOUNDED_DISRUPTIONS
    );
}

#[test]
fn by_reason_takes_min_across_matching_budgets() {
    let budgets = vec![budget("10%", &["Underutilized"]), budget("2", &["Underutilized"])];
    // 10% of 100 = 10 vs flat 2 → min = 2
    assert_eq!(
        nodepool_allowed_disruptions_by_reason(&budgets, 100, "Underutilized").unwrap(),
        2
    );
}

#[test]
fn by_reason_empty_reasons_applies_to_all_reasons() {
    // budget with no reasons applies to every reason
    let budgets = vec![budget("1", &[])];
    assert_eq!(
        nodepool_allowed_disruptions_by_reason(&budgets, 100, "Drifted").unwrap(),
        1
    );
}

#[test]
fn by_reason_ignores_budget_for_other_reason() {
    // this budget only constrains "Empty"; querying "Drifted" must ignore it
    let budgets = vec![budget("0", &["Empty"])];
    assert_eq!(
        nodepool_allowed_disruptions_by_reason(&budgets, 100, "Drifted").unwrap(),
        UNBOUNDED_DISRUPTIONS
    );
}

#[test]
fn by_reason_mixed_reasons_uses_only_relevant_minimum() {
    let budgets = vec![
        budget("0", &["Empty"]),         // irrelevant to Drifted
        budget("5", &["Drifted"]),       // relevant
        budget("50%", &[]),              // relevant (applies to all): 50% of 100 = 50
    ];
    // only 5 and 50 apply → min = 5
    assert_eq!(
        nodepool_allowed_disruptions_by_reason(&budgets, 100, "Drifted").unwrap(),
        5
    );
}

// ---- MustGetAllowedDisruptions (fail-closed) ---------------------------------

#[test]
fn must_get_returns_value_on_success() {
    let budgets = vec![budget("3", &[])];
    assert_eq!(must_get_allowed_disruptions(&budgets, 100, "Drifted"), 3);
}

#[test]
fn must_get_fails_closed_to_zero_on_error() {
    let budgets = vec![budget("garbage", &[])];
    assert_eq!(must_get_allowed_disruptions(&budgets, 100, "Drifted"), 0);
}
