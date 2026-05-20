// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Batch 4 (2026-05-14) — upstream test port for PLEG
//! (PodLifecycleEventGenerator).
//!
//! Upstream: kubernetes/kubernetes @ v1.36.0
//!   * pkg/kubelet/pleg/generic_test.go
//!   * pkg/kubelet/pleg/generic.go
//!   * pkg/kubelet/pleg/pleg.go
//!
//! PLEG is the loop that polls the runtime's pod list and emits a
//! `PodLifecycleEvent` stream (ContainerStarted / ContainerDied /
//! ContainerRemoved / ContainerChanged / PodSync) by diffing the
//! previous snapshot against the new one.
//!
//! These tests are line-by-line ports of upstream Go tests. Each
//! `#[test]` carries an `Upstream:` doc-comment naming the source
//! file, the Go test function, and the line range it ports.

use cave_kubelet::pleg::{
    ContainerState, ContainerStatus, GenericPleg, PodLifecycleEvent, PodLifecycleEventType,
    PodSnapshot, RelistOutcome,
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};

// ────────────────────────────────────────────────────────────────────────────
// Helpers (mirror upstream pleg/generic_test.go lines 48-72)
// ────────────────────────────────────────────────────────────────────────────

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-05-14T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

/// Mirrors upstream `createTestContainer(id, state)`
/// (generic_test.go:67-72).
fn c(id: &str, state: ContainerState) -> ContainerStatus {
    ContainerStatus {
        id: id.to_string(),
        state,
    }
}

fn pod(uid: &str, containers: Vec<ContainerStatus>) -> PodSnapshot {
    PodSnapshot {
        uid: uid.to_string(),
        containers,
        sandboxes: Vec::new(),
        ip: None,
    }
}

fn pod_with_sandboxes(uid: &str, sandboxes: Vec<ContainerStatus>) -> PodSnapshot {
    PodSnapshot {
        uid: uid.to_string(),
        containers: Vec::new(),
        sandboxes,
        ip: None,
    }
}

fn pod_with_ip(uid: &str, containers: Vec<ContainerStatus>, ip: &str) -> PodSnapshot {
    PodSnapshot {
        uid: uid.to_string(),
        containers,
        sandboxes: Vec::new(),
        ip: Some(ip.to_string()),
    }
}

/// Multiset comparison — upstream's `verifyEvents` is order-insensitive
/// because the per-pod iteration order in Go is map-iteration random.
fn verify_events(expected: &[PodLifecycleEvent], actual: &[PodLifecycleEvent]) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "event count mismatch:\n  expected = {:#?}\n  actual   = {:#?}",
        expected,
        actual
    );
    for e in expected {
        let count_e = expected.iter().filter(|x| *x == e).count();
        let count_a = actual.iter().filter(|x| *x == e).count();
        assert_eq!(
            count_a, count_e,
            "event {:?} missing from actual\n  expected = {:#?}\n  actual   = {:#?}",
            e, expected, actual
        );
    }
}

