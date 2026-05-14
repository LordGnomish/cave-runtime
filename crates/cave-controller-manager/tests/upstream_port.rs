//! Line-by-line ports of upstream kube-controller-manager tests,
//! cross-referenced from `parity.manifest.toml`'s `[[upstream_test]]`
//! block (batch3 — 2026-05-14).
//!
//! Upstream: kubernetes/kubernetes @ v1.36.0
//!   * pkg/controller/deployment/{deployment_controller,rolling,rollback,recreate}_test.go
//!   * pkg/controller/replicaset/replica_set_test.go + controller_ref_manager_test.go
//!   * pkg/controller/job/{job_controller,indexed_job_utils}_test.go
//!   * pkg/controller/cronjob/cronjob_controllerv2_test.go
//!   * pkg/controller/daemon/{daemon_controller,update}_test.go
//!
//! Subtests (Go `t.Run`) split into individual `#[test]` fns. Each
//! asserts the same input → output equivalence class the upstream
//! test asserts.

use cave_controller_manager::cronjob::{
    ConcurrencyPolicy, CronJobSpec, CronJobStatus, reconcile as cron_reconcile, validate_schedule,
};
use cave_controller_manager::daemonset::{
    DaemonSetSpec, DaemonSetStatus, NodeView, Taint, TaintEffect, Toleration, TolerationOperator,
    node_should_run, reconcile as ds_reconcile, tolerates, tolerates_all,
};
use cave_controller_manager::deployment::{
    DeploymentSpec, DeploymentStatus, RevisionHistory, Strategy, max_pods_during_surge,
    plan_rolling_step, reconcile as dep_reconcile, rollback,
};
use cave_controller_manager::job::{
    IndexState, IndexedJobStatus, JobSpec, JobStatus, index_status, is_complete, past_backoff,
    reconcile as job_reconcile,
};
use cave_controller_manager::replicaset::{
    AdoptionPlan, PodView, ReplicaSetSpec, ReplicaSetStatus, adopt_orphans, clamp_burst,
    reconcile as rs_reconcile, release_mismatched,
};
use cave_controller_manager::types::{ControllerError, Reconcile, TenantId};

