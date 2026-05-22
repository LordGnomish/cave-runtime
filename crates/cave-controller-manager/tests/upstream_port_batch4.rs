// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Line-by-line ports of upstream kube-controller-manager tests — batch 4.
//!
//! Upstream: kubernetes/kubernetes @ v1.36.0
//! (tag commit `02d6d2a6157dd33cb6db3c68c4c6dcb15fd1b3f5`).
//!
//! Scope (closes three honest `status="missing"` entries documented in
//! `parity.manifest.toml` head of audit 2026-05-14):
//!
//!   * `pkg/controller/deployment/progress.go` + `progress_test.go`
//!     (rollout `Progressing` / `Available` / `ReplicaFailure`
//!      conditions + `progressDeadlineSeconds` timeout via
//!      `requeueStuckDeployment` / `DeploymentTimedOut`).
//!   * `pkg/controller/job/job_controller.go::pastActiveDeadline`
//!     (`activeDeadlineSeconds` timer wired into `reconcile`).
//!   * `pkg/controller/cronjob/utils.go::nextScheduleTime` +
//!     `mostRecentScheduleTime` (5-field cron parser replacing the
//!      previous `unimplemented!()` stub).
//!
//! Each `#[test]` carries an `Upstream:` doc-comment with the upstream
//! path + symbol it asserts on, per Charter v2 traceability.

use cave_controller_manager::cronjob::{CronJobSpec, ScheduleError, next_schedule_time};
use cave_controller_manager::deployment::{
    DeploymentSpec, DeploymentStatus, RolloutConditionStatus, RolloutConditionType, RolloutReason,
    Strategy, compute_conditions,
};
use cave_controller_manager::job::{
    JobSpec, JobStatus, past_active_deadline, reconcile_with_clock as job_reconcile,
};
use cave_controller_manager::types::{Reconcile, TenantId};
use chrono::{Duration, TimeZone, Utc};

fn tenant(s: &str) -> TenantId {
    TenantId::new(s).expect("valid tenant fixture")
}

fn dep_spec(replicas: u32, paused: bool, deadline: Option<i64>) -> DeploymentSpec {
    DeploymentSpec {
        name: "web".into(),
        namespace: "default".into(),
        replicas,
        strategy: Strategy::RollingUpdate {
            max_surge: 1,
            max_unavailable: 0,
        },
        paused,
        progress_deadline_seconds: deadline,
    }
}

fn cron(schedule: &str) -> CronJobSpec {
    use cave_controller_manager::cronjob::ConcurrencyPolicy;
    CronJobSpec {
        name: "report".into(),
        namespace: "default".into(),
        schedule: schedule.into(),
        concurrency: ConcurrencyPolicy::Allow,
        suspended: false,
    }
}

