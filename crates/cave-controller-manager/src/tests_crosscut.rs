//! Cross-controller crosscut tests.
//!
//! Verifies behaviours that span multiple controller reconcile loops in the
//! upstream `kube-controller-manager` package set:
//!
//! * Reconcile decisions for the workload controllers (Deployment,
//!   ReplicaSet, StatefulSet, DaemonSet, Job, CronJob) under classic
//!   spec/status combinations.
//! * Tenant-isolation invariants enforced by the manager loop wiring.
//! * Stable invariants on the public admin surface (`CONTROLLERS`,
//!   `leader_state`).
//!
//! Each test carries a `Cite` to the upstream Go source it mirrors so the
//! parity audit trail stays explicit.

#![cfg(test)]

use crate::test_ctx;
use crate::types::{Reconcile, TenantId};
use crate::{cronjob, daemonset, deployment, hpa, job, pdb, replicaset, statefulset};

// ── Deployment ──────────────────────────────────────────────────────────────

#[test]
fn deployment_paused_short_circuits_to_noop() {
    let (_c, t) = test_ctx!(
        "pkg/controller/deployment/deployment_controller.go",
        "syncDeployment",
        "tenant-cm-deploy-paused"
    );
    let spec = deployment::DeploymentSpec {
        name: "web".into(),
        namespace: "default".into(),
        replicas: 5,
        strategy: deployment::Strategy::RollingUpdate { max_surge: 1, max_unavailable: 0 },
        paused: true,
    };
    let status = deployment::DeploymentStatus { observed_replicas: 0, ..Default::default() };
    assert_eq!(deployment::reconcile(&spec, &status, &t).unwrap(), Reconcile::NoOp);
}

#[test]
fn deployment_scale_up_emits_create_decision() {
    let (_c, t) = test_ctx!(
        "pkg/controller/deployment/sync.go",
        "scale",
        "tenant-cm-deploy-up"
    );
    let spec = deployment::DeploymentSpec {
        name: "web".into(),
        namespace: "default".into(),
        replicas: 5,
        strategy: deployment::Strategy::RollingUpdate { max_surge: 1, max_unavailable: 0 },
        paused: false,
    };
    let status = deployment::DeploymentStatus { observed_replicas: 2, ..Default::default() };
    assert_eq!(deployment::reconcile(&spec, &status, &t).unwrap(), Reconcile::Create(3));
}

#[test]
fn deployment_scale_down_emits_delete_decision() {
    let (_c, t) = test_ctx!(
        "pkg/controller/deployment/sync.go",
        "scale",
        "tenant-cm-deploy-down"
    );
    let spec = deployment::DeploymentSpec {
        name: "web".into(),
        namespace: "default".into(),
        replicas: 2,
        strategy: deployment::Strategy::Recreate,
        paused: false,
    };
    let status = deployment::DeploymentStatus { observed_replicas: 5, ..Default::default() };
    assert_eq!(deployment::reconcile(&spec, &status, &t).unwrap(), Reconcile::Delete(3));
}

#[test]
fn deployment_max_pods_during_surge_caps_at_replicas_for_recreate() {
    let (_c, _t) = test_ctx!(
        "pkg/controller/deployment/util/deployment_util.go",
        "MaxSurge",
        "tenant-cm-deploy-recreate"
    );
    let spec = deployment::DeploymentSpec {
        name: "web".into(),
        namespace: "default".into(),
        replicas: 7,
        strategy: deployment::Strategy::Recreate,
        paused: false,
    };
    assert_eq!(deployment::max_pods_during_surge(&spec), 7);
}

#[test]
fn deployment_revision_history_evicts_oldest_past_limit() {
    let (_c, _t) = test_ctx!(
        "pkg/controller/deployment/util/deployment_util.go",
        "revisionHistoryLimit",
        "tenant-cm-deploy-rev"
    );
    let mut h = deployment::RevisionHistory::new(3);
    for tag in ["a", "b", "c", "d", "e"] {
        h.record(tag);
    }
    assert_eq!(h.revisions.len(), 3);
    assert_eq!(h.revisions.first().unwrap().pod_template_hash, "c");
    assert_eq!(h.revisions.last().unwrap().pod_template_hash, "e");
}

// ── ReplicaSet ──────────────────────────────────────────────────────────────