fn tenant(s: &str) -> TenantId {
    TenantId::new(s).expect("valid tenant fixture")
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/controller/deployment/deployment_controller_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestSyncDeployment / `replicas_match_status_observed_is_noop`.
#[test]
fn upstream_deployment_steady_state_is_noop() {
    let spec = DeploymentSpec {
        name: "web".into(),
        namespace: "default".into(),
        replicas: 4,
        strategy: Strategy::RollingUpdate {
            max_surge: 1,
            max_unavailable: 0,
        },
        paused: false,
        progress_deadline_seconds: None,
    };
    let status = DeploymentStatus {
        observed_replicas: 4,
        ..Default::default()
    };
    assert_eq!(
        dep_reconcile(&spec, &status, &tenant("acme")).unwrap(),
        Reconcile::NoOp
    );
}

/// Upstream: TestSyncDeployment / `paused_freezes_rollout`.
#[test]
fn upstream_deployment_pause_freezes_rollout() {
    let spec = DeploymentSpec {
        name: "web".into(),
        namespace: "default".into(),
        replicas: 10,
        strategy: Strategy::RollingUpdate {
            max_surge: 2,
            max_unavailable: 0,
        },
        paused: true,
        progress_deadline_seconds: None,
    };
    let status = DeploymentStatus::default(); // 0 observed
    // Even with replicas=10 and observed=0, paused → NoOp.
    assert_eq!(
        dep_reconcile(&spec, &status, &tenant("acme")).unwrap(),
        Reconcile::NoOp
    );
}

/// Upstream: TestMaxSurge / `rolling_includes_max_surge`.
#[test]
fn upstream_deployment_max_pods_during_surge_includes_max_surge() {
    let spec = DeploymentSpec {
        name: "w".into(),
        namespace: "d".into(),
        replicas: 8,
        strategy: Strategy::RollingUpdate {
            max_surge: 3,
            max_unavailable: 0,
        },
        paused: false,
        progress_deadline_seconds: None,
    };
    assert_eq!(max_pods_during_surge(&spec), 11);
}

/// Upstream: TestRollback / `unknown_revision_requeues`.
/// `pkg/controller/deployment/rollback.go::rollback` returns Requeue with
/// `RollbackRevisionNotFound` condition when the target revision is missing.
#[test]
fn upstream_deployment_rollback_unknown_revision_requeues() {
    let spec = DeploymentSpec {
        name: "w".into(),
        namespace: "d".into(),
        replicas: 5,
        strategy: Strategy::RollingUpdate {
            max_surge: 1,
            max_unavailable: 0,
        },
        paused: false,
        progress_deadline_seconds: None,
    };
    let mut history = RevisionHistory::new(5);
    history.record("h1");
    history.record("h2");
    let decision = rollback(&spec, &history, 999).unwrap();
    assert_eq!(decision, Reconcile::Requeue);
    // Revision 0 is rejected outright.
    assert!(rollback(&spec, &history, 0).is_err());
}

/// Upstream: TestRollingUpdate / `surge_room_caps_new_rs_target`.
/// rolling.go::reconcileNewReplicaSet — new RS may scale up by
/// `max_surge - (current_pods - replicas)`. With max_unavailable>0
/// old RS may simultaneously shed pods up to that budget.
#[test]
fn upstream_deployment_rolling_step_caps_new_rs_at_replicas_plus_surge() {
    let spec = DeploymentSpec {
        name: "w".into(),
        namespace: "d".into(),
        replicas: 10,
        strategy: Strategy::RollingUpdate {
            max_surge: 2,
            max_unavailable: 2,
        },
        paused: false,
        progress_deadline_seconds: None,
    };
    // Start of rollout: 0 new pods, 10 old.
    let step = plan_rolling_step(&spec, 0, 10).unwrap();
    // surge_room = (10 + 2) − 10 = 2 → new can scale up to 2.
    assert_eq!(step.new_rs_target, 2);
    // With max_unavailable=2: min_available=8, currently alive=10,
    // removable=2, so old RS drops from 10 → 8 in the same pass.
    assert_eq!(step.old_rs_target, 8);
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/controller/replicaset/replica_set_test.go
// ────────────────────────────────────────────────────────────────────────────

fn rs_spec(replicas: u32, selector: &[(&str, &str)]) -> ReplicaSetSpec {
    ReplicaSetSpec {
        name: "web-abc".into(),
        namespace: "default".into(),
        replicas,
        selector: selector
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
    }
}

/// Upstream: TestManageReplicas / `empty_selector_is_rejected`.
/// replica_set.go::syncReplicaSet — an empty selector is `everything`,
/// upstream refuses to manage that to prevent runaway deletions.
#[test]
fn upstream_replicaset_empty_selector_rejected() {
    let spec = rs_spec(3, &[]);
    let status = ReplicaSetStatus::default();
    let err = rs_reconcile(&spec, &status, &tenant("acme")).unwrap_err();
    assert!(matches!(err, ControllerError::InvalidSpec { kind: "ReplicaSet", .. }));
}

/// Upstream: TestManageReplicas / `under_replicated_yields_create`.
#[test]
fn upstream_replicaset_under_replicated_creates_diff() {
    let spec = rs_spec(5, &[("app", "web")]);
    let status = ReplicaSetStatus {
        running_pods: 2,
        ..Default::default()
    };
    assert_eq!(
        rs_reconcile(&spec, &status, &tenant("acme")).unwrap(),
        Reconcile::Create(3)
    );
}

/// Upstream: TestBurstReplicas / `slow_start_caps_creates`.
/// replica_set.go::BurstReplicas — at most `burst_replicas` per pass.
#[test]
fn upstream_replicaset_burst_replicas_caps_per_pass_creates() {
    let big_diff = Reconcile::Create(500);
    let capped = clamp_burst(big_diff, 50);
    assert_eq!(capped, Reconcile::Create(50));
    // Deletes are also bursted.
    assert_eq!(clamp_burst(Reconcile::Delete(80), 25), Reconcile::Delete(25));
    // NoOp/Update pass through.
    assert_eq!(clamp_burst(Reconcile::NoOp, 10), Reconcile::NoOp);
}

/// Upstream: TestClaimPods / `adopts_orphans_matching_selector`.
/// `controller_ref_manager.go::ClaimPods` — pods with no controllerRef AND
/// matching selector AND in the same namespace are claim candidates.
#[test]
fn upstream_replicaset_adopts_orphan_matching_selector_only() {
    let spec = rs_spec(3, &[("app", "web")]);
    let pods = vec![
        // Orphan match → adopt.
        PodView {
            name: "p1".into(),
            namespace: "default".into(),
            labels: vec![("app".into(), "web".into())],
            controller_ref: None,
        },
        // Already owned → skip.
        PodView {
            name: "p2".into(),
            namespace: "default".into(),
            labels: vec![("app".into(), "web".into())],
            controller_ref: Some("rs-other".into()),
        },
        // Wrong namespace → skip.
        PodView {
            name: "p3".into(),
            namespace: "kube-system".into(),
            labels: vec![("app".into(), "web".into())],
            controller_ref: None,
        },
        // Wrong label → skip.
        PodView {
            name: "p4".into(),
            namespace: "default".into(),
            labels: vec![("app".into(), "api".into())],
            controller_ref: None,
        },
    ];
    let plan: AdoptionPlan = adopt_orphans(&spec, &pods, &tenant("acme")).unwrap();
    assert_eq!(plan.claimed, vec!["p1".to_string()]);
    assert_eq!(plan.count(), 1);
}

/// Upstream: TestReleasePods / `release_pod_with_mismatched_labels`.
/// `controller_ref_manager.go::release` — adopted pods whose labels no
/// longer match the selector are released.
#[test]
fn upstream_replicaset_releases_owned_pod_when_labels_drift() {
    let spec = rs_spec(3, &[("app", "web")]);
    let pods = vec![
        PodView {
            name: "still-matches".into(),
            namespace: "default".into(),
            labels: vec![("app".into(), "web".into())],
            controller_ref: Some("rs-1".into()),
        },
        PodView {
            name: "label-drifted".into(),
            namespace: "default".into(),
            labels: vec![("app".into(), "api".into())],
            controller_ref: Some("rs-1".into()),
        },
        PodView {
            name: "not-mine".into(),
            namespace: "default".into(),
            labels: vec![("app".into(), "api".into())],
            controller_ref: Some("rs-other".into()),
        },
    ];
    let released = release_mismatched(&spec, &pods, "rs-1").unwrap();
    // Only owned + label-mismatched gets released; not-mine is ignored.
    assert_eq!(released, vec!["label-drifted".to_string()]);
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/controller/job/job_controller_test.go
// ────────────────────────────────────────────────────────────────────────────

fn job(parallelism: u32, completions: u32, backoff: u32, suspended: bool) -> JobSpec {
    JobSpec {
        name: "build".into(),
        namespace: "ci".into(),
        completions,
        parallelism,
        backoff_limit: backoff,
        suspended,
        active_deadline_seconds: None,
    }
}

/// Upstream: TestManageJob / `creates_up_to_parallelism_on_fresh_job`.
#[test]
fn upstream_job_fresh_creates_up_to_parallelism() {
    let spec = job(/*par*/ 4, /*compl*/ 10, /*backoff*/ 6, false);
    let status = JobStatus::default();
    assert_eq!(
        job_reconcile(&spec, &status, &tenant("ci")).unwrap(),
        Reconcile::Create(4)
    );
}

/// Upstream: TestIsJobFinished / `succeeded_ge_completions`.
#[test]
fn upstream_job_is_complete_at_succeeded_equals_completions() {
    let spec = job(2, 5, 6, false);
    let status = JobStatus {
        active: 0,
        succeeded: 5,
        failed: 0,
        start_time: None,
    };
    assert!(is_complete(&spec, &status));
    // Past completions also counts as complete (idempotency).
    let over = JobStatus {
        active: 0,
        succeeded: 6,
        failed: 0,
        start_time: None,
    };
    assert!(is_complete(&spec, &over));
}

/// Upstream: TestPastBackoffLimitOnFailure / `strictly_greater_than_limit`.
/// Boundary case — `failed == backoff_limit` is NOT past the limit.
#[test]
fn upstream_job_past_backoff_is_strictly_greater_than_limit() {
    let spec = job(2, 10, 3, false);
    let at_limit = JobStatus {
        active: 0,
        succeeded: 0,
        failed: 3,
        start_time: None,
    };
    assert!(!past_backoff(&spec, &at_limit));
    let over = JobStatus {
        active: 0,
        succeeded: 0,
        failed: 4,
        start_time: None,
    };
    assert!(past_backoff(&spec, &over));
}

/// Upstream: TestSuspendedJob / `suspended_deletes_active_pods`.
#[test]
fn upstream_job_suspended_deletes_active_pods() {
    let spec = job(4, 10, 6, /*suspended=*/ true);
    let status = JobStatus {
        active: 3,
        succeeded: 0,
        failed: 0,
        start_time: None,
    };
    assert_eq!(
        job_reconcile(&spec, &status, &tenant("ci")).unwrap(),
        Reconcile::Delete(3)
    );
}

/// Upstream: TestFirstPendingIndexes / `picks_lowest_within_budget`.
/// `indexed_job_utils.go::firstPendingIndexes` — lowest pending indexes
/// up to `parallelism - active`.
#[test]
fn upstream_indexed_job_picks_lowest_pending_within_budget() {
    let spec = job(/*par*/ 3, /*compl*/ 5, 6, false);
    let status = IndexedJobStatus {
        indexes: vec![
            IndexState::Succeeded, // 0
            IndexState::Pending,   // 1
            IndexState::Active,    // 2 (counts toward parallelism)
            IndexState::Pending,   // 3
            IndexState::Pending,   // 4
        ],
    };
    // active=1, parallelism=3 → budget=2 → pick lowest two pending: 1, 3.
    let picks = index_status(&spec, &status).unwrap();
    assert_eq!(picks, vec![1, 3]);
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/controller/cronjob/cronjob_controllerv2_test.go
// ────────────────────────────────────────────────────────────────────────────

fn cj(schedule: &str, policy: ConcurrencyPolicy, suspended: bool) -> CronJobSpec {
    CronJobSpec {
        name: "report".into(),
        namespace: "default".into(),
        schedule: schedule.into(),
        concurrency: policy,
        suspended,
    }
}

/// Upstream: TestParseSchedule / `accepts_five_fields_rejects_others`.
#[test]
fn upstream_cronjob_validate_schedule_requires_five_fields() {
    assert!(validate_schedule("*/5 * * * *").is_ok());
    assert!(validate_schedule("0 0 1 1 0").is_ok());
    assert!(validate_schedule("only-one-field").is_err());
    assert!(validate_schedule("a b c d").is_err());
    assert!(validate_schedule("a b c d e f").is_err());
}

/// Upstream: TestSyncCronJob_ConcurrencyPolicy / `Forbid_no_op_when_active`.
#[test]
fn upstream_cronjob_forbid_policy_skips_when_active_jobs_exist() {
    let spec = cj("0 * * * *", ConcurrencyPolicy::Forbid, false);
    let status = CronJobStatus {
        active_jobs: 1,
        ..Default::default()
    };
    assert_eq!(
        cron_reconcile(&spec, &status, &tenant("ops")).unwrap(),
        Reconcile::NoOp
    );
}

/// Upstream: TestSyncCronJob_ConcurrencyPolicy / `Replace_deletes_active_before_creating`.
#[test]
fn upstream_cronjob_replace_policy_deletes_active_jobs() {
    let spec = cj("0 * * * *", ConcurrencyPolicy::Replace, false);
    let status = CronJobStatus {
        active_jobs: 2,
        ..Default::default()
    };
    // Mirrors syncCronJob's Replace branch: first delete active, next pass
    // re-fires.
    assert_eq!(
        cron_reconcile(&spec, &status, &tenant("ops")).unwrap(),
        Reconcile::Delete(2)
    );
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/controller/daemon/daemon_controller_test.go +
//           apimachinery/pkg/api/v1/helper/helpers_test.go (TestToleratesTaint)
// ────────────────────────────────────────────────────────────────────────────

fn ds_spec(selector: &[(&str, &str)]) -> DaemonSetSpec {
    DaemonSetSpec {
        name: "node-exporter".into(),
        namespace: "monitoring".into(),
        node_selector: selector
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
    }
}

fn node(name: &str, labels: &[(&str, &str)], schedulable: bool, has_pod: bool) -> NodeView {
    NodeView {
        name: name.into(),
        labels: labels
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
        schedulable,
        running_ds_pod: has_pod,
    }
}

/// Upstream: TestNodeShouldRunDaemonPod / `unschedulable_node_skipped`.
#[test]
fn upstream_daemonset_node_should_run_skips_unschedulable_node() {
    let spec = ds_spec(&[]);
    let schedulable_n = node("a", &[], true, false);
    let unschedulable_n = node("b", &[], false, false);
    assert!(node_should_run(&spec, &schedulable_n));
    assert!(!node_should_run(&spec, &unschedulable_n));
}

/// Upstream: TestManage / `evicts_pod_when_node_no_longer_matches`.
#[test]
fn upstream_daemonset_evicts_pod_when_node_no_longer_matches_selector() {
    let spec = ds_spec(&[("role", "edge")]);
    let nodes = vec![node("former-edge", &[("role", "core")], true, true)];
    assert_eq!(
        ds_reconcile(&spec, &nodes, &tenant("monitoring")).unwrap(),
        Reconcile::Delete(1)
    );
}

/// Upstream: TestToleratesTaint / `exists_operator_with_no_key_tolerates_all`.
/// `helpers.go::ToleratesTaint` — Exists with empty key tolerates EVERY
/// taint of the matching effect.
#[test]
fn upstream_daemonset_toleration_exists_with_no_key_tolerates_any_taint() {
    let any = Toleration {
        key: None,
        operator: TolerationOperator::Exists,
        value: None,
        effect: Some(TaintEffect::NoSchedule),
    };
    let t1 = Taint {
        key: "any".into(),
        value: None,
        effect: TaintEffect::NoSchedule,
    };
    let t2 = Taint {
        key: "other".into(),
        value: Some("v".into()),
        effect: TaintEffect::NoSchedule,
    };
    assert!(tolerates(&any, &t1));
    assert!(tolerates(&any, &t2));
}

/// Upstream: TestFindMatchingUntoleratedTaint / `prefer_no_schedule_is_advisory`.
/// PreferNoSchedule is NEVER a hard block — tolerates_all ignores it.
#[test]
fn upstream_daemonset_tolerates_all_ignores_prefer_no_schedule_taints() {
    let taints = vec![Taint {
        key: "nice-to-have".into(),
        value: None,
        effect: TaintEffect::PreferNoSchedule,
    }];
    let tolerations: Vec<Toleration> = vec![];
    assert!(tolerates_all(&taints, &tolerations));
}
