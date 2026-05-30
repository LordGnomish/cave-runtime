// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD: Kyverno CleanupPolicy schedule + selection logic.
//!
//! Upstream: kyverno/kyverno v1.18.1 —
//!   - api/kyverno/v2/cleanup_policy_types.go (CleanupPolicy CRD)
//!   - pkg/controllers/cleanup/controller.go (cron schedule + resource select)
//!
//! The Phase-2 controller reconciler (k8s list/delete loop) stays scope_cut;
//! the *pure* schedule parsing + "is this resource a cleanup candidate"
//! selection logic is portable and lives in src/kyverno/cleanup.rs.

use cave_policy::kyverno::cleanup::{selects_for_cleanup, CronSchedule};
use cave_policy::kyverno::models::{
    CleanupPolicy, CleanupPolicySpec, Condition, ConditionOperator, Conditions, MatchResources,
    ObjectMeta, ResourceDescription,
};
use chrono::{TimeZone, Utc};
use serde_json::json;

// ─── Cron schedule ──────────────────────────────────────────────────────────

#[test]
fn test_cron_step_minute_matches() {
    let sched = CronSchedule::parse("*/5 * * * *").expect("parse");
    let at_05 = Utc.with_ymd_and_hms(2026, 5, 30, 10, 5, 0).unwrap();
    let at_10 = Utc.with_ymd_and_hms(2026, 5, 30, 10, 10, 0).unwrap();
    let at_03 = Utc.with_ymd_and_hms(2026, 5, 30, 10, 3, 0).unwrap();
    assert!(sched.matches(&at_05), "minute 5 matches */5");
    assert!(sched.matches(&at_10), "minute 10 matches */5");
    assert!(!sched.matches(&at_03), "minute 3 does not match */5");
}

#[test]
fn test_cron_midnight_only() {
    let sched = CronSchedule::parse("0 0 * * *").expect("parse");
    let midnight = Utc.with_ymd_and_hms(2026, 5, 30, 0, 0, 0).unwrap();
    let noon = Utc.with_ymd_and_hms(2026, 5, 30, 12, 0, 0).unwrap();
    assert!(sched.matches(&midnight), "00:00 matches '0 0 * * *'");
    assert!(!sched.matches(&noon), "12:00 does not match '0 0 * * *'");
}

#[test]
fn test_cron_list_and_range() {
    // minutes 0,15,30,45 ; hours 9-17
    let sched = CronSchedule::parse("0,15,30,45 9-17 * * *").expect("parse");
    assert!(sched.matches(&Utc.with_ymd_and_hms(2026, 5, 30, 9, 15, 0).unwrap()));
    assert!(sched.matches(&Utc.with_ymd_and_hms(2026, 5, 30, 17, 45, 0).unwrap()));
    assert!(!sched.matches(&Utc.with_ymd_and_hms(2026, 5, 30, 8, 15, 0).unwrap()));
    assert!(!sched.matches(&Utc.with_ymd_and_hms(2026, 5, 30, 9, 20, 0).unwrap()));
}

#[test]
fn test_cron_day_of_week() {
    // Sundays only (0). 2026-05-31 is a Sunday; 2026-05-30 is a Saturday.
    let sched = CronSchedule::parse("0 0 * * 0").expect("parse");
    let sunday = Utc.with_ymd_and_hms(2026, 5, 31, 0, 0, 0).unwrap();
    let saturday = Utc.with_ymd_and_hms(2026, 5, 30, 0, 0, 0).unwrap();
    assert!(sched.matches(&sunday), "Sunday matches dow=0");
    assert!(!sched.matches(&saturday), "Saturday does not match dow=0");
}

#[test]
fn test_cron_next_after() {
    let sched = CronSchedule::parse("*/15 * * * *").expect("parse");
    let from = Utc.with_ymd_and_hms(2026, 5, 30, 10, 7, 30).unwrap();
    let next = sched.next_after(&from).expect("next run exists");
    assert_eq!(next, Utc.with_ymd_and_hms(2026, 5, 30, 10, 15, 0).unwrap());
}

#[test]
fn test_cron_invalid_field_count() {
    assert!(CronSchedule::parse("* * *").is_err(), "3 fields is invalid");
    assert!(CronSchedule::parse("60 * * * *").is_err(), "minute 60 out of range");
}

// ─── Resource selection ───────────────────────────────────────────────────────

fn cleanup_policy(conditions: Option<Conditions>) -> CleanupPolicy {
    CleanupPolicy {
        api_version: "kyverno.io/v2".into(),
        kind: "ClusterCleanupPolicy".into(),
        metadata: ObjectMeta {
            name: "stale-jobs".into(),
            ..Default::default()
        },
        spec: CleanupPolicySpec {
            schedule: "0 * * * *".into(),
            match_resources: MatchResources {
                resources: Some(ResourceDescription {
                    kinds: vec!["Job".into()],
                    namespaces: vec!["batch".into()],
                    ..Default::default()
                }),
                ..Default::default()
            },
            exclude: None,
            conditions,
        },
        status: None,
    }
}

fn job(namespace: &str, succeeded: i64) -> serde_json::Value {
    json!({
        "apiVersion": "batch/v1",
        "kind": "Job",
        "metadata": {"name": "j1", "namespace": namespace},
        "status": {"succeeded": succeeded}
    })
}

#[test]
fn test_select_matches_kind_and_namespace() {
    let policy = cleanup_policy(None);
    let ctx = json!({"request": {"object": job("batch", 1)}});
    assert!(
        selects_for_cleanup(&policy, &job("batch", 1), Some("batch"), &ctx),
        "Job in batch ns is a cleanup candidate"
    );
}

#[test]
fn test_select_rejects_wrong_namespace() {
    let policy = cleanup_policy(None);
    let res = json!({
        "apiVersion": "batch/v1", "kind": "Job",
        "metadata": {"name": "j1", "namespace": "default"}, "status": {"succeeded": 1}
    });
    let ctx = json!({"request": {"object": res}});
    assert!(
        !selects_for_cleanup(&policy, &res, Some("default"), &ctx),
        "Job outside batch ns is NOT a candidate"
    );
}

#[test]
fn test_select_honors_conditions() {
    // Only clean up Jobs that have succeeded (status.succeeded == 1).
    let conditions = Conditions {
        all: Some(vec![Condition {
            key: json!("{{ request.object.status.succeeded }}"),
            operator: ConditionOperator::Equals,
            value: Some(json!(1)),
            message: None,
        }]),
        any: None,
    };
    let policy = cleanup_policy(Some(conditions));

    let done = job("batch", 1);
    let running = job("batch", 0);
    assert!(
        selects_for_cleanup(&policy, &done, Some("batch"), &json!({"request": {"object": done}})),
        "succeeded job is a candidate"
    );
    assert!(
        !selects_for_cleanup(
            &policy,
            &running,
            Some("batch"),
            &json!({"request": {"object": running}})
        ),
        "running job (succeeded=0) is NOT a candidate"
    );
}
