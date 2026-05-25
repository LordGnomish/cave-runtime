// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Edge coverage for cave-controller-manager — types, cronjob, replicaset
//! adoption, reconcile decisions.

use cave_controller_manager::cronjob::{
    next_fire_time, next_schedule_time, validate_schedule, ConcurrencyPolicy, CronJobSpec,
    CronJobStatus, ScheduleError,
};
use cave_controller_manager::replicaset::{
    adopt_orphans, clamp_burst, AdoptionPlan, PodView, ReplicaSetSpec, ReplicaSetStatus,
};
use cave_controller_manager::types::{Cite, ControllerError, Reconcile, TenantId, UPSTREAM_VERSION};
use chrono::{TimeZone, Utc};

fn tenant() -> TenantId {
    TenantId::new("test-tenant").unwrap()
}

// ---------------------------------------------------------------------------
// types
// ---------------------------------------------------------------------------

#[test]
fn upstream_version_is_pinned() {
    assert!(UPSTREAM_VERSION.starts_with("v1."));
    assert!(UPSTREAM_VERSION.contains('.'));
}

#[test]
fn cite_new_uses_default_version_and_url_contains_path() {
    let c = Cite::new("pkg/controller/x.go", "Foo");
    assert_eq!(c.version, UPSTREAM_VERSION);
    let url = c.url();
    assert!(url.contains("github.com"));
    assert!(url.contains(UPSTREAM_VERSION));
    assert!(url.contains("pkg/controller/x.go"));
}

#[test]
fn cite_display_format_path_symbol_version() {
    let c = Cite::new("pkg/x.go", "Bar");
    let s = format!("{}", c);
    assert!(s.contains("pkg/x.go"));
    assert!(s.contains("Bar"));
    assert!(s.contains(UPSTREAM_VERSION));
}

#[test]
fn reconcile_variants_serialize_round_trip() {
    for r in [
        Reconcile::NoOp,
        Reconcile::Create(3),
        Reconcile::Delete(1),
        Reconcile::Update(2),
        Reconcile::Requeue,
    ] {
        let j = serde_json::to_string(&r).unwrap();
        let back: Reconcile = serde_json::from_str(&j).unwrap();
        assert_eq!(r, back);
    }
}

#[test]
fn controller_error_display_includes_tenant_and_kind() {
    let e = ControllerError::TenantDenied {
        tenant: tenant(),
        kind: "Pod",
        name: "x".into(),
    };
    let s = e.to_string();
    assert!(s.contains("test-tenant"));
    assert!(s.contains("Pod"));
    assert!(s.contains("x"));
}

#[test]
fn tenant_id_rejects_empty() {
    assert!(TenantId::new("").is_err());
}

// ---------------------------------------------------------------------------
// CronJob
// ---------------------------------------------------------------------------

#[test]
fn cron_validate_schedule_5_fields_ok() {
    assert!(validate_schedule("* * * * *").is_ok());
    assert!(validate_schedule("0 0 1 1 *").is_ok());
}

#[test]
fn cron_validate_schedule_wrong_field_count_errors() {
    let err = validate_schedule("* * *").unwrap_err();
    assert!(matches!(err, ControllerError::InvalidSpec { .. }));
}

#[test]
fn cron_reconcile_suspended_is_noop() {
    let spec = CronJobSpec {
        name: "cj".into(),
        namespace: "ns".into(),
        schedule: "* * * * *".into(),
        concurrency: ConcurrencyPolicy::Allow,
        suspended: true,
    };
    let r = cave_controller_manager::cronjob::reconcile(&spec, &CronJobStatus::default(), &tenant())
        .unwrap();
    assert_eq!(r, Reconcile::NoOp);
}

#[test]
fn cron_reconcile_forbid_when_active_is_noop() {
    let spec = CronJobSpec {
        name: "cj".into(),
        namespace: "ns".into(),
        schedule: "* * * * *".into(),
        concurrency: ConcurrencyPolicy::Forbid,
        suspended: false,
    };
    let mut status = CronJobStatus::default();
    status.active_jobs = 1;
    let r = cave_controller_manager::cronjob::reconcile(&spec, &status, &tenant()).unwrap();
    assert_eq!(r, Reconcile::NoOp);
}

