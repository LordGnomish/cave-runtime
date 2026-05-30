// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// RED→GREEN cycle 9 (continuation ray #3): Budget.IsActive schedule window —
// port of (*Budget).IsActive from pkg/apis/v1/nodepool.go (v1.12.1, sha
// ed490e8), resolving the cron scope-cut left in cont2. Combines the cycle-8
// cron engine with the cycle-7 duration parser, threading a clock (Unix
// seconds, UTC) exactly as Go's clock.Clock does.
//
// Reference: 2026-01-01 00:00:00 UTC = 1767225600.

use cave_karpenter::budgets::{
    budget_allowed_disruptions_at, budget_is_active_at, BudgetError, UNBOUNDED_DISRUPTIONS,
};
use cave_karpenter::models::Budget;

const T0: i64 = 1_767_225_600; // 2026-01-01 00:00:00 UTC
const H: i64 = 3600;

fn scheduled(nodes: &str, schedule: &str, duration: &str) -> Budget {
    Budget {
        nodes: nodes.to_string(),
        schedule: Some(schedule.to_string()),
        duration: Some(duration.to_string()),
        reasons: vec![],
    }
}

fn unscheduled(nodes: &str) -> Budget {
    Budget {
        nodes: nodes.to_string(),
        schedule: None,
        duration: None,
        reasons: vec![],
    }
}

// ── IsActive: no schedule is always active ───────────────────────────────────

#[test]
fn no_schedule_is_always_active() {
    let b = unscheduled("3");
    assert!(budget_is_active_at(&b, T0).unwrap());
    assert!(budget_is_active_at(&b, T0 + 12345).unwrap());
}

// ── IsActive: daily 09:00 window of 8h ───────────────────────────────────────

#[test]
fn scheduled_active_inside_window() {
    // 09:00 daily, 8h window → active 09:00–17:00. Check at 10:00.
    let b = scheduled("3", "0 9 * * *", "8h");
    assert!(budget_is_active_at(&b, T0 + 10 * H).unwrap());
}

#[test]
fn scheduled_inactive_after_window() {
    // Check at 18:00 — past the 17:00 window end.
    let b = scheduled("3", "0 9 * * *", "8h");
    assert!(!budget_is_active_at(&b, T0 + 18 * H).unwrap());
}

#[test]
fn scheduled_inactive_before_first_hit() {
    // Check at 08:00 — before the 09:00 hit.
    let b = scheduled("3", "0 9 * * *", "8h");
    assert!(!budget_is_active_at(&b, T0 + 8 * H).unwrap());
}

// ── IsActive: invalid cron surfaces an error ─────────────────────────────────

#[test]
fn invalid_cron_schedule_is_error() {
    let b = scheduled("3", "not a cron", "8h");
    assert!(matches!(
        budget_is_active_at(&b, T0),
        Err(BudgetError::InvalidCron(_))
    ));
}

#[test]
fn invalid_duration_is_error() {
    let b = scheduled("3", "0 9 * * *", "8x");
    assert!(budget_is_active_at(&b, T0).is_err());
}

// ── GetAllowedDisruptions with a clock ───────────────────────────────────────

#[test]
fn allowed_disruptions_active_scales_nodes() {
    // active window → scale "3" against node count
    let b = scheduled("3", "0 9 * * *", "8h");
    assert_eq!(budget_allowed_disruptions_at(&b, 100, T0 + 10 * H).unwrap(), 3);
}

#[test]
fn allowed_disruptions_inactive_is_unbounded() {
    // outside window → the budget imposes no constraint
    let b = scheduled("3", "0 9 * * *", "8h");
    assert_eq!(
        budget_allowed_disruptions_at(&b, 100, T0 + 18 * H).unwrap(),
        UNBOUNDED_DISRUPTIONS
    );
}

#[test]
fn allowed_disruptions_active_percentage() {
    let b = scheduled("20%", "0 9 * * *", "8h");
    assert_eq!(
        budget_allowed_disruptions_at(&b, 100, T0 + 10 * H).unwrap(),
        20
    );
}
