// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Failing tests for experiment scheduling logic.
//! The schedule module is NEW (not in origin/main).

use cave_chaos::schedule::{
    cron_field_matches, is_cron_due, next_cron_run, validate_cron_expression, CronField,
    ScheduledRunDecision,
};
use cave_chaos::models::ExperimentSchedule;
use uuid::Uuid;
use chrono::{TimeZone, Utc};

// ─── cron field parsing ───────────────────────────────────────────────────────

#[test]
fn cron_field_wildcard_matches_any() {
    let f = CronField::Wildcard;
    for v in 0u32..=59 {
        assert!(cron_field_matches(&f, v));
    }
}

#[test]
fn cron_field_exact_matches_only_value() {
    let f = CronField::Exact(15);
    assert!(cron_field_matches(&f, 15));
    assert!(!cron_field_matches(&f, 14));
    assert!(!cron_field_matches(&f, 16));
}

#[test]
fn cron_field_step_matches_multiples() {
    // */5 in minute field matches 0, 5, 10, 15, ...
    let f = CronField::Step(5);
    assert!(cron_field_matches(&f, 0));
    assert!(cron_field_matches(&f, 5));
    assert!(cron_field_matches(&f, 10));
    assert!(!cron_field_matches(&f, 3));
    assert!(!cron_field_matches(&f, 7));
}

#[test]
fn cron_field_range_matches_inclusive() {
    let f = CronField::Range(10, 20);
    assert!(cron_field_matches(&f, 10));
    assert!(cron_field_matches(&f, 15));
    assert!(cron_field_matches(&f, 20));
    assert!(!cron_field_matches(&f, 9));
    assert!(!cron_field_matches(&f, 21));
}

#[test]
fn cron_field_list_matches_any_in_set() {
    let f = CronField::List(vec![1, 3, 5, 7]);
    assert!(cron_field_matches(&f, 1));
    assert!(cron_field_matches(&f, 7));
    assert!(!cron_field_matches(&f, 2));
    assert!(!cron_field_matches(&f, 6));
}

// ─── validate_cron_expression ─────────────────────────────────────────────────

#[test]
fn validate_cron_expression_standard_5_fields() {
    assert!(validate_cron_expression("0 2 * * 1").is_ok());
    assert!(validate_cron_expression("*/5 * * * *").is_ok());
    assert!(validate_cron_expression("0 0 1 1 *").is_ok());
}

#[test]
fn validate_cron_expression_rejects_bad_formats() {
    assert!(validate_cron_expression("").is_err());
    assert!(validate_cron_expression("bad").is_err());
    assert!(validate_cron_expression("* * * *").is_err()); // only 4 fields
    assert!(validate_cron_expression("60 * * * *").is_err()); // minute out of range
    assert!(validate_cron_expression("* 25 * * *").is_err()); // hour out of range
}

// ─── is_cron_due ──────────────────────────────────────────────────────────────

#[test]
fn is_cron_due_at_matching_time() {
    // "0 2 * * 1" = every Monday at 02:00
    // 2026-06-01 is a Monday at 02:00 UTC
    let monday_2am = Utc.with_ymd_and_hms(2026, 6, 1, 2, 0, 0).unwrap();
    assert!(is_cron_due("0 2 * * 1", &monday_2am).unwrap());
}

#[test]
fn is_cron_due_at_non_matching_time() {
    // 2026-06-01 is a Monday — but 03:00 doesn't match "0 2 * * 1"
    let monday_3am = Utc.with_ymd_and_hms(2026, 6, 1, 3, 0, 0).unwrap();
    assert!(!is_cron_due("0 2 * * 1", &monday_3am).unwrap());
}

#[test]
fn is_cron_due_every_minute() {
    let any_time = Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap();
    assert!(is_cron_due("* * * * *", &any_time).unwrap());
}

// ─── next_cron_run ────────────────────────────────────────────────────────────

#[test]
fn next_cron_run_returns_future_time() {
    let now = Utc::now();
    let next = next_cron_run("0 * * * *", &now).unwrap(); // hourly
    assert!(next > now, "next run must be in the future");
}

#[test]
fn next_cron_run_daily_midnight() {
    let morning = Utc.with_ymd_and_hms(2026, 6, 1, 8, 0, 0).unwrap();
    let next = next_cron_run("0 0 * * *", &morning).unwrap();
    // Should be the NEXT midnight after 08:00 on 2026-06-01
    assert!(next > morning);
    // Should be within 24 hours
    let diff_hours = (next - morning).num_hours();
    assert!(diff_hours <= 24, "next midnight should be within 24h (got {}h)", diff_hours);
}

// ─── ScheduledRunDecision ─────────────────────────────────────────────────────

#[test]
fn schedule_decision_run_when_enabled_and_due() {
    let sched = ExperimentSchedule {
        id: Uuid::new_v4(),
        experiment_id: Uuid::new_v4(),
        cron_expression: "* * * * *".into(), // every minute
        enabled: true,
        last_run: None,
        next_run: None,
        max_runs: None,
        run_count: 0,
    };
    let now = Utc::now();
    let decision = cave_chaos::schedule::should_run(&sched, &now);
    assert_eq!(decision, ScheduledRunDecision::Run);
}

#[test]
fn schedule_decision_skip_when_disabled() {
    let sched = ExperimentSchedule {
        id: Uuid::new_v4(),
        experiment_id: Uuid::new_v4(),
        cron_expression: "* * * * *".into(),
        enabled: false,
        last_run: None,
        next_run: None,
        max_runs: None,
        run_count: 0,
    };
    let now = Utc::now();
    let decision = cave_chaos::schedule::should_run(&sched, &now);
    assert_eq!(decision, ScheduledRunDecision::Skip);
}

#[test]
fn schedule_decision_exhausted_when_max_runs_reached() {
    let sched = ExperimentSchedule {
        id: Uuid::new_v4(),
        experiment_id: Uuid::new_v4(),
        cron_expression: "* * * * *".into(),
        enabled: true,
        last_run: None,
        next_run: None,
        max_runs: Some(5),
        run_count: 5, // already at max
    };
    let now = Utc::now();
    let decision = cave_chaos::schedule::should_run(&sched, &now);
    assert_eq!(decision, ScheduledRunDecision::Exhausted);
}

#[test]
fn schedule_decision_skip_when_already_ran_this_minute() {
    let now = Utc::now();
    let sched = ExperimentSchedule {
        id: Uuid::new_v4(),
        experiment_id: Uuid::new_v4(),
        cron_expression: "* * * * *".into(),
        enabled: true,
        last_run: Some(now), // ran at exact same minute
        next_run: None,
        max_runs: None,
        run_count: 1,
    };
    let decision = cave_chaos::schedule::should_run(&sched, &now);
    // Same minute → already ran this tick → Skip
    assert_eq!(decision, ScheduledRunDecision::Skip);
}