#[test]
fn replicaset_reconcile_at_target_is_noop() {
    let (_c, t) = test_ctx!(
        "pkg/controller/replicaset/replica_set.go",
        "syncReplicaSet",
        "tenant-cm-rs-noop"
    );
    let spec = replicaset::ReplicaSetSpec {
        name: "web-rs".into(),
        namespace: "default".into(),
        replicas: 4,
        selector: vec![("app".into(), "web".into())],
    };
    let status = replicaset::ReplicaSetStatus { running_pods: 4, failed_pods: 0 };
    assert_eq!(replicaset::reconcile(&spec, &status, &t).unwrap(), Reconcile::NoOp);
}

#[test]
fn replicaset_under_replicated_creates() {
    let (_c, t) = test_ctx!(
        "pkg/controller/replicaset/replica_set.go",
        "manageReplicas",
        "tenant-cm-rs-create"
    );
    let spec = replicaset::ReplicaSetSpec {
        name: "web-rs".into(),
        namespace: "default".into(),
        replicas: 5,
        selector: vec![("app".into(), "web".into())],
    };
    let status = replicaset::ReplicaSetStatus { running_pods: 1, failed_pods: 0 };
    assert_eq!(replicaset::reconcile(&spec, &status, &t).unwrap(), Reconcile::Create(4));
}

#[test]
fn replicaset_empty_selector_is_invalid_spec() {
    let (_c, t) = test_ctx!(
        "pkg/controller/replicaset/replica_set.go",
        "validateSelector",
        "tenant-cm-rs-empty-sel"
    );
    let spec = replicaset::ReplicaSetSpec {
        name: "web-rs".into(),
        namespace: "default".into(),
        replicas: 3,
        selector: vec![],
    };
    let status = replicaset::ReplicaSetStatus::default();
    assert!(replicaset::reconcile(&spec, &status, &t).is_err());
}

#[test]
fn replicaset_burst_clamps_create_decision() {
    let (_c, _t) = test_ctx!(
        "pkg/controller/replicaset/replica_set.go",
        "BurstReplicas",
        "tenant-cm-rs-burst"
    );
    let clamped = replicaset::clamp_burst(Reconcile::Create(50), 5);
    assert_eq!(clamped, Reconcile::Create(5));
}

// ── StatefulSet ─────────────────────────────────────────────────────────────

#[test]
fn statefulset_pod_identity_uses_zero_indexed_ordinals() {
    let (_c, _t) = test_ctx!(
        "pkg/controller/statefulset/stateful_set_utils.go",
        "getPodName",
        "tenant-cm-sts-id"
    );
    let spec = statefulset::StatefulSetSpec {
        name: "redis".into(),
        namespace: "default".into(),
        replicas: 3,
        policy: statefulset::PodManagementPolicy::OrderedReady,
    };
    assert_eq!(statefulset::pod_identity(&spec, 0), "redis-0");
    assert_eq!(statefulset::pod_identity(&spec, 2), "redis-2");
}

#[test]
fn statefulset_ordinal_range_matches_replica_count() {
    let (_c, _t) = test_ctx!(
        "pkg/controller/statefulset/stateful_set_control.go",
        "getStartOrdinal",
        "tenant-cm-sts-range"
    );
    let spec = statefulset::StatefulSetSpec {
        name: "kafka".into(),
        namespace: "default".into(),
        replicas: 3,
        policy: statefulset::PodManagementPolicy::Parallel,
    };
    let r = statefulset::ordinal_range(&spec);
    assert_eq!(r.start, 0);
    assert_eq!(r.end, 3);
}

#[test]
fn statefulset_ordered_policy_steps_one_pod_per_pass() {
    let (_c, t) = test_ctx!(
        "pkg/controller/statefulset/stateful_set.go",
        "updateStatefulSet",
        "tenant-cm-sts-ordered"
    );
    let spec = statefulset::StatefulSetSpec {
        name: "kafka".into(),
        namespace: "default".into(),
        replicas: 5,
        policy: statefulset::PodManagementPolicy::OrderedReady,
    };
    let status = statefulset::StatefulSetStatus { current_replicas: 2, ready_replicas: 2 };
    assert_eq!(statefulset::reconcile(&spec, &status, &t).unwrap(), Reconcile::Create(1));
}

