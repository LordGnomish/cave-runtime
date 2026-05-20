// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Line-by-line ports of upstream kubelet tests, cross-referenced
//! from `parity.manifest.toml`'s `[[upstream_test]]` block.
//!
//! Upstream: kubernetes/kubernetes @ v1.36.0
//!   * pkg/kubelet/prober/{prober_test.go,worker_test.go,results/*_test.go}
//!   * pkg/kubelet/preemption/preemption_test.go
//!   * pkg/kubelet/lifecycle/handlers_test.go
//!
//! Subtests (Go `t.Run`) split into individual `#[test]` fns. Each
//! test asserts the same input → output equivalence class as the
//! upstream case it ports.

use cave_kubelet::lifecycle::{
    evaluate, HookExecution, HookHandler, HookOutcome, HookSample, HookStage, HttpScheme,
};
use cave_kubelet::preemption::{
    evaluate as preempt_evaluate, CandidatePod, PreemptionDecision, PreemptionRequest,
    ResourceRequest,
};
use cave_kubelet::probe::{
    decide, exec_exit_to_result, grpc_status_to_result, http_status_to_result, GrpcServingStatus,
    ProbeKind, ProbeOutcome, ProbeResult, ProbeSpec, ProbeWorkerState, ProberAction,
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use std::time::Duration;

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-05-13T10:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/kubelet/prober/prober_test.go +
//           pkg/kubelet/prober/results/manager_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestStatusToResult / HTTP 2xx → Success.
#[test]
fn upstream_probe_http_status_2xx_success() {
    assert_eq!(http_status_to_result(200), ProbeResult::Success);
    assert_eq!(http_status_to_result(299), ProbeResult::Success);
}

/// Upstream: TestStatusToResult / HTTP 3xx → Success (redirect counted as
/// healthy upstream).
#[test]
fn upstream_probe_http_3xx_counted_as_success() {
    assert_eq!(http_status_to_result(301), ProbeResult::Success);
    assert_eq!(http_status_to_result(399), ProbeResult::Success);
}

/// Upstream: TestStatusToResult / HTTP 4xx or 5xx → Failure.
#[test]
fn upstream_probe_http_4xx_5xx_failure() {
    assert_eq!(http_status_to_result(404), ProbeResult::Failure);
    assert_eq!(http_status_to_result(503), ProbeResult::Failure);
    assert_eq!(http_status_to_result(100), ProbeResult::Failure);
}

/// Upstream: TestGrpcProbe / SERVING → Success; NOT_SERVING → Failure.
#[test]
fn upstream_probe_grpc_serving_to_success_not_serving_to_failure() {
    assert_eq!(
        grpc_status_to_result(GrpcServingStatus::Serving),
        ProbeResult::Success
    );
    assert_eq!(
        grpc_status_to_result(GrpcServingStatus::NotServing),
        ProbeResult::Failure
    );
}

/// Upstream: TestExecResult / nonzero exit code → Failure; 0 → Success.
#[test]
fn upstream_probe_exec_zero_success_nonzero_failure() {
    assert_eq!(exec_exit_to_result(0), ProbeResult::Success);
    assert_eq!(exec_exit_to_result(1), ProbeResult::Failure);
    assert_eq!(exec_exit_to_result(127), ProbeResult::Failure);
}

/// Upstream: prober/worker_test.go / `failureThreshold consecutive failures
/// → outcome flips to Failure`.
#[test]
fn upstream_probe_worker_failure_threshold_flips_to_failure() {
    let spec = ProbeSpec {
        failure_threshold: 3,
        ..ProbeSpec::http_get(8080, "/healthz")
    };
    let mut state = ProbeWorkerState::new(t0());
    state.record(
        &spec,
        ProbeResult::Failure,
        t0() + ChronoDuration::seconds(10),
    );
    state.record(
        &spec,
        ProbeResult::Failure,
        t0() + ChronoDuration::seconds(20),
    );
    // Two failures: still success (default-success liveness pre-threshold).
    assert_eq!(state.effective_outcome(&spec), ProbeOutcome::Success);
    state.record(
        &spec,
        ProbeResult::Failure,
        t0() + ChronoDuration::seconds(30),
    );
    assert_eq!(state.effective_outcome(&spec), ProbeOutcome::Failure);
}

/// Upstream: prober/worker_test.go / `success after failure resets counter`.
#[test]
fn upstream_probe_worker_success_resets_failure_counter() {
    let spec = ProbeSpec {
        failure_threshold: 3,
        ..ProbeSpec::http_get(8080, "/healthz")
    };
    let mut state = ProbeWorkerState::new(t0());
    state.record(
        &spec,
        ProbeResult::Failure,
        t0() + ChronoDuration::seconds(10),
    );
    state.record(
        &spec,
        ProbeResult::Failure,
        t0() + ChronoDuration::seconds(20),
    );
    state.record(
        &spec,
        ProbeResult::Success,
        t0() + ChronoDuration::seconds(30),
    );
    assert_eq!(state.consecutive_failure, 0);
    assert_eq!(state.consecutive_success, 1);
}

/// Upstream: prober/results/manager_test.go / readiness default-failure
/// pre-threshold (so endpoints don't include not-yet-ready pods).
#[test]
fn upstream_probe_readiness_defaults_to_failure_pre_threshold() {
    let mut spec = ProbeSpec::http_get(8080, "/ready");
    spec.kind = ProbeKind::Readiness;
    let state = ProbeWorkerState::new(t0());
    assert_eq!(state.effective_outcome(&spec), ProbeOutcome::Failure);
    assert_eq!(decide(&spec, &state), ProberAction::MarkNotReady);
}

/// Upstream: TestDoProbe / `initialDelaySeconds blocks probe firing`.
#[test]
fn upstream_probe_initial_delay_blocks_fire() {
    let spec = ProbeSpec {
        initial_delay_seconds: 30,
        ..ProbeSpec::http_get(8080, "/")
    };
    let state = ProbeWorkerState::new(t0());
    assert!(!state.should_fire(&spec, t0() + ChronoDuration::seconds(10)));
    assert!(state.should_fire(&spec, t0() + ChronoDuration::seconds(30)));
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/kubelet/preemption/preemption_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestEvictPodsToFreeRequests / `node already has room → admit
/// without victims`.
#[test]
fn upstream_preemption_admit_when_node_already_has_room() {
    let req = PreemptionRequest {
        incoming_uid: "new".into(),
        incoming_priority: 1000,
        incoming_resources: ResourceRequest {
            cpu_millicores: 100,
            memory_bytes: 100,
        },
        node_free: ResourceRequest {
            cpu_millicores: 1000,
            memory_bytes: 1000,
        },
        candidates: vec![],
    };
    assert_eq!(preempt_evaluate(&req), PreemptionDecision::AdmitNoVictims);
}

/// Upstream: TestEvictPodsToFreeRequests / `lowest-priority pod is evicted
/// first to free CPU`.
#[test]
fn upstream_preemption_picks_lowest_priority_victim_first() {
    let candidates = vec![
        CandidatePod {
            uid: "high".into(),
            name: "high".into(),
            priority: 900,
            resources: ResourceRequest {
                cpu_millicores: 500,
                memory_bytes: 500,
            },
        },
        CandidatePod {
            uid: "low".into(),
            name: "low".into(),
            priority: 100,
            resources: ResourceRequest {
                cpu_millicores: 500,
                memory_bytes: 500,
            },
        },
    ];
    let req = PreemptionRequest {
        incoming_uid: "new".into(),
        incoming_priority: 1000,
        incoming_resources: ResourceRequest {
            cpu_millicores: 600,
            memory_bytes: 100,
        },
        node_free: ResourceRequest {
            cpu_millicores: 100,
            memory_bytes: 1000,
        },
        candidates,
    };
    match preempt_evaluate(&req) {
        PreemptionDecision::Evict { victim_uids } => {
            assert_eq!(victim_uids, vec!["low".to_string()]);
        }
        other => panic!("expected Evict, got {other:?}"),
    }
}

/// Upstream: TestEvictPodsToFreeRequests / `no lower-priority pods can
/// cover the deficit → Insufficient`.
#[test]
fn upstream_preemption_returns_insufficient_when_no_lower_priority_pods_help() {
    let candidates = vec![CandidatePod {
        uid: "tiny".into(),
        name: "tiny".into(),
        priority: 100,
        resources: ResourceRequest {
            cpu_millicores: 50,
            memory_bytes: 50,
        },
    }];
    let req = PreemptionRequest {
        incoming_uid: "new".into(),
        incoming_priority: 1000,
        incoming_resources: ResourceRequest {
            cpu_millicores: 1000,
            memory_bytes: 1000,
        },
        node_free: ResourceRequest {
            cpu_millicores: 0,
            memory_bytes: 0,
        },
        candidates,
    };
    match preempt_evaluate(&req) {
        PreemptionDecision::Insufficient { reason } => {
            assert!(
                reason.contains("deficit"),
                "expected reason to mention deficit, got: {reason}"
            );
        }
        other => panic!("expected Insufficient, got {other:?}"),
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/kubelet/lifecycle/handlers_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestRunHandlerExec / `successful exec marks hook completed`.
#[test]
fn upstream_lifecycle_exec_success_marks_completed() {
    let mut hook = HookExecution::new(
        HookStage::PreStop,
        HookHandler::Exec {
            command: vec!["/bin/sh".into(), "-c".into(), "/bin/drain.sh".into()],
        },
        t0(),
        Duration::from_secs(30),
    );
    let outcome = evaluate(&mut hook, HookSample::Success, t0());
    assert_eq!(outcome, HookOutcome::Completed);
    assert!(hook.completed);
}

/// Upstream: TestRunHandlerHttp / `httpGet handler executes against pod IP +
/// port`. cave equivalent: serde round-trip preserves shape (transport not
/// in scope of the pure decision module).
#[test]
fn upstream_lifecycle_http_handler_serde_round_trips() {
    let h = HookHandler::HttpGet {
        port: 8080,
        path: "/quitquitquit".into(),
        scheme: HttpScheme::Http,
    };
    let json = serde_json::to_string(&h).unwrap();
    let back: HookHandler = serde_json::from_str(&json).unwrap();
    assert_eq!(h, back);
}

/// Upstream: TestRunHandlerExec / `non-zero exit → hook records failure`.
#[test]
fn upstream_lifecycle_exec_failure_records_reason_and_sticky() {
    let mut hook = HookExecution::new(
        HookStage::PostStart,
        HookHandler::Exec {
            command: vec!["init".into()],
        },
        t0(),
        Duration::from_secs(10),
    );
    let outcome = evaluate(&mut hook, HookSample::Failure, t0());
    assert!(matches!(outcome, HookOutcome::Failed { .. }));
    // Sticky: later success does NOT clear the failure.
    let outcome2 = evaluate(&mut hook, HookSample::Success, t0());
    assert!(matches!(outcome2, HookOutcome::Failed { .. }));
}

/// Upstream: TestPreStopTimeout / `handler not completed within timeout →
/// TimedOut`.
#[test]
fn upstream_lifecycle_pre_stop_timeout_fires_at_boundary() {
    let mut hook = HookExecution::new(
        HookStage::PreStop,
        HookHandler::Exec {
            command: vec!["drain".into()],
        },
        t0(),
        Duration::from_secs(30),
    );
    // First tick: handler fires.
    let _ = evaluate(&mut hook, HookSample::NotFiredYet, t0());
    // 31 s later, still no sample → timeout.
    let late = t0() + ChronoDuration::seconds(31);
    let outcome = evaluate(&mut hook, HookSample::NotFiredYet, late);
    assert!(matches!(outcome, HookOutcome::TimedOut { .. }));
}