#[test]
fn cron_reconcile_replace_when_active_deletes_active_jobs() {
    let spec = CronJobSpec {
        name: "cj".into(),
        namespace: "ns".into(),
        schedule: "* * * * *".into(),
        concurrency: ConcurrencyPolicy::Replace,
        suspended: false,
    };
    let mut status = CronJobStatus::default();
    status.active_jobs = 2;
    let r = cave_controller_manager::cronjob::reconcile(&spec, &status, &tenant()).unwrap();
    assert_eq!(r, Reconcile::Delete(2));
}

#[test]
fn cron_reconcile_allow_creates_one() {
    let spec = CronJobSpec {
        name: "cj".into(),
        namespace: "ns".into(),
        schedule: "* * * * *".into(),
        concurrency: ConcurrencyPolicy::Allow,
        suspended: false,
    };
    let r = cave_controller_manager::cronjob::reconcile(&spec, &CronJobStatus::default(), &tenant())
        .unwrap();
    assert_eq!(r, Reconcile::Create(1));
}

#[test]
fn cron_reconcile_invalid_schedule_errors() {
    let spec = CronJobSpec {
        name: "cj".into(),
        namespace: "ns".into(),
        schedule: "not a cron".into(),
        concurrency: ConcurrencyPolicy::Allow,
        suspended: false,
    };
    let res = cave_controller_manager::cronjob::reconcile(&spec, &CronJobStatus::default(), &tenant());
    assert!(res.is_err());
}

#[test]
fn cron_next_schedule_wrong_field_count() {
    let now = Utc.with_ymd_and_hms(2026, 5, 19, 12, 0, 0).unwrap();
    let res = next_schedule_time("a b c", None, now);
    assert!(matches!(res, Err(ScheduleError::WrongFieldCount { got: 3 })));
}

#[test]
fn cron_next_schedule_finds_minute_match() {
    // Star schedule fires every minute → most-recent fire is "now".
    let now = Utc.with_ymd_and_hms(2026, 5, 19, 12, 30, 45).unwrap();
    let res = next_schedule_time("* * * * *", None, now).unwrap();
    let t = res.expect("must find a fire time");
    // Truncated to the minute boundary.
    assert_eq!(t.second(), 0);
    assert!(t <= now);
}

#[test]
fn cron_next_fire_time_advances() {
    let spec = CronJobSpec {
        name: "cj".into(),
        namespace: "ns".into(),
        schedule: "0 * * * *".into(), // every hour on the 0 minute
        concurrency: ConcurrencyPolicy::Allow,
        suspended: false,
    };
    let now = Utc.with_ymd_and_hms(2026, 5, 19, 12, 30, 0).unwrap();
    let next = next_fire_time(&spec, now).unwrap();
    assert!(next > now);
    assert_eq!(next.minute(), 0);
}

// ---------------------------------------------------------------------------
// ReplicaSet — reconcile / clamp_burst / adopt_orphans
// ---------------------------------------------------------------------------

use chrono::Timelike;

fn rs(name: &str, replicas: u32) -> ReplicaSetSpec {
    ReplicaSetSpec {
        name: name.into(),
        namespace: "ns".into(),
        replicas,
        selector: vec![("app".into(), name.into())],
    }
}

#[test]
fn rs_reconcile_noop_when_at_target() {
    let spec = rs("nginx", 3);
    let status = ReplicaSetStatus { running_pods: 3, failed_pods: 0 };
    let r = cave_controller_manager::replicaset::reconcile(&spec, &status, &tenant()).unwrap();
    assert_eq!(r, Reconcile::NoOp);
}

