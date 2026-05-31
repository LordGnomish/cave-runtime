// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! RED → GREEN TDD for the edge node agent (`edged`).
//!
//! Faithful port of the kubelet pod-phase machine (kubernetes `pkg/kubelet/
//! kubelet_pods.go::getPhase`) and the KubeEdge `edge/pkg/edged` pod-worker
//! lifecycle (add/update/delete), orphaned-pod cleanup, and the status
//! manager's 10s report cadence (`statusUpdateInterval`).
//!
//! Pure decision logic — no container runtime, no network.

use cave_edge_runtime::edged::{
    ContainerState, ContainerStatus, Edged, Pod, PodPhase, PodWork, RestartPolicy,
};

fn pod_with(name: &str, policy: RestartPolicy, states: &[ContainerState]) -> Pod {
    let containers = states
        .iter()
        .enumerate()
        .map(|(i, s)| ContainerStatus {
            name: format!("c{i}"),
            state: s.clone(),
        })
        .collect();
    Pod {
        name: name.to_string(),
        namespace: "default".to_string(),
        uid: format!("uid-{name}"),
        restart_policy: policy,
        containers,
    }
}

// ─── kubelet getPhase ───────────────────────────────────────────────────────

#[test]
fn phase_pending_when_any_container_waiting() {
    let p = pod_with(
        "a",
        RestartPolicy::Always,
        &[ContainerState::Running, ContainerState::Waiting],
    );
    assert_eq!(p.compute_phase(false), PodPhase::Pending);
}

#[test]
fn phase_running_when_all_running() {
    let p = pod_with(
        "a",
        RestartPolicy::Always,
        &[ContainerState::Running, ContainerState::Running],
    );
    assert_eq!(p.compute_phase(false), PodPhase::Running);
}

#[test]
fn phase_pending_when_no_containers() {
    let p = pod_with("a", RestartPolicy::Always, &[]);
    assert_eq!(p.compute_phase(false), PodPhase::Pending);
}

#[test]
fn phase_succeeded_when_never_and_all_exit_zero() {
    let p = pod_with(
        "a",
        RestartPolicy::Never,
        &[
            ContainerState::Terminated { exit_code: 0 },
            ContainerState::Terminated { exit_code: 0 },
        ],
    );
    assert_eq!(p.compute_phase(false), PodPhase::Succeeded);
}

#[test]
fn phase_failed_when_never_and_any_nonzero() {
    let p = pod_with(
        "a",
        RestartPolicy::Never,
        &[
            ContainerState::Terminated { exit_code: 0 },
            ContainerState::Terminated { exit_code: 137 },
        ],
    );
    assert_eq!(p.compute_phase(false), PodPhase::Failed);
}

#[test]
fn phase_running_when_always_and_all_stopped_not_terminal() {
    // RestartPolicy=Always: stopped containers will be restarted → Running.
    let p = pod_with(
        "a",
        RestartPolicy::Always,
        &[ContainerState::Terminated { exit_code: 0 }],
    );
    assert_eq!(p.compute_phase(false), PodPhase::Running);
}

#[test]
fn phase_succeeded_when_always_terminal_and_all_exit_zero() {
    // Pod is terminating: Always no longer forces Running.
    let p = pod_with(
        "a",
        RestartPolicy::Always,
        &[ContainerState::Terminated { exit_code: 0 }],
    );
    assert_eq!(p.compute_phase(true), PodPhase::Succeeded);
}

#[test]
fn phase_running_when_onfailure_and_failure_present() {
    // OnFailure with a non-zero exit will restart → Running.
    let p = pod_with(
        "a",
        RestartPolicy::OnFailure,
        &[ContainerState::Terminated { exit_code: 2 }],
    );
    assert_eq!(p.compute_phase(false), PodPhase::Running);
}

// ─── pod-worker lifecycle ───────────────────────────────────────────────────

#[test]
fn worker_add_registers_pod_and_computes_phase() {
    let mut e = Edged::new("edge-node-1");
    let p = pod_with("web", RestartPolicy::Always, &[ContainerState::Running]);
    e.dispatch(PodWork::Add(p));
    assert_eq!(e.pod_count(), 1);
    assert_eq!(e.phase_of("web"), Some(PodPhase::Running));
}

#[test]
fn worker_delete_removes_pod() {
    let mut e = Edged::new("edge-node-1");
    e.dispatch(PodWork::Add(pod_with(
        "web",
        RestartPolicy::Always,
        &[ContainerState::Running],
    )));
    e.dispatch(PodWork::Delete("web".to_string()));
    assert_eq!(e.pod_count(), 0);
    assert_eq!(e.phase_of("web"), None);
}

#[test]
fn worker_update_replaces_spec_and_recomputes_phase() {
    let mut e = Edged::new("edge-node-1");
    e.dispatch(PodWork::Add(pod_with(
        "web",
        RestartPolicy::Always,
        &[ContainerState::Waiting],
    )));
    assert_eq!(e.phase_of("web"), Some(PodPhase::Pending));
    e.dispatch(PodWork::Update(pod_with(
        "web",
        RestartPolicy::Always,
        &[ContainerState::Running],
    )));
    assert_eq!(e.pod_count(), 1);
    assert_eq!(e.phase_of("web"), Some(PodPhase::Running));
}

// ─── orphaned-pod cleanup (cleanupOrphanedPodDirectories) ───────────────────

#[test]
fn orphan_cleanup_terminates_pods_absent_from_desired_set() {
    let mut e = Edged::new("edge-node-1");
    e.dispatch(PodWork::Add(pod_with("keep", RestartPolicy::Always, &[ContainerState::Running])));
    e.dispatch(PodWork::Add(pod_with("orphan", RestartPolicy::Always, &[ContainerState::Running])));
    // Desired set from the cloud now lists only "keep".
    let removed = e.cleanup_orphans(&["keep".to_string()]);
    assert_eq!(removed, vec!["orphan".to_string()]);
    assert_eq!(e.pod_count(), 1);
    assert!(e.phase_of("orphan").is_none());
}

#[test]
fn orphan_cleanup_noop_when_all_desired() {
    let mut e = Edged::new("edge-node-1");
    e.dispatch(PodWork::Add(pod_with("a", RestartPolicy::Always, &[ContainerState::Running])));
    let removed = e.cleanup_orphans(&["a".to_string()]);
    assert!(removed.is_empty());
    assert_eq!(e.pod_count(), 1);
}

// ─── status manager 10s cadence (statusUpdateInterval) ──────────────────────

#[test]
fn status_report_due_every_ten_seconds() {
    let mut e = Edged::new("edge-node-1");
    // First call at t=0 establishes the baseline and is due.
    assert!(e.status_report_due(0));
    assert!(!e.status_report_due(5));
    assert!(!e.status_report_due(9));
    assert!(e.status_report_due(10));
    // After reporting at t=10, the next window opens at t=20.
    assert!(!e.status_report_due(15));
    assert!(e.status_report_due(20));
}