#[test]
fn statefulset_parallel_policy_fans_out_in_one_pass() {
    let (_c, t) = test_ctx!(
        "pkg/controller/statefulset/stateful_set.go",
        "updateStatefulSet",
        "tenant-cm-sts-parallel"
    );
    let spec = statefulset::StatefulSetSpec {
        name: "kafka".into(),
        namespace: "default".into(),
        replicas: 5,
        policy: statefulset::PodManagementPolicy::Parallel,
    };
    let status = statefulset::StatefulSetStatus { current_replicas: 1, ready_replicas: 1 };
    assert_eq!(statefulset::reconcile(&spec, &status, &t).unwrap(), Reconcile::Create(4));
}

// ── DaemonSet ───────────────────────────────────────────────────────────────

#[test]
fn daemonset_node_should_run_when_no_node_selector_and_schedulable() {
    let (_c, _t) = test_ctx!(
        "pkg/controller/daemon/util/daemonset_util.go",
        "ShouldRunDaemonPodOnNode",
        "tenant-cm-ds-default"
    );
    let spec = daemonset::DaemonSetSpec {
        name: "log-agent".into(),
        namespace: "kube-system".into(),
        node_selector: vec![],
    };
    let node = daemonset::NodeView {
        name: "n1".into(),
        labels: vec![("zone".into(), "eu".into())],
        schedulable: true,
        running_ds_pod: false,
    };
    assert!(daemonset::node_should_run(&spec, &node));
}

#[test]
fn daemonset_node_skipped_when_unschedulable() {
    let (_c, _t) = test_ctx!(
        "pkg/controller/daemon/util/daemonset_util.go",
        "ShouldRunDaemonPodOnNode",
        "tenant-cm-ds-cordoned"
    );
    let spec = daemonset::DaemonSetSpec {
        name: "log-agent".into(),
        namespace: "kube-system".into(),
        node_selector: vec![],
    };
    let node = daemonset::NodeView {
        name: "n1".into(),
        labels: vec![],
        schedulable: false,
        running_ds_pod: false,
    };
    assert!(!daemonset::node_should_run(&spec, &node));
}

#[test]
fn daemonset_toleration_equal_matches_taint_by_key_and_value() {
    let (_c, _t) = test_ctx!(
        "pkg/util/taints/taints.go",
        "TolerationsTolerateTaint",
        "tenant-cm-ds-tol-eq"
    );
    let taint = daemonset::Taint {
        key: "dedicated".into(),
        value: Some("gpu".into()),
        effect: daemonset::TaintEffect::NoSchedule,
    };
    let toleration = daemonset::Toleration {
        key: Some("dedicated".into()),
        operator: daemonset::TolerationOperator::Equal,
        value: Some("gpu".into()),
        effect: Some(daemonset::TaintEffect::NoSchedule),
    };
    assert!(daemonset::tolerates(&toleration, &taint));
}

#[test]
fn daemonset_tolerates_all_only_considers_noschedule_and_noexecute() {
    let (_c, _t) = test_ctx!(
        "apimachinery/pkg/api/v1/helper/helpers.go",
        "FindMatchingUntoleratedTaint",
        "tenant-cm-ds-tol-prefer"
    );
    let taints = vec![daemonset::Taint {
        key: "x".into(),
        value: None,
        effect: daemonset::TaintEffect::PreferNoSchedule,
    }];
    // No tolerations supplied — PreferNoSchedule is informational, so this passes.
    assert!(daemonset::tolerates_all(&taints, &[]));
}

// ── Job ─────────────────────────────────────────────────────────────────────

#[test]
fn job_is_complete_when_succeeded_meets_completions() {
    let (_c, _t) = test_ctx!(
        "pkg/controller/job/util/job_utils.go",
        "IsJobFinished",
        "tenant-cm-job-done"
    );
    let spec = job::JobSpec {
        name: "etl".into(),
        namespace: "default".into(),
        completions: 5,
        parallelism: 2,
        backoff_limit: 3,
        suspended: false,
    };
    let status = job::JobStatus { active: 0, succeeded: 5, failed: 0 };
    assert!(job::is_complete(&spec, &status));
}