#[test]
fn rs_reconcile_scale_up_creates_diff() {
    let spec = rs("nginx", 5);
    let status = ReplicaSetStatus { running_pods: 2, failed_pods: 0 };
    let r = cave_controller_manager::replicaset::reconcile(&spec, &status, &tenant()).unwrap();
    assert_eq!(r, Reconcile::Create(3));
}

#[test]
fn rs_reconcile_scale_down_deletes_diff() {
    let spec = rs("nginx", 1);
    let status = ReplicaSetStatus { running_pods: 4, failed_pods: 0 };
    let r = cave_controller_manager::replicaset::reconcile(&spec, &status, &tenant()).unwrap();
    assert_eq!(r, Reconcile::Delete(3));
}

#[test]
fn rs_reconcile_empty_selector_errors() {
    let mut spec = rs("nginx", 1);
    spec.selector = vec![];
    let res = cave_controller_manager::replicaset::reconcile(&spec, &ReplicaSetStatus::default(), &tenant());
    assert!(matches!(res, Err(ControllerError::InvalidSpec { kind: "ReplicaSet", .. })));
}

#[test]
fn clamp_burst_caps_creations_and_deletions() {
    assert_eq!(clamp_burst(Reconcile::Create(100), 5), Reconcile::Create(5));
    assert_eq!(clamp_burst(Reconcile::Delete(100), 5), Reconcile::Delete(5));
    // Under cap is preserved
    assert_eq!(clamp_burst(Reconcile::Create(2), 5), Reconcile::Create(2));
    // Other variants untouched
    assert_eq!(clamp_burst(Reconcile::NoOp, 5), Reconcile::NoOp);
    assert_eq!(clamp_burst(Reconcile::Requeue, 5), Reconcile::Requeue);
}

fn orphan(name: &str, labels: Vec<(&str, &str)>) -> PodView {
    PodView {
        name: name.into(),
        namespace: "ns".into(),
        labels: labels.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        controller_ref: None,
    }
}

#[test]
fn adopt_orphans_claims_matching_unowned_pods() {
    let spec = rs("nginx", 3);
    let pods = vec![
        orphan("p1", vec![("app", "nginx")]),
        orphan("p2", vec![("app", "other")]),
    ];
    let plan = adopt_orphans(&spec, &pods, &tenant()).unwrap();
    assert_eq!(plan.claimed, vec!["p1".to_string()]);
    assert_eq!(plan.count(), 1);
}

#[test]
fn adopt_orphans_skips_pods_in_different_namespace() {
    let spec = rs("nginx", 3);
    let mut p = orphan("p1", vec![("app", "nginx")]);
    p.namespace = "other".into();
    let plan = adopt_orphans(&spec, &[p], &tenant()).unwrap();
    assert!(plan.claimed.is_empty());
}

#[test]
fn adopt_orphans_skips_owned_pods() {
    let spec = rs("nginx", 3);
    let mut p = orphan("p1", vec![("app", "nginx")]);
    p.controller_ref = Some("uid-existing-owner".into());
    let plan = adopt_orphans(&spec, &[p], &tenant()).unwrap();
    assert!(plan.claimed.is_empty(), "already-owned pod must not be re-adopted");
}

#[test]
fn adopt_orphans_requires_non_empty_selector() {
    let mut spec = rs("nginx", 3);
    spec.selector = vec![];
    let res = adopt_orphans(&spec, &[], &tenant());
    assert!(matches!(res, Err(ControllerError::InvalidSpec { .. })));
}

#[test]
fn adoption_plan_count_matches_vec_len() {
    let plan = AdoptionPlan { claimed: vec!["a".into(), "b".into(), "c".into()] };
    assert_eq!(plan.count(), 3);
}

#[test]
fn concurrency_policy_serde_round_trip() {
    for p in [ConcurrencyPolicy::Allow, ConcurrencyPolicy::Forbid, ConcurrencyPolicy::Replace] {
        let j = serde_json::to_string(&p).unwrap();
        let back: ConcurrencyPolicy = serde_json::from_str(&j).unwrap();
        assert_eq!(p, back);
    }
}