fn ev(uid: &str, ty: PodLifecycleEventType, data: &str) -> PodLifecycleEvent {
    PodLifecycleEvent {
        pod_uid: uid.to_string(),
        event_type: ty,
        container_id: Some(data.to_string()),
        data: Some(data.to_string()),
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: kubernetes/kubernetes@v1.36.0
///   pkg/kubelet/pleg/generic_test.go::TestRelisting (lines 233-310)
/// First relist emits Started for Running and Died for Exited;
/// Unknown is silent (treated as "no transition from non-existent
/// to unknown" — upstream emits ContainerChanged on that edge but
/// this first relist case in upstream verifies *only* the three
/// concrete events).
#[test]
fn upstream_relisting_first_pass_emits_started_and_died() {
    let mut pleg = GenericPleg::new(1024);
    let pods = vec![
        pod(
            "1234",
            vec![
                c("c1", ContainerState::Exited),
                c("c2", ContainerState::Running),
                c("c3", ContainerState::Unknown),
            ],
        ),
        pod("4567", vec![c("c1", ContainerState::Exited)]),
    ];
    let out = pleg.relist(t0(), pods);
    let expected = vec![
        ev("1234", PodLifecycleEventType::ContainerStarted, "c2"),
        ev("1234", PodLifecycleEventType::ContainerDied, "c1"),
        ev("1234", PodLifecycleEventType::ContainerChanged, "c3"),
        ev("4567", PodLifecycleEventType::ContainerDied, "c1"),
    ];
    verify_events(&expected, &out.events);
    assert_eq!(out.dropped, 0);
}

/// Upstream: kubernetes/kubernetes@v1.36.0
///   pkg/kubelet/pleg/generic_test.go::TestRelisting (lines 233-310)
/// Second relist with identical snapshot emits no events.
#[test]
fn upstream_relisting_second_pass_no_change_no_events() {
    let mut pleg = GenericPleg::new(1024);
    let p1 = vec![
        pod(
            "1234",
            vec![
                c("c1", ContainerState::Exited),
                c("c2", ContainerState::Running),
                c("c3", ContainerState::Unknown),
            ],
        ),
        pod("4567", vec![c("c1", ContainerState::Exited)]),
    ];
    pleg.relist(t0(), p1.clone());
    let out = pleg.relist(t0() + ChronoDuration::seconds(1), p1);
    assert!(out.events.is_empty(), "no change => no events");
}

/// Upstream: kubernetes/kubernetes@v1.36.0
///   pkg/kubelet/pleg/generic_test.go::TestRelisting (lines 233-310)
/// Third relist after container set has shifted: removed containers,
/// state transitions, and newly running containers all emit.
/// Upstream rule: previously-exited then non-existent => ContainerRemoved
/// alone; previously-running then non-existent => ContainerDied +
/// ContainerRemoved.
#[test]
fn upstream_relisting_third_pass_remove_and_transition() {
    let mut pleg = GenericPleg::new(1024);
    let p1 = vec![
        pod(
            "1234",
            vec![
                c("c1", ContainerState::Exited),
                c("c2", ContainerState::Running),
                c("c3", ContainerState::Unknown),
            ],
        ),
        pod("4567", vec![c("c1", ContainerState::Exited)]),
    ];
    pleg.relist(t0(), p1);
    let p2 = vec![
        pod(
            "1234",
            vec![
                c("c2", ContainerState::Exited),
                c("c3", ContainerState::Running),
            ],
        ),
        pod("4567", vec![c("c4", ContainerState::Running)]),
    ];
    let out = pleg.relist(t0() + ChronoDuration::seconds(2), p2);
    let expected = vec![
        ev("1234", PodLifecycleEventType::ContainerRemoved, "c1"),
        ev("1234", PodLifecycleEventType::ContainerDied, "c2"),
        ev("1234", PodLifecycleEventType::ContainerStarted, "c3"),
        ev("4567", PodLifecycleEventType::ContainerRemoved, "c1"),
        ev("4567", PodLifecycleEventType::ContainerStarted, "c4"),
    ];
    verify_events(&expected, &out.events);
}

/// Upstream: kubernetes/kubernetes@v1.36.0
///   pkg/kubelet/pleg/generic_test.go::TestEventChannelFull
///   (lines 312-371)
/// When the buffer capacity is exceeded on a relist, upstream drops
/// new events past `cap` and records the loss. Verify that exactly
/// `cap` events are emitted and the dropped counter is the remainder.
#[test]
fn upstream_event_channel_full_drops_excess() {
    let mut pleg = GenericPleg::new(1024);
    // Pre-populate so the cap=4 channel is hit on the next relist.
    let p1 = vec![
        pod(
            "1234",
            vec![
                c("c1", ContainerState::Exited),
                c("c2", ContainerState::Running),
                c("c3", ContainerState::Unknown),
            ],
        ),
        pod("4567", vec![c("c1", ContainerState::Exited)]),
    ];
    let _ = pleg.relist(t0(), p1);

    // Reset capacity to 4 and prime the snapshot via prior relist.
    let mut bounded = GenericPleg::new(4);
    let p1 = vec![
        pod(
            "1234",
            vec![
                c("c1", ContainerState::Exited),
                c("c2", ContainerState::Running),
                c("c3", ContainerState::Unknown),
            ],
        ),
        pod("4567", vec![c("c1", ContainerState::Exited)]),
    ];
    // First relist would itself emit 4 events; consume by allowing
    // them to be emitted (cap is 4 → first relist fits exactly).
    let first = bounded.relist(t0(), p1);
    assert!(first.events.len() <= 4);
    let _drained_first = first.events.len();

    // Second relist generates 5 events, only 4 fit, 1 dropped.
    let p2 = vec![
        pod(
            "1234",
            vec![
                c("c2", ContainerState::Exited),
                c("c3", ContainerState::Running),
            ],
        ),
        pod("4567", vec![c("c4", ContainerState::Running)]),
    ];
    let out = bounded.relist(t0() + ChronoDuration::seconds(2), p2);
    assert_eq!(out.events.len(), 4, "events capped at channel capacity");
    assert_eq!(out.dropped, 1, "exactly one event dropped");
    let all_possible = vec![
        ev("1234", PodLifecycleEventType::ContainerRemoved, "c1"),
        ev("1234", PodLifecycleEventType::ContainerDied, "c2"),
        ev("1234", PodLifecycleEventType::ContainerStarted, "c3"),
        ev("4567", PodLifecycleEventType::ContainerRemoved, "c1"),
        ev("4567", PodLifecycleEventType::ContainerStarted, "c4"),
    ];
    for e in &out.events {
        assert!(all_possible.contains(e), "unexpected event {:?}", e);
    }
}

/// Upstream: kubernetes/kubernetes@v1.36.0
///   pkg/kubelet/pleg/generic_test.go::TestRelistingWithSandboxes
///   (lines 635-715)
/// Same diff semantics as containers but on sandbox state.
#[test]
fn upstream_relisting_sandboxes_first_pass() {
    let mut pleg = GenericPleg::new(1024);
    let pods = vec![
        pod_with_sandboxes(
            "1234",
            vec![
                c("c1", ContainerState::Exited),
                c("c2", ContainerState::Running),
                c("c3", ContainerState::Unknown),
            ],
        ),
        pod_with_sandboxes("4567", vec![c("c1", ContainerState::Exited)]),
    ];
    let out = pleg.relist(t0(), pods);
    let expected = vec![
        ev("1234", PodLifecycleEventType::ContainerStarted, "c2"),
        ev("1234", PodLifecycleEventType::ContainerDied, "c1"),
        ev("1234", PodLifecycleEventType::ContainerChanged, "c3"),
        ev("4567", PodLifecycleEventType::ContainerDied, "c1"),
    ];
    verify_events(&expected, &out.events);
}

/// Upstream: kubernetes/kubernetes@v1.36.0
///   pkg/kubelet/pleg/generic_test.go::TestRelistingWithSandboxes
///   (lines 635-715)
/// Third relist with sandbox set churn — must match the same
/// state-machine rules generateEvents uses for containers
/// (generic.go:214-235).
#[test]
fn upstream_relisting_sandboxes_third_pass_remove_and_transition() {
    let mut pleg = GenericPleg::new(1024);
    let p1 = vec![
        pod_with_sandboxes(
            "1234",
            vec![
                c("c1", ContainerState::Exited),
                c("c2", ContainerState::Running),
                c("c3", ContainerState::Unknown),
            ],
        ),
        pod_with_sandboxes("4567", vec![c("c1", ContainerState::Exited)]),
    ];
    pleg.relist(t0(), p1);
    let p2 = vec![
        pod_with_sandboxes(
            "1234",
            vec![
                c("c2", ContainerState::Exited),
                c("c3", ContainerState::Running),
            ],
        ),
        pod_with_sandboxes("4567", vec![c("c4", ContainerState::Running)]),
    ];
    let out = pleg.relist(t0() + ChronoDuration::seconds(2), p2);
    let expected = vec![
        ev("1234", PodLifecycleEventType::ContainerRemoved, "c1"),
        ev("1234", PodLifecycleEventType::ContainerDied, "c2"),
        ev("1234", PodLifecycleEventType::ContainerStarted, "c3"),
        ev("4567", PodLifecycleEventType::ContainerRemoved, "c1"),
        ev("4567", PodLifecycleEventType::ContainerStarted, "c4"),
    ];
    verify_events(&expected, &out.events);
}

/// Upstream: kubernetes/kubernetes@v1.36.0
///   pkg/kubelet/pleg/generic_test.go::TestRemoveCacheEntry
///   (lines 523-541)
/// Pod missing from second relist → cache entry purged.
#[test]
fn upstream_remove_cache_entry_when_pod_gone() {
    let mut pleg = GenericPleg::new(1024);
    let pods = vec![pod("test-pod", vec![c("c0", ContainerState::Running)])];
    pleg.relist(t0(), pods);
    assert!(pleg.cache_get("test-pod").is_some());
    // Pod disappeared.
    pleg.relist(t0() + ChronoDuration::seconds(1), vec![]);
    assert!(
        pleg.cache_get("test-pod").is_none(),
        "cache entry removed for missing pod"
    );
}

/// Upstream: kubernetes/kubernetes@v1.36.0
///   pkg/kubelet/pleg/generic_test.go::TestHealthy (lines 543-571)
/// Without any relist, pleg is unhealthy. After a relist within
/// `relistThreshold` (3 min default), it's healthy. Past threshold,
/// unhealthy again.
#[test]
fn upstream_healthy_only_within_relist_threshold() {
    let mut pleg = GenericPleg::new(1024);
    let now = t0();
    // No relist yet → unhealthy.
    assert!(!pleg.healthy(now));
    // Advance 10 min without relist → still unhealthy.
    assert!(!pleg.healthy(now + ChronoDuration::minutes(10)));
    // Relist + 1 min later → healthy.
    pleg.relist(now + ChronoDuration::minutes(11), vec![]);
    assert!(pleg.healthy(now + ChronoDuration::minutes(12)));
    // Past relistThreshold (3 min) → unhealthy.
    assert!(!pleg.healthy(now + ChronoDuration::minutes(20)));
}

/// Upstream: kubernetes/kubernetes@v1.36.0
///   pkg/kubelet/pleg/generic_test.go::TestRelistIPChange
///   (lines 813-888)
/// First relist with a running container records IP in the cache;
/// subsequent relist where the container has exited preserves the
/// previously-cached IP rather than wiping it.
#[test]
fn upstream_relist_ip_change_preserves_previous_ip_on_exit() {
    let mut pleg = GenericPleg::new(1024);
    let pods = vec![pod_with_ip(
        "test-pod-0",
        vec![c("c0", ContainerState::Running)],
        "192.168.1.5",
    )];
    let out = pleg.relist(t0(), pods);
    let expected = vec![ev(
        "test-pod-0",
        PodLifecycleEventType::ContainerStarted,
        "c0",
    )];
    verify_events(&expected, &out.events);
    let cached = pleg.cache_get("test-pod-0").expect("cached entry");
    assert_eq!(cached.ip.as_deref(), Some("192.168.1.5"));

    // Now container exits and snapshot reports no IP — upstream
    // preserves the previously-cached IP.
    let pods2 = vec![pod("test-pod-0", vec![c("c0", ContainerState::Exited)])];
    let out2 = pleg.relist(t0() + ChronoDuration::seconds(1), pods2);
    let expected2 = vec![ev("test-pod-0", PodLifecycleEventType::ContainerDied, "c0")];
    verify_events(&expected2, &out2.events);
    let cached2 = pleg.cache_get("test-pod-0").expect("cached entry");
    assert_eq!(
        cached2.ip.as_deref(),
        Some("192.168.1.5"),
        "previous IP retained after container exit"
    );
}

/// Upstream: kubernetes/kubernetes@v1.36.0
///   pkg/kubelet/pleg/generic.go::generateEvents (lines 214-235)
/// Direct state-transition test: running → exited emits ContainerDied
/// only (not paired with Removed).
#[test]
fn upstream_generate_events_running_to_exited_emits_died() {
    let mut pleg = GenericPleg::new(1024);
    pleg.relist(t0(), vec![pod("p", vec![c("c", ContainerState::Running)])]);
    let out = pleg.relist(
        t0() + ChronoDuration::seconds(1),
        vec![pod("p", vec![c("c", ContainerState::Exited)])],
    );
    verify_events(
        &[ev("p", PodLifecycleEventType::ContainerDied, "c")],
        &out.events,
    );
}

/// Upstream: kubernetes/kubernetes@v1.36.0
///   pkg/kubelet/pleg/generic.go::generateEvents (lines 214-235)
/// Per upstream switch: running → non_existent emits both
/// ContainerDied and ContainerRemoved.
#[test]
fn upstream_generate_events_running_to_nonexistent_emits_died_and_removed() {
    let mut pleg = GenericPleg::new(1024);
    pleg.relist(t0(), vec![pod("p", vec![c("c", ContainerState::Running)])]);
    let out = pleg.relist(t0() + ChronoDuration::seconds(1), vec![pod("p", vec![])]);
    verify_events(
        &[
            ev("p", PodLifecycleEventType::ContainerDied, "c"),
            ev("p", PodLifecycleEventType::ContainerRemoved, "c"),
        ],
        &out.events,
    );
}

/// Upstream: kubernetes/kubernetes@v1.36.0
///   pkg/kubelet/pleg/generic.go::generateEvents (lines 214-235)
/// Exited → non_existent emits ContainerRemoved alone (we already
/// reported the death on a prior relist).
#[test]
fn upstream_generate_events_exited_to_nonexistent_emits_removed_only() {
    let mut pleg = GenericPleg::new(1024);
    pleg.relist(t0(), vec![pod("p", vec![c("c", ContainerState::Exited)])]);
    let out = pleg.relist(t0() + ChronoDuration::seconds(1), vec![pod("p", vec![])]);
    verify_events(
        &[ev("p", PodLifecycleEventType::ContainerRemoved, "c")],
        &out.events,
    );
}

/// Upstream: kubernetes/kubernetes@v1.36.0
///   pkg/kubelet/pleg/generic.go::generateEvents (lines 214-235)
/// running → unknown emits ContainerChanged.
#[test]
fn upstream_generate_events_running_to_unknown_emits_changed() {
    let mut pleg = GenericPleg::new(1024);
    pleg.relist(t0(), vec![pod("p", vec![c("c", ContainerState::Running)])]);
    let out = pleg.relist(
        t0() + ChronoDuration::seconds(1),
        vec![pod("p", vec![c("c", ContainerState::Unknown)])],
    );
    verify_events(
        &[ev("p", PodLifecycleEventType::ContainerChanged, "c")],
        &out.events,
    );
}

/// Upstream: kubernetes/kubernetes@v1.36.0
///   pkg/kubelet/pleg/generic.go::Relist (lines 253-285)
/// `Relist` records the wall-clock timestamp on every invocation —
/// `relistTime` (here `last_relist_time`) is what `Healthy` consults.
#[test]
fn upstream_relist_records_last_relist_time() {
    let mut pleg = GenericPleg::new(1024);
    assert!(pleg.last_relist_time().is_none());
    let t = t0();
    pleg.relist(t, vec![]);
    assert_eq!(pleg.last_relist_time(), Some(t));
    let t2 = t + ChronoDuration::seconds(5);
    pleg.relist(t2, vec![]);
    assert_eq!(pleg.last_relist_time(), Some(t2));
}

/// Upstream: kubernetes/kubernetes@v1.36.0
///   pkg/kubelet/pleg/pleg.go (lines 26-48)
/// `PodLifecycleEventType` is its own discriminant — values are
/// distinct constants (mirrors the Go const block).
#[test]
fn upstream_pleg_event_type_variants_are_distinct() {
    use PodLifecycleEventType::*;
    let variants = [
        ContainerStarted,
        ContainerDied,
        ContainerRemoved,
        ContainerChanged,
        NetworkSetupCompleted,
        ConditionMet,
        PodSync,
    ];
    for (i, a) in variants.iter().enumerate() {
        for (j, b) in variants.iter().enumerate() {
            if i == j {
                assert_eq!(a, b);
            } else {
                assert_ne!(a, b, "{:?} must differ from {:?}", a, b);
            }
        }
    }
}

/// Upstream: kubernetes/kubernetes@v1.36.0
///   pkg/kubelet/pleg/generic_test.go (relist outcome wiring)
/// `RelistOutcome.dropped` is zero on small/healthy channel and the
/// `events` vector is the canonical event sequence.
#[test]
fn upstream_relist_outcome_no_drop_in_unbounded_run() {
    let mut pleg = GenericPleg::new(usize::MAX);
    let out: RelistOutcome =
        pleg.relist(t0(), vec![pod("p", vec![c("c", ContainerState::Running)])]);
    assert_eq!(out.dropped, 0);
    assert_eq!(out.events.len(), 1);
}