#[test]
fn job_past_backoff_when_failed_exceeds_limit() {
    let (_c, _t) = test_ctx!(
        "pkg/controller/job/job_controller.go",
        "pastBackoffLimitOnFailure",
        "tenant-cm-job-fail"
    );
    let spec = job::JobSpec {
        name: "etl".into(),
        namespace: "default".into(),
        completions: 5,
        parallelism: 2,
        backoff_limit: 3,
        suspended: false,
    };
    let status = job::JobStatus { active: 0, succeeded: 0, failed: 4 };
    assert!(job::past_backoff(&spec, &status));
}

#[test]
fn job_suspended_with_active_pods_emits_delete() {
    let (_c, t) = test_ctx!(
        "pkg/controller/job/job_controller.go",
        "manageJob",
        "tenant-cm-job-suspend"
    );
    let spec = job::JobSpec {
        name: "etl".into(),
        namespace: "default".into(),
        completions: 5,
        parallelism: 2,
        backoff_limit: 3,
        suspended: true,
    };
    let status = job::JobStatus { active: 2, succeeded: 0, failed: 0 };
    assert_eq!(job::reconcile(&spec, &status, &t).unwrap(), Reconcile::Delete(2));
}

#[test]
fn job_clamps_creates_to_parallelism_remaining() {
    let (_c, t) = test_ctx!(
        "pkg/controller/job/job_controller.go",
        "manageJob",
        "tenant-cm-job-paral"
    );
    let spec = job::JobSpec {
        name: "etl".into(),
        namespace: "default".into(),
        completions: 10,
        parallelism: 3,
        backoff_limit: 6,
        suspended: false,
    };
    let status = job::JobStatus { active: 1, succeeded: 0, failed: 0 };
    assert_eq!(job::reconcile(&spec, &status, &t).unwrap(), Reconcile::Create(2));
}

// ── CronJob ─────────────────────────────────────────────────────────────────

#[test]
fn cronjob_reconcile_emits_create_when_allow_and_no_active() {
    let (_c, t) = test_ctx!(
        "pkg/controller/cronjob/cronjob_controllerv2.go",
        "syncCronJob",
        "tenant-cm-cj-due"
    );
    let spec = cronjob::CronJobSpec {
        name: "nightly".into(),
        namespace: "default".into(),
        schedule: "0 * * * *".into(),
        concurrency: cronjob::ConcurrencyPolicy::Allow,
        suspended: false,
    };
    let status = cronjob::CronJobStatus::default();
    assert_eq!(cronjob::reconcile(&spec, &status, &t).unwrap(), Reconcile::Create(1));
}

#[test]
fn cronjob_reconcile_suspended_is_noop() {
    let (_c, t) = test_ctx!(
        "pkg/controller/cronjob/cronjob_controllerv2.go",
        "syncCronJob",
        "tenant-cm-cj-suspend"
    );
    let spec = cronjob::CronJobSpec {
        name: "nightly".into(),
        namespace: "default".into(),
        schedule: "0 * * * *".into(),
        concurrency: cronjob::ConcurrencyPolicy::Allow,
        suspended: true,
    };
    let status = cronjob::CronJobStatus::default();
    assert_eq!(cronjob::reconcile(&spec, &status, &t).unwrap(), Reconcile::NoOp);
}

#[test]
fn cronjob_forbid_with_active_run_skips_new_fire() {
    let (_c, t) = test_ctx!(
        "pkg/controller/cronjob/cronjob_controllerv2.go",
        "syncCronJob",
        "tenant-cm-cj-forbid"
    );
    let spec = cronjob::CronJobSpec {
        name: "nightly".into(),
        namespace: "default".into(),
        schedule: "0 * * * *".into(),
        concurrency: cronjob::ConcurrencyPolicy::Forbid,
        suspended: false,
    };
    let status = cronjob::CronJobStatus { active_jobs: 1, last_schedule_time: None };
    assert_eq!(cronjob::reconcile(&spec, &status, &t).unwrap(), Reconcile::NoOp);
}

#[test]
fn cronjob_invalid_schedule_string_errors() {
    let (_c, _t) = test_ctx!(
        "pkg/controller/cronjob/utils.go",
        "ParseSchedule",
        "tenant-cm-cj-invalid"
    );
    assert!(cronjob::validate_schedule("not a cron").is_err());
    assert!(cronjob::validate_schedule("0 * * * *").is_ok());
}

// ── HPA ─────────────────────────────────────────────────────────────────────

