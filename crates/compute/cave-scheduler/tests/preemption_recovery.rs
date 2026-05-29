// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! TDD — failing test for preemption-victim-recovery loop.
//!
//! Upstream reference:
//!   kubernetes/kubernetes v1.36.0
//!   pkg/scheduler/framework/preemption/preemption.go  (postFilter, handlePreemptionResult)
//!
//! The gap: when an apiserver eviction call fails transiently, the scheduler
//! must re-admit those victims (restore them to the scheduling queue) so their
//! resources are not silently leaked.  The existing AsyncPreemptHandle tracks
//! pending/completed evictions, but there is no mechanism to:
//!   1. Record an eviction as *failed* (as opposed to completed successfully).
//!   2. Retrieve failed-eviction victim metadata for re-admission.
//!   3. Clear the nomination for a preemptor whose eviction window permanently
//!      fails (so a subsequent scheduling cycle can nominate a different node).
//!
//! This test file drives the `EvictionRecoveryLoop` type that must be added to
//! `cave_scheduler::default_preemption`.

use cave_scheduler::default_preemption::{
    AsyncPreemptHandle, EvictionRecoveryLoop, EvictionTask, NominatedNodeMap,
};
use std::sync::Arc;

/// Helper: build an eviction task.
fn task(victim_uid: &str, node: &str, preemptor_uid: &str) -> EvictionTask {
    EvictionTask {
        victim_uid: victim_uid.into(),
        victim_namespace: "ns".into(),
        victim_name: victim_uid.into(),
        node_name: node.into(),
        preemptor_uid: preemptor_uid.into(),
    }
}

// ── Failure recording ────────────────────────────────────────────────────────

/// A task marked failed must appear in `failed_evictions()`.
#[test]
fn failed_eviction_recorded_in_handle() {
    let handle = Arc::new(AsyncPreemptHandle::new());
    let t = task("v1", "node-a", "p1");
    handle.enqueue(t.clone());
    handle.mark_failed("v1");
    let failed = handle.failed_evictions();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].victim_uid, "v1");
}

/// A task that was dequeued (completed successfully) must NOT appear in failed.
#[test]
fn successfully_completed_eviction_not_in_failed() {
    let handle = Arc::new(AsyncPreemptHandle::new());
    let t = task("v2", "node-a", "p1");
    handle.enqueue(t);
    handle.dequeue(); // success path
    assert!(handle.failed_evictions().is_empty());
}

/// mark_failed on an unknown uid is a no-op (idempotent).
#[test]
fn mark_failed_unknown_uid_is_noop() {
    let handle = Arc::new(AsyncPreemptHandle::new());
    handle.mark_failed("does-not-exist"); // must not panic
    assert!(handle.failed_evictions().is_empty());
}

// ── Recovery loop ────────────────────────────────────────────────────────────

/// When a victim's eviction fails, the recovery loop re-admits the victim
/// (adds it back to the re-admit list) and clears the preemptor's nomination.
#[test]
fn recovery_loop_readmits_victims_and_clears_nomination() {
    let handle = Arc::new(AsyncPreemptHandle::new());
    let nominated = Arc::new(NominatedNodeMap::new());

    // Preemptor p1 was nominated for node-a, victim v1 was queued for eviction.
    nominated.nominate("p1", "node-a");
    let t = task("v1", "node-a", "p1");
    handle.enqueue(t);

    // Simulate: eviction RPC to apiserver returned a transient error.
    handle.mark_failed("v1");

    // Run the recovery loop.
    let recovery = EvictionRecoveryLoop::new(handle.clone(), nominated.clone());
    let readmitted = recovery.drain_failed();

    // Victim must be returned for re-admission.
    assert_eq!(readmitted.len(), 1);
    assert_eq!(readmitted[0].victim_uid, "v1");

    // Preemptor's nomination must be cleared so the next cycle can pick a
    // different node.
    assert!(
        nominated.nominated_for("p1").is_none(),
        "nomination must be cleared after eviction failure"
    );

    // failed_evictions list must be drained after drain_failed().
    assert!(handle.failed_evictions().is_empty());
}

/// Multiple victims across different preemptors are all recovered.
#[test]
fn recovery_loop_handles_multiple_victims_across_preemptors() {
    let handle = Arc::new(AsyncPreemptHandle::new());
    let nominated = Arc::new(NominatedNodeMap::new());

    nominated.nominate("p1", "node-a");
    nominated.nominate("p2", "node-b");

    handle.enqueue(task("v1", "node-a", "p1"));
    handle.enqueue(task("v2", "node-a", "p1"));
    handle.enqueue(task("v3", "node-b", "p2"));

    handle.mark_failed("v1");
    handle.mark_failed("v3");
    // v2 succeeds (dequeued normally — never marked failed).
    handle.dequeue(); // dequeues v1 from pending
    // Actually dequeue v2 (v1 already marked failed and removed from pending):
    // We re-think: mark_failed should move from pending to failed without
    // requiring a dequeue call. Let's verify the semantics.

    let recovery = EvictionRecoveryLoop::new(handle.clone(), nominated.clone());
    let readmitted = recovery.drain_failed();

    // v1 and v3 failed → both readmitted.
    let uids: std::collections::HashSet<&str> =
        readmitted.iter().map(|t| t.victim_uid.as_str()).collect();
    assert!(uids.contains("v1"), "v1 must be readmitted");
    assert!(uids.contains("v3"), "v3 must be readmitted");

    // Nominations for p1 and p2 cleared.
    assert!(nominated.nominated_for("p1").is_none());
    assert!(nominated.nominated_for("p2").is_none());
}

/// drain_failed is idempotent: calling it twice returns empty on second call.
#[test]
fn drain_failed_is_idempotent_second_call_empty() {
    let handle = Arc::new(AsyncPreemptHandle::new());
    let nominated = Arc::new(NominatedNodeMap::new());
    nominated.nominate("p1", "node-a");
    handle.enqueue(task("v1", "node-a", "p1"));
    handle.mark_failed("v1");

    let recovery = EvictionRecoveryLoop::new(handle.clone(), nominated.clone());
    let first = recovery.drain_failed();
    assert_eq!(first.len(), 1);

    let second = recovery.drain_failed();
    assert!(second.is_empty(), "second drain must return nothing");
}

/// When all evictions succeed (none marked failed), drain_failed returns empty
/// and no nominations are touched.
#[test]
fn no_failures_drain_returns_empty_nominations_intact() {
    let handle = Arc::new(AsyncPreemptHandle::new());
    let nominated = Arc::new(NominatedNodeMap::new());
    nominated.nominate("p1", "node-a");

    handle.enqueue(task("v1", "node-a", "p1"));
    handle.dequeue(); // success

    let recovery = EvictionRecoveryLoop::new(handle.clone(), nominated.clone());
    let readmitted = recovery.drain_failed();
    assert!(readmitted.is_empty());
    // Nomination NOT cleared (eviction succeeded).
    assert_eq!(nominated.nominated_for("p1").as_deref(), Some("node-a"));
}