fn job_spec(active_deadline: Option<i64>, suspended: bool) -> JobSpec {
    JobSpec {
        name: "build".into(),
        namespace: "ci".into(),
        completions: 5,
        parallelism: 2,
        backoff_limit: 6,
        suspended,
        active_deadline_seconds: active_deadline,
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/controller/deployment/progress_test.go::TestRequeueStuckDeployment
// pkg/controller/deployment/progress.go::requeueStuckDeployment
// pkg/controller/deployment/util/deployment_util.go::DeploymentTimedOut
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: kubernetes/kubernetes@v1.36.0
///   pkg/controller/deployment/progress_test.go
///   TestRequeueStuckDeployment / `nil progressDeadlineSeconds specified`
///   — when `progressDeadlineSeconds` is unset, no Progressing condition
///     transitions and no timeout fires.
#[test]
fn upstream_deployment_progress_nil_deadline_emits_no_progressing_condition() {
    let spec = dep_spec(3, false, None);
    let status = DeploymentStatus {
        observed_replicas: 2,
        ready_replicas: 2,
        updated_replicas: 2,
        available_replicas: 2,
    };
    let now = Utc.with_ymd_and_hms(2026, 5, 14, 12, 0, 0).unwrap();
    let conds = compute_conditions(&spec, &status, now, None);
    assert!(
        conds
            .iter()
            .all(|c| c.kind != RolloutConditionType::Progressing),
        "no Progressing condition when progressDeadlineSeconds is nil"
    );
    assert!(
        conds
            .iter()
            .all(|c| c.reason != RolloutReason::ProgressDeadlineExceeded),
        "no TimedOut reason without a deadline"
    );
}

/// Upstream: pkg/controller/deployment/progress_test.go::TestRequeueStuckDeployment
///   / `stuck deployment - 30s` — deadline=60s, last progress 30s ago,
///   not yet timed out → Progressing condition still ConditionTrue with
///   reason ReplicaSetUpdated.
#[test]
fn upstream_deployment_progress_stuck_30s_still_progressing() {
    let spec = dep_spec(3, false, Some(60));
    let status = DeploymentStatus {
        observed_replicas: 3,
        ready_replicas: 2,
        updated_replicas: 2,
        available_replicas: 2,
    };
    let now = Utc.with_ymd_and_hms(2026, 5, 14, 18, 49, 30).unwrap();
    let last_progress = Utc.with_ymd_and_hms(2026, 5, 14, 18, 49, 0).unwrap();
    let conds = compute_conditions(&spec, &status, now, Some(last_progress));
    let progressing = conds
        .iter()
        .find(|c| c.kind == RolloutConditionType::Progressing)
        .expect("Progressing condition present when deadline is set");
    assert_eq!(progressing.status, RolloutConditionStatus::True);
    assert_eq!(progressing.reason, RolloutReason::ReplicaSetUpdated);
}

/// Upstream: pkg/controller/deployment/progress_test.go::TestRequeueStuckDeployment
///   / `failed deployment - 1s after deadline` — deadline=60s, last
///   progress was 61s ago → Progressing condition flips to False with
///   reason ProgressDeadlineExceeded.
#[test]
fn upstream_deployment_progress_one_second_past_deadline_times_out() {
    let spec = dep_spec(3, false, Some(60));
    let status = DeploymentStatus {
        observed_replicas: 3,
        ready_replicas: 2,
        updated_replicas: 2,
        available_replicas: 2,
    };
    let now = Utc.with_ymd_and_hms(2026, 5, 14, 18, 50, 1).unwrap();
    let last_progress = Utc.with_ymd_and_hms(2026, 5, 14, 18, 49, 0).unwrap();
    let conds = compute_conditions(&spec, &status, now, Some(last_progress));
    let progressing = conds
        .iter()
        .find(|c| c.kind == RolloutConditionType::Progressing)
        .expect("Progressing condition present");
    assert_eq!(progressing.status, RolloutConditionStatus::False);
    assert_eq!(progressing.reason, RolloutReason::ProgressDeadlineExceeded);
}

/// Upstream: pkg/controller/deployment/progress_test.go::TestSyncRolloutStatus
///   / `Single active ReplicaSet only` — replicas==updated==available,
///   reason = NewReplicaSetAvailable, status=True.
#[test]
fn upstream_deployment_progress_complete_emits_new_rs_available() {
    let spec = dep_spec(3, false, Some(60));
    let status = DeploymentStatus {
        observed_replicas: 3,
        ready_replicas: 3,
        updated_replicas: 3,
        available_replicas: 3,
    };
    let now = Utc.with_ymd_and_hms(2026, 5, 14, 12, 0, 0).unwrap();
    let conds = compute_conditions(&spec, &status, now, Some(now - Duration::seconds(10)));
    let progressing = conds
        .iter()
        .find(|c| c.kind == RolloutConditionType::Progressing)
        .expect("Progressing condition present");
    assert_eq!(progressing.status, RolloutConditionStatus::True);
    assert_eq!(progressing.reason, RolloutReason::NewReplicaSetAvailable);
}

/// Upstream: pkg/controller/deployment/util/deployment_util.go
///   MinimumReplicasAvailable / MinimumReplicasUnavailable — Available
///   condition is True when `available_replicas >= replicas -
///   max_unavailable`, otherwise False.
#[test]
fn upstream_deployment_available_condition_tracks_minimum_replicas() {
    let spec = DeploymentSpec {
        name: "web".into(),
        namespace: "default".into(),
        replicas: 5,
        strategy: Strategy::RollingUpdate {
            max_surge: 0,
            max_unavailable: 1,
        },
        paused: false,
        progress_deadline_seconds: Some(600),
    };
    // 4 available out of 5 with maxUnavailable=1 → still True.
    let ok = DeploymentStatus {
        observed_replicas: 5,
        ready_replicas: 4,
        updated_replicas: 5,
        available_replicas: 4,
    };
    let now = Utc.with_ymd_and_hms(2026, 5, 14, 12, 0, 0).unwrap();
    let conds = compute_conditions(&spec, &ok, now, Some(now));
    let avail = conds
        .iter()
        .find(|c| c.kind == RolloutConditionType::Available)
        .expect("Available condition present");
    assert_eq!(avail.status, RolloutConditionStatus::True);
    assert_eq!(avail.reason, RolloutReason::MinimumReplicasAvailable);
    // 3 available out of 5 with maxUnavailable=1 → False / MinimumReplicasUnavailable.
    let bad = DeploymentStatus {
        observed_replicas: 5,
        ready_replicas: 3,
        updated_replicas: 5,
        available_replicas: 3,
    };
    let conds = compute_conditions(&spec, &bad, now, Some(now));
    let avail = conds
        .iter()
        .find(|c| c.kind == RolloutConditionType::Available)
        .expect("Available condition present");
    assert_eq!(avail.status, RolloutConditionStatus::False);
    assert_eq!(avail.reason, RolloutReason::MinimumReplicasUnavailable);
}

/// Upstream: pkg/controller/deployment/util/deployment_util.go
///   PausedDeployReason — when spec.paused is true and a deadline is
///   set, Progressing carries reason DeploymentPaused.
#[test]
fn upstream_deployment_paused_emits_paused_reason() {
    let spec = dep_spec(3, /*paused=*/ true, Some(60));
    let status = DeploymentStatus {
        observed_replicas: 3,
        ready_replicas: 0,
        updated_replicas: 0,
        available_replicas: 0,
    };
    let now = Utc.with_ymd_and_hms(2026, 5, 14, 12, 0, 0).unwrap();
    let conds = compute_conditions(&spec, &status, now, Some(now));
    let progressing = conds
        .iter()
        .find(|c| c.kind == RolloutConditionType::Progressing)
        .expect("Progressing condition present");
    assert_eq!(progressing.reason, RolloutReason::DeploymentPaused);
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/controller/job/job_controller.go::pastActiveDeadline
// pkg/controller/job/job_controller_test.go::TestSyncJobPastDeadline
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: job_controller_test.go::TestSyncJobPastDeadline
///   / `activeDeadlineSeconds bigger than single pod execution` —
///   activeDeadlineSeconds=10, startTime=15s ago → deadline exceeded.
#[test]
fn upstream_job_active_deadline_seconds_past_when_started_long_ago() {
    let spec = job_spec(Some(10), /*suspended=*/ false);
    let status = JobStatus {
        active: 1,
        succeeded: 0,
        failed: 0,
        start_time: Some(Utc.with_ymd_and_hms(2026, 5, 14, 12, 0, 0).unwrap()),
    };
    let now = Utc.with_ymd_and_hms(2026, 5, 14, 12, 0, 15).unwrap();
    assert!(
        past_active_deadline(&spec, &status, now),
        "15s elapsed > 10s deadline → past"
    );
}

/// Upstream: job_controller.go::pastActiveDeadline — returns false when
/// `ActiveDeadlineSeconds == nil` regardless of elapsed time.
#[test]
fn upstream_job_active_deadline_nil_never_expires() {
    let spec = job_spec(None, false);
    let status = JobStatus {
        active: 1,
        succeeded: 0,
        failed: 0,
        start_time: Some(Utc.with_ymd_and_hms(2026, 5, 14, 12, 0, 0).unwrap()),
    };
    let now = Utc.with_ymd_and_hms(2026, 5, 14, 13, 0, 0).unwrap();
    assert!(!past_active_deadline(&spec, &status, now));
}

/// Upstream: job_controller.go::pastActiveDeadline — returns false when
/// `Status.StartTime == nil` (pod has not started yet).
#[test]
fn upstream_job_active_deadline_no_start_time_never_expires() {
    let spec = job_spec(Some(1), false);
    let status = JobStatus {
        active: 0,
        succeeded: 0,
        failed: 0,
        start_time: None,
    };
    let now = Utc.with_ymd_and_hms(2026, 5, 14, 12, 1, 0).unwrap();
    assert!(!past_active_deadline(&spec, &status, now));
}

/// Upstream: job_controller_test.go::TestSyncJobPastDeadline
///   / `activeDeadlineSeconds is not triggered when Job is suspended` —
///   suspended jobs skip the deadline check.
#[test]
fn upstream_job_active_deadline_suspended_does_not_expire() {
    let spec = job_spec(Some(10), /*suspended=*/ true);
    let status = JobStatus {
        active: 0,
        succeeded: 0,
        failed: 0,
        start_time: Some(Utc.with_ymd_and_hms(2026, 5, 14, 12, 0, 0).unwrap()),
    };
    let now = Utc.with_ymd_and_hms(2026, 5, 14, 12, 5, 0).unwrap();
    assert!(!past_active_deadline(&spec, &status, now));
}

/// Upstream: job_controller_test.go::TestSyncJobPastDeadline — the
/// reconciler returns `Reconcile::Delete(active)` once deadline expired
/// (controller drains active pods and emits JobReasonDeadlineExceeded).
#[test]
fn upstream_job_reconcile_deletes_active_pods_past_deadline() {
    let spec = job_spec(Some(10), false);
    let mut status = JobStatus {
        active: 2,
        succeeded: 0,
        failed: 0,
        start_time: Some(Utc.with_ymd_and_hms(2026, 5, 14, 12, 0, 0).unwrap()),
    };
    let now = Utc.with_ymd_and_hms(2026, 5, 14, 12, 0, 20).unwrap();
    // Past the deadline → drain active pods.
    let r = job_reconcile(&spec, &status, &tenant("acme"), now).unwrap();
    assert_eq!(r, Reconcile::Delete(2));
    // No active pods means no work even past deadline.
    status.active = 0;
    let r = job_reconcile(&spec, &status, &tenant("acme"), now).unwrap();
    assert_eq!(r, Reconcile::NoOp);
}

/// Upstream: job_controller.go::pastActiveDeadline — boundary case:
/// elapsed == ActiveDeadlineSeconds is past (`duration >= allowed`).
#[test]
fn upstream_job_active_deadline_boundary_inclusive_at_exact_seconds() {
    let spec = job_spec(Some(10), false);
    let status = JobStatus {
        active: 1,
        succeeded: 0,
        failed: 0,
        start_time: Some(Utc.with_ymd_and_hms(2026, 5, 14, 12, 0, 0).unwrap()),
    };
    let now = Utc.with_ymd_and_hms(2026, 5, 14, 12, 0, 10).unwrap();
    assert!(
        past_active_deadline(&spec, &status, now),
        "duration >= allowed includes the equality case"
    );
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/controller/cronjob/utils.go::nextScheduleTime
// pkg/controller/cronjob/utils_test.go::TestNextScheduleTime
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: utils_test.go::TestNextScheduleTime case 2 — `0 * * * ?`,
/// no last schedule, now slightly past the hour → most recent schedule
/// is the top of the hour.
#[test]
fn upstream_cronjob_next_schedule_top_of_hour_when_just_past() {
    let spec = cron("0 * * * *");
    let now = Utc.with_ymd_and_hms(2026, 5, 14, 12, 0, 2).unwrap();
    let next = next_schedule_time(&spec.schedule, None, now).unwrap();
    assert_eq!(
        next,
        Some(Utc.with_ymd_and_hms(2026, 5, 14, 12, 0, 0).unwrap())
    );
}

/// Upstream: utils_test.go::TestMostRecentScheduleTime case 1 — `0 * * * *`,
/// no last schedule, now is 30s after the top → no past schedule yet
/// (the cron is set to fire AT the top, and we have NOT crossed it
/// since the cronjob has no observed last_schedule).
#[test]
fn upstream_cronjob_next_schedule_returns_none_when_no_fire_passed() {
    let spec = cron("0 * * * *");
    // Now is 30s before the next top of the hour, last fire was 12:00:00,
    // so no new fire is due yet.
    let last = Utc.with_ymd_and_hms(2026, 5, 14, 12, 0, 0).unwrap();
    let now = Utc.with_ymd_and_hms(2026, 5, 14, 12, 0, 30).unwrap();
    let next = next_schedule_time(&spec.schedule, Some(last), now).unwrap();
    assert_eq!(next, None);
}

/// Upstream: utils_test.go::TestNextScheduleTime case 8 — `59 23 31 2 *`
/// (Feb 31 — impossible date) → cron parse error.
#[test]
fn upstream_cronjob_invalid_day_of_month_returns_error() {
    let spec = cron("59 23 31 2 *");
    let now = Utc.with_ymd_and_hms(2026, 5, 14, 12, 0, 0).unwrap();
    let err = next_schedule_time(&spec.schedule, None, now);
    assert!(matches!(err, Err(ScheduleError::Unsatisfiable { .. })));
}

/// Upstream: utils_test.go::TestNextScheduleTime — `*/5 * * * *` every
/// 5 minutes; the most recent fire on/before `now` is at xx:05:00.
#[test]
fn upstream_cronjob_step_field_returns_most_recent_multiple() {
    let spec = cron("*/5 * * * *");
    let now = Utc.with_ymd_and_hms(2026, 5, 14, 12, 7, 30).unwrap();
    let next = next_schedule_time(&spec.schedule, None, now).unwrap();
    assert_eq!(
        next,
        Some(Utc.with_ymd_and_hms(2026, 5, 14, 12, 5, 0).unwrap())
    );
}

/// Upstream: utils_test.go — `30 10,11,12 * * 1-5` list of hours,
/// restricted to weekdays. now = Mon 12:30 → most recent fire is
/// today at 12:30.
#[test]
fn upstream_cronjob_list_and_range_resolves_within_day() {
    let spec = cron("30 10,11,12 * * 1-5");
    // 2026-05-11 is a Monday; 12:30 fires on the list.
    let now = Utc.with_ymd_and_hms(2026, 5, 11, 12, 35, 0).unwrap();
    let next = next_schedule_time(&spec.schedule, None, now).unwrap();
    assert_eq!(
        next,
        Some(Utc.with_ymd_and_hms(2026, 5, 11, 12, 30, 0).unwrap())
    );
}

/// Upstream: cronjob_controllerv2_test.go::TestSyncCronJob — when the
/// schedule has not fired since `last_schedule_time`, the controller
/// stays NoOp (no new Job spawned).
#[test]
fn upstream_cronjob_does_not_fire_when_no_schedule_passed() {
    let spec = cron("0 0 * * *"); // midnight daily
    let last = Utc.with_ymd_and_hms(2026, 5, 14, 0, 0, 0).unwrap();
    let now = Utc.with_ymd_and_hms(2026, 5, 14, 12, 0, 0).unwrap();
    let next = next_schedule_time(&spec.schedule, Some(last), now).unwrap();
    assert_eq!(next, None);
}

/// Upstream: utils.go — five-field cron with whitespace tolerance —
/// extra spaces and tabs are valid separators.
#[test]
fn upstream_cronjob_whitespace_tolerant_parsing() {
    let now = Utc.with_ymd_and_hms(2026, 5, 14, 12, 7, 30).unwrap();
    let next = next_schedule_time("*/5  *   *  *  *", None, now).unwrap();
    assert_eq!(
        next,
        Some(Utc.with_ymd_and_hms(2026, 5, 14, 12, 5, 0).unwrap())
    );
}

/// Upstream: utils.go — non-numeric / unparseable field → InvalidField.
#[test]
fn upstream_cronjob_unparseable_field_returns_error() {
    let now = Utc.with_ymd_and_hms(2026, 5, 14, 12, 0, 0).unwrap();
    let err = next_schedule_time("xx * * * *", None, now);
    assert!(matches!(err, Err(ScheduleError::InvalidField { .. })));
}