#[test]
fn hpa_reconcile_inside_band_is_noop() {
    let (_c, t) = test_ctx!(
        "pkg/controller/podautoscaler/horizontal.go",
        "reconcileAutoscaler",
        "tenant-cm-hpa-noop"
    );
    let spec = hpa::HpaSpec {
        name: "web-hpa".into(),
        namespace: "default".into(),
        min_replicas: 2,
        max_replicas: 10,
        target_cpu_utilization_pct: 80,
    };
    // current_replicas=3, current=80, target=80 → desired=3 (no change)
    let status = hpa::HpaStatus { current_replicas: 3, current_cpu_utilization_pct: 80 };
    assert_eq!(hpa::reconcile(&spec, &status, &t).unwrap(), Reconcile::NoOp);
}

#[test]
fn hpa_reconcile_above_target_scales_up_within_max() {
    let (_c, t) = test_ctx!(
        "pkg/controller/podautoscaler/horizontal.go",
        "reconcileAutoscaler",
        "tenant-cm-hpa-up"
    );
    let spec = hpa::HpaSpec {
        name: "web-hpa".into(),
        namespace: "default".into(),
        min_replicas: 2,
        max_replicas: 10,
        target_cpu_utilization_pct: 50,
    };
    // current=4, util=100, target=50 → desired = ceil(4 * 100 / 50) = 8
    let status = hpa::HpaStatus { current_replicas: 4, current_cpu_utilization_pct: 100 };
    assert_eq!(hpa::reconcile(&spec, &status, &t).unwrap(), Reconcile::Update(8));
}

#[test]
fn hpa_target_zero_is_invalid_spec() {
    let (_c, _t) = test_ctx!(
        "pkg/controller/podautoscaler/replica_calculator.go",
        "GetResourceReplicas",
        "tenant-cm-hpa-zero"
    );
    let spec = hpa::HpaSpec {
        name: "x".into(),
        namespace: "default".into(),
        min_replicas: 1,
        max_replicas: 10,
        target_cpu_utilization_pct: 0,
    };
    let status = hpa::HpaStatus { current_replicas: 1, current_cpu_utilization_pct: 50 };
    assert!(hpa::desired_replicas(&spec, &status).is_err());
}

// ── PDB ─────────────────────────────────────────────────────────────────────

#[test]
fn pdb_disruptions_allowed_uses_min_available_count() {
    let (_c, _t) = test_ctx!(
        "pkg/controller/disruption/disruption.go",
        "trySetPDBStatus",
        "tenant-cm-pdb-min-count"
    );
    let spec = pdb::PdbSpec {
        name: "api-pdb".into(),
        namespace: "default".into(),
        min_available: Some(pdb::Threshold::Count(3)),
        max_unavailable: None,
    };
    let status = pdb::PdbStatus {
        current_healthy: 5,
        expected_pods: 6,
        disruptions_allowed: 0,
    };
    // healthy=5, need=3 → allowed = 2
    assert_eq!(pdb::disruptions_allowed(&spec, &status).unwrap(), 2);
}

#[test]
fn pdb_admit_eviction_denies_when_no_budget() {
    let (_c, _t) = test_ctx!(
        "pkg/registry/policy/eviction/storage/storage.go",
        "Eviction.Create",
        "tenant-cm-pdb-deny"
    );
    let status = pdb::PdbStatus {
        current_healthy: 1,
        expected_pods: 3,
        disruptions_allowed: 0,
    };
    let decision = pdb::admit_eviction(&status, false);
    assert!(matches!(decision, pdb::EvictionDecision::Deny { .. }));
}

#[test]
fn pdb_admit_eviction_dry_run_always_allows() {
    let (_c, _t) = test_ctx!(
        "pkg/registry/policy/eviction/storage/storage.go",
        "Eviction.Create",
        "tenant-cm-pdb-dry"
    );
    let status = pdb::PdbStatus {
        current_healthy: 1,
        expected_pods: 3,
        disruptions_allowed: 0,
    };
    assert_eq!(pdb::admit_eviction(&status, true), pdb::EvictionDecision::Allow);
}

// ── Manager loop / workqueue ─────────────────────────────────────────────────

#[test]
fn manager_loop_workqueue_dedups_repeated_adds() {
    let (_c, _t) = test_ctx!(
        "client-go/util/workqueue/queue.go",
        "Type.Add",
        "tenant-cm-mgr-dedup"
    );
    use crate::deeper::manager::{ObjectKey, Workqueue};
    let mut q = Workqueue::new();
    let k = ObjectKey::new(TenantId::new("acme").expect("test fixture"), "Deployment", "default", "web");
    q.add(k.clone());
    q.add(k.clone());
    q.add(k);
    assert_eq!(q.len(), 1);
}

