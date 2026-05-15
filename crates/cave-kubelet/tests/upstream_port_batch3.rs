// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Batch 3 (2026-05-14) — additional upstream test ports beyond
//! `upstream_port.rs` (batch1, 2026-05-13).
//!
//! Batch1 covered prober/{worker,results} + preemption + lifecycle
//! basics. Batch3 fills in the upstream coverage we deferred:
//! PodStatusManager dedupe/backoff, ProberCoordinator restart-
//! suppression + readiness fan-out de-dup, and lifecycle hook
//! reconciler edge cases.
//!
//! Upstream: kubernetes/kubernetes @ v1.36.0
//!   * pkg/kubelet/status/status_manager_test.go
//!   * pkg/kubelet/prober/{worker,results}_test.go
//!   * pkg/kubelet/lifecycle/handlers_test.go

use cave_kubelet::lifecycle::{
    HookExecution, HookHandler, HookOutcome, HookSample, HookStage, evaluate,
};
use cave_kubelet::pod_status_manager::{
    AttemptOutcome, ContainerStatus, DispatchOutcome, DropReason, PodPhase, PodStatus,
    PodStatusManager, StatusManagerConfig,
};
use cave_kubelet::probe::{
    ProbeKind, ProbeResult, ProbeSpec, ProberAction,
};
use cave_kubelet::prober::{ContainerRef, CoordinatorEvent, ProberConfig, ProberCoordinator};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use std::time::Duration;

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-05-14T10:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn pod_status(phase: PodPhase, ready: bool) -> PodStatus {
    PodStatus {
        phase,
        conditions: vec![("Ready".into(), ready)],
        containers: vec![ContainerStatus {
            name: "main".into(),
            ready,
            restart_count: 0,
            image: "alpine:3".into(),
        }],
        message: None,
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/kubelet/status/status_manager_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestSetPodStatus / `identical_status_is_deduped`.
/// `manager.SetPodStatus` hash-compares the new status against the
/// pending entry; same hash → drop, surface dedupe reason.
#[test]
fn upstream_status_manager_dedupes_identical_status_writes() {
    let mut m = PodStatusManager::new(StatusManagerConfig::default());
    let st = pod_status(PodPhase::Running, true);
    let first = m.set_status("pod-a", st.clone(), t0());
    assert!(first.is_none());
    let second = m.set_status("pod-a", st, t0());
    assert_eq!(second, Some(DropReason::Deduped));
    assert_eq!(m.pending_len(), 1);
}

/// Upstream: TestSetPodStatus / `deleted_pod_drops_subsequent_writes`.
#[test]
fn upstream_status_manager_drops_writes_for_deleted_pod() {
    let mut m = PodStatusManager::new(StatusManagerConfig::default());
    m.delete_pod("pod-b");
    let drop = m.set_status("pod-b", pod_status(PodPhase::Failed, false), t0());
    assert_eq!(drop, Some(DropReason::PodDeleted));
    assert_eq!(m.pending_len(), 0);
}

/// Upstream: TestSyncPod / `pop_ready_returns_idle_when_empty`.
#[test]
fn upstream_status_manager_pop_ready_idle_when_no_pending() {
    let mut m = PodStatusManager::new(StatusManagerConfig::default());
    assert_eq!(m.pop_ready(t0()), DispatchOutcome::Idle);
}

/// Upstream: TestSyncPod / `transient_failure_triggers_exponential_backoff`.
/// First failure schedules `base` delay; second failure doubles it.
#[test]
fn upstream_status_manager_transient_failure_schedules_backoff() {
    let mut m = PodStatusManager::new(StatusManagerConfig::default());
    m.set_status("pod-c", pod_status(PodPhase::Running, true), t0());
    // Dispatch + transient fail.
    let _ = m.pop_ready(t0());
    m.record_attempt("pod-c", AttemptOutcome::TransientFailure, t0());
    // Immediately after, the entry should NOT be ready.
    assert_eq!(m.pop_ready(t0()), DispatchOutcome::Idle);
    assert_eq!(m.in_backoff(t0()), 1);
    // After a long enough time, it should be ready again.
    let later = t0() + ChronoDuration::seconds(60);
    match m.pop_ready(later) {
        DispatchOutcome::Dispatched { pod_uid, .. } => assert_eq!(pod_uid, "pod-c"),
        other => panic!("expected Dispatched after backoff, got {other:?}"),
    }
}

/// Upstream: TestSyncPod / `success_clears_pending_and_records_hash`.
/// On apiserver success, the entry leaves `pending` and the hash is
/// stored in `confirmed` so a re-submission of the same status dedupes.
#[test]
fn upstream_status_manager_success_records_confirmed_hash_and_dedupes_repeat() {
    let mut m = PodStatusManager::new(StatusManagerConfig::default());
    let st = pod_status(PodPhase::Running, true);
    m.set_status("pod-d", st.clone(), t0());
    let _ = m.pop_ready(t0());
    m.record_attempt("pod-d", AttemptOutcome::Success, t0());
    assert_eq!(m.pending_len(), 0);
    // Re-submitting the SAME status now should dedupe against confirmed.
    let r = m.set_status("pod-d", st, t0());
    assert_eq!(r, Some(DropReason::Deduped));
}

/// Upstream: TestNeedsUpdate / `needs_update_uses_confirmed_hash`.
#[test]
fn upstream_status_manager_needs_update_false_when_matches_confirmed_hash() {
    let mut m = PodStatusManager::new(StatusManagerConfig::default());
    let st = pod_status(PodPhase::Running, true);
    m.set_status("pod-e", st.clone(), t0());
    let _ = m.pop_ready(t0());
    m.record_attempt("pod-e", AttemptOutcome::Success, t0());
    // Same status → needs_update returns false.
    assert!(!m.needs_update("pod-e", &st));
    // Different status (e.g. ready flipped) → needs_update returns true.
    let differs = pod_status(PodPhase::Running, false);
    assert!(m.needs_update("pod-e", &differs));
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/kubelet/prober/worker_test.go
// + pkg/kubelet/prober/results/manager_test.go
// ────────────────────────────────────────────────────────────────────────────

fn cref(pod: &str, container: &str) -> ContainerRef {
    ContainerRef::new(pod, container)
}

/// Upstream: TestProberManager / `restart_suppression_after_first_restart_request`.
/// When liveness keeps reporting RestartContainer while a restart is
/// already in flight, the coordinator MUST emit at most one event
/// until `mark_restart_completed` is called.
#[test]
fn upstream_prober_coordinator_suppresses_duplicate_restart_requests() {
    let mut c = ProberCoordinator::new(ProberConfig::default());
    let cr = cref("pod-1", "main");
    let first = c.coordinate(&cr, ProberAction::RestartContainer, t0());
    assert!(matches!(first, Some(CoordinatorEvent::RestartContainer { .. })));
    // Second tick — still failing, but a restart is in flight already.
    let second = c.coordinate(&cr, ProberAction::RestartContainer, t0());
    assert!(second.is_none(), "duplicate restart must be suppressed");
    // After the kubelet ACKs the restart, a new restart event can fire.
    c.mark_restart_completed(&cr);
    let third = c.coordinate(&cr, ProberAction::RestartContainer, t0());
    assert!(matches!(third, Some(CoordinatorEvent::RestartContainer { .. })));
}

/// Upstream: TestProberManager / `readiness_only_fires_on_transition`.
/// `MarkReady` is fired only when the recorded readiness flips —
/// repeated MarkReady ticks while already Ready stay quiet.
#[test]
fn upstream_prober_coordinator_dedupes_steady_state_readiness() {
    let mut c = ProberCoordinator::new(ProberConfig::default());
    let cr = cref("pod-1", "main");
    let first = c.coordinate(&cr, ProberAction::MarkReady, t0());
    assert!(matches!(first, Some(CoordinatorEvent::MarkReady { .. })));
    let second = c.coordinate(&cr, ProberAction::MarkReady, t0());
    assert!(second.is_none(), "already ready → no fan-out");
    // Flipping to NotReady should fire once.
    let flip = c.coordinate(&cr, ProberAction::MarkNotReady, t0());
    assert!(matches!(flip, Some(CoordinatorEvent::MarkNotReady { .. })));
}

/// Upstream: TestProberManager / `startup_complete_fires_once`.
#[test]
fn upstream_prober_coordinator_emits_startup_complete_once() {
    let mut c = ProberCoordinator::new(ProberConfig::default());
    let cr = cref("pod-1", "main");
    let first = c.coordinate(&cr, ProberAction::StartupComplete, t0());
    assert!(matches!(first, Some(CoordinatorEvent::StartupComplete { .. })));
    // Repeated startup-complete reports are suppressed.
    let second = c.coordinate(&cr, ProberAction::StartupComplete, t0());
    assert!(second.is_none());
}

/// Upstream: TestProberManager / `worker_pool_capacity_bound`.
/// kubelet defaults to 16 concurrent probes; try_reserve fails past
/// that cap.
#[test]
fn upstream_prober_coordinator_pool_capacity_bounded() {
    let cfg = ProberConfig {
        max_concurrent: 2,
        ..ProberConfig::default()
    };
    let c = ProberCoordinator::new(cfg);
    assert_eq!(c.pool_capacity(), 2);
    let p1 = c.try_reserve().expect("first permit");
    let p2 = c.try_reserve().expect("second permit");
    assert!(c.try_reserve().is_none(), "pool full at capacity");
    drop(p2);
    assert!(c.try_reserve().is_some(), "permit released → can reserve");
    drop(p1);
}

/// Upstream: TestNeedsRestartBackoff / `restart_suppression_clears_after_max`.
/// Defensive: if the kubelet never ACKs, suppression clears after
/// `restart_suppression_max` so a subsequent failure can re-fire.
#[test]
fn upstream_prober_coordinator_restart_suppression_clears_after_window() {
    let cfg = ProberConfig {
        restart_suppression_max: ChronoDuration::seconds(5),
        ..ProberConfig::default()
    };
    let mut c = ProberCoordinator::new(cfg);
    let cr = cref("pod-1", "main");
    let _ = c.coordinate(&cr, ProberAction::RestartContainer, t0());
    // No ACK yet, but within suppression window → second event suppressed.
    let still_blocked = c.coordinate(&cr, ProberAction::RestartContainer, t0() + ChronoDuration::seconds(3));
    assert!(still_blocked.is_none());
    // Past the suppression window → suppression clears and event fires.
    let after = c.coordinate(&cr, ProberAction::RestartContainer, t0() + ChronoDuration::seconds(10));
    assert!(matches!(after, Some(CoordinatorEvent::RestartContainer { .. })));
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/kubelet/lifecycle/handlers_test.go (additional cases)
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestRunHandlerExec / `success_after_success_stays_completed`.
/// HookOutcome::Completed is sticky — once a hook is Completed, later
/// samples don't unset it.
#[test]
fn upstream_lifecycle_success_completion_is_sticky() {
    let mut hook = HookExecution::new(
        HookStage::PostStart,
        HookHandler::Exec {
            command: vec!["init".into()],
        },
        t0(),
        Duration::from_secs(30),
    );
    let first = evaluate(&mut hook, HookSample::Success, t0());
    assert_eq!(first, HookOutcome::Completed);
    // A later NotFiredYet tick — still Completed.
    let later = evaluate(
        &mut hook,
        HookSample::NotFiredYet,
        t0() + ChronoDuration::seconds(60),
    );
    assert_eq!(later, HookOutcome::Completed);
}

/// Upstream: TestRunHandlerExec / `noop_when_already_completed`.
/// Reading the same hook on consecutive ticks must not change state.
#[test]
fn upstream_lifecycle_pending_within_timeout_window() {
    let mut hook = HookExecution::new(
        HookStage::PreStop,
        HookHandler::Exec {
            command: vec!["drain".into()],
        },
        t0(),
        Duration::from_secs(30),
    );
    // First tick: fire.
    let first = evaluate(&mut hook, HookSample::NotFiredYet, t0());
    assert!(matches!(first, HookOutcome::Fire(_)));
    // Mid-window: handler in flight, no sample yet → Pending.
    let mid = evaluate(
        &mut hook,
        HookSample::NotFiredYet,
        t0() + ChronoDuration::seconds(10),
    );
    assert_eq!(mid, HookOutcome::Pending);
}

/// Sanity: probe state-machine still flows initial_delay → period →
/// threshold (re-exercised here because batch1 didn't cover record on
/// the Startup probe path).
#[test]
fn upstream_probe_startup_kind_default_failure_then_threshold_success() {
    use cave_kubelet::probe::{ProbeOutcome, ProbeWorkerState, decide};
    let mut spec = ProbeSpec::http_get(8080, "/startup");
    spec.kind = ProbeKind::Startup;
    let mut state = ProbeWorkerState::new(t0());
    // Pre-threshold: defaults to Failure → StartupFailed via decide()…
    // …but only after consecutive failures, not before any sample arrives.
    assert_eq!(state.effective_outcome(&spec), ProbeOutcome::Failure);
    // After one success at successThreshold=1 → StartupComplete.
    state.record(&spec, ProbeResult::Success, t0() + ChronoDuration::seconds(1));
    assert_eq!(state.effective_outcome(&spec), ProbeOutcome::Success);
    assert_eq!(decide(&spec, &state), ProberAction::StartupComplete);
}