#[test]
fn manager_loop_drops_cross_tenant_keys_at_drain() {
    let (_c, _t) = test_ctx!(
        "cmd/kube-controller-manager/app/controllermanager.go",
        "Run",
        "tenant-cm-mgr-isolation"
    );
    use crate::deeper::manager::{
        ConstReconciler, Event, EventSource, ObjectKey, SyncController, Workqueue,
    };
    let owner = TenantId::new("acme").expect("test fixture");
    let mut src = EventSource::new();
    src.push(Event::Add(ObjectKey::new(TenantId::new("evil").expect("test fixture"), "Deployment", "default", "x")));
    src.push(Event::Add(ObjectKey::new(owner.clone(), "Deployment", "default", "y")));
    let mut q = Workqueue::new();
    src.drain_into(&mut q, &owner);
    let mut ctrl = SyncController::new(owner, ConstReconciler::new(Reconcile::NoOp));
    ctrl.run_until_idle(&mut q);
    assert_eq!(ctrl.processed, 1);
    // Cross-tenant filtered at drain stage, so the controller never sees it.
    assert_eq!(ctrl.denied_cross_tenant, 0);
}

// ── Admin surface (CONTROLLERS / leader_state / parity) ──────────────────────

#[test]
fn admin_controllers_surface_includes_all_workload_kinds() {
    let (_c, _t) = test_ctx!(
        "cmd/kube-controller-manager/app/controllermanager.go",
        "NewControllerInitializers",
        "tenant-cm-admin-controllers"
    );
    for must in [
        "deployment", "replicaset", "statefulset", "daemonset", "job", "cronjob",
        "hpa", "pdb", "endpointslice", "service",
    ] {
        assert!(crate::CONTROLLERS.contains(&must), "missing: {must}");
    }
}

#[test]
fn admin_controllers_surface_includes_lifecycle_set() {
    let (_c, _t) = test_ctx!(
        "cmd/kube-controller-manager/app/controllermanager.go",
        "NewControllerInitializers",
        "tenant-cm-admin-lifecycle"
    );
    for must in [
        "garbage-collector", "podgc", "ttl-after-finished",
        "node-lease", "node-lifecycle", "root-ca-publisher",
        "serviceaccount", "csr-signer", "rbac-aggregation",
    ] {
        assert!(crate::CONTROLLERS.contains(&must), "missing: {must}");
    }
}

#[test]
fn admin_leader_state_carries_holder_and_active_count() {
    let (_c, _t) = test_ctx!(
        "client-go/tools/leaderelection/leaderelection.go",
        "LeaderElector.Run",
        "tenant-cm-admin-leader"
    );
    let v = crate::leader_state("kcm-bootstrap");
    assert_eq!(v["holder_identity"], "kcm-bootstrap");
    assert_eq!(v["upstream_version"], crate::UPSTREAM_VERSION);
    assert_eq!(v["controllers_active"], crate::CONTROLLERS.len());
}

#[test]
fn admin_parity_report_returns_module_metadata() {
    let (_c, _t) = test_ctx!(
        "cave-controller-manager/parity.manifest.toml",
        "calculate_parity",
        "tenant-cm-admin-parity"
    );
    let report = crate::calculate_parity().expect("parity must succeed");
    assert_eq!(report.module, "cave-controller-manager");
    assert!(report.upstream_ref.contains("v1.36.0"));
}

#[test]
fn upstream_pkg_pins_to_canonical_path() {
    let (_c, _t) = test_ctx!(
        "pkg/controller/doc.go",
        "Package controller",
        "tenant-cm-pin-pkg"
    );
    assert_eq!(crate::UPSTREAM_PKG, "k8s.io/kubernetes/pkg/controller");
}

#[test]
fn upstream_version_pinned_to_release() {
    let (_c, _t) = test_ctx!(
        "version.go",
        "RELEASE",
        "tenant-cm-pin-ver"
    );
    assert!(crate::UPSTREAM_VERSION.starts_with('v'));
    assert!(crate::UPSTREAM_VERSION.split('.').count() >= 2);
}
