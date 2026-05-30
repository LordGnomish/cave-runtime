// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// RED→GREEN cycle 11 (continuation ray #3): port of
// pkg/utils/pod/scheduling.go from kubernetes-sigs/karpenter v1.12.1 (sha
// ed490e8) — the pod-state predicates the provisioning + disruption
// controllers gate on. Pure / clock-threaded; the events.Recorder emission is
// dropped (non-behavioral — return values are independent of it).
//
// Reference instant: T0 = 1_767_225_600 (2026-01-01 00:00:00 UTC).

use cave_karpenter::pod::{
    is_active, is_disruptable, is_do_not_disrupt_active, is_drainable,
    is_pod_eligible_for_forced_eviction, is_provisionable, is_reschedulable, is_stuck_terminating,
    is_terminal, is_terminating, owner, Pod, PodCondition, PodPhase,
};
use cave_karpenter::scheduling::taints::{Effect, Operator, Toleration};

const T0: i64 = 1_767_225_600;

fn pod() -> Pod {
    Pod::default()
}

fn unschedulable_condition() -> PodCondition {
    PodCondition {
        type_: "PodScheduled".to_string(),
        reason: "Unschedulable".to_string(),
    }
}

// ── terminal / terminating ───────────────────────────────────────────────────

#[test]
fn is_terminal_for_failed_and_succeeded() {
    assert!(is_terminal(&Pod { phase: PodPhase::Failed, ..pod() }));
    assert!(is_terminal(&Pod { phase: PodPhase::Succeeded, ..pod() }));
    assert!(!is_terminal(&Pod { phase: PodPhase::Running, ..pod() }));
}

#[test]
fn is_terminating_when_deletion_timestamp_set() {
    assert!(is_terminating(&Pod { deletion_timestamp: Some(T0), ..pod() }));
    assert!(!is_terminating(&pod()));
}

#[test]
fn is_active_requires_not_terminal_and_not_terminating() {
    assert!(is_active(&Pod { phase: PodPhase::Running, ..pod() }));
    assert!(!is_active(&Pod { phase: PodPhase::Failed, ..pod() }));
    assert!(!is_active(&Pod { deletion_timestamp: Some(T0), phase: PodPhase::Running, ..pod() }));
}

// ── stuck terminating (clock) ────────────────────────────────────────────────

#[test]
fn is_stuck_terminating_after_one_minute() {
    let p = Pod { deletion_timestamp: Some(T0), ..pod() };
    assert!(!is_stuck_terminating(&p, T0 + 30)); // within the 1m buffer
    assert!(is_stuck_terminating(&p, T0 + 61)); // past 1m
    assert!(!is_stuck_terminating(&pod(), T0 + 1000)); // not terminating at all
}

// ── ownership ────────────────────────────────────────────────────────────────

#[test]
fn owner_predicates_match_gvk() {
    let ds = Pod {
        owner_references: vec![owner("apps/v1", "DaemonSet")],
        ..pod()
    };
    let ss = Pod {
        owner_references: vec![owner("apps/v1", "StatefulSet")],
        ..pod()
    };
    let node = Pod {
        owner_references: vec![owner("v1", "Node")],
        ..pod()
    };
    assert!(cave_karpenter::pod::is_owned_by_daemon_set(&ds));
    assert!(!cave_karpenter::pod::is_owned_by_daemon_set(&ss));
    assert!(cave_karpenter::pod::is_owned_by_stateful_set(&ss));
    assert!(cave_karpenter::pod::is_owned_by_node(&node));
}

// ── provisionable ────────────────────────────────────────────────────────────

#[test]
fn is_provisionable_when_unschedulable_unbound_unowned() {
    let p = Pod {
        conditions: vec![unschedulable_condition()],
        ..pod()
    };
    assert!(is_provisionable(&p));
}

#[test]
fn not_provisionable_when_already_scheduled() {
    let p = Pod {
        conditions: vec![unschedulable_condition()],
        node_name: "node-a".to_string(),
        ..pod()
    };
    assert!(!is_provisionable(&p));
}

#[test]
fn not_provisionable_when_preempting_or_daemonset() {
    let preempting = Pod {
        conditions: vec![unschedulable_condition()],
        nominated_node_name: "node-b".to_string(),
        ..pod()
    };
    assert!(!is_provisionable(&preempting));
    let ds = Pod {
        conditions: vec![unschedulable_condition()],
        owner_references: vec![owner("apps/v1", "DaemonSet")],
        ..pod()
    };
    assert!(!is_provisionable(&ds));
}

// ── reschedulable ────────────────────────────────────────────────────────────

#[test]
fn is_reschedulable_active_unowned_pod() {
    let p = Pod { phase: PodPhase::Running, ..pod() };
    assert!(is_reschedulable(&p));
}

#[test]
fn statefulset_terminating_pod_is_reschedulable() {
    // active==false (terminating) but owned by statefulset → still reschedulable
    let p = Pod {
        phase: PodPhase::Running,
        deletion_timestamp: Some(T0),
        owner_references: vec![owner("apps/v1", "StatefulSet")],
        ..pod()
    };
    assert!(is_reschedulable(&p));
}

#[test]
fn daemonset_pod_is_not_reschedulable() {
    let p = Pod {
        phase: PodPhase::Running,
        owner_references: vec![owner("apps/v1", "DaemonSet")],
        ..pod()
    };
    assert!(!is_reschedulable(&p));
}

// ── forced eviction ──────────────────────────────────────────────────────────

#[test]
fn forced_eviction_when_terminating_past_node_grace() {
    let p = Pod { deletion_timestamp: Some(T0 + 100), ..pod() };
    assert!(is_pod_eligible_for_forced_eviction(&p, Some(T0 + 50)));
    assert!(!is_pod_eligible_for_forced_eviction(&p, Some(T0 + 200)));
    assert!(!is_pod_eligible_for_forced_eviction(&p, None));
}

// ── do-not-disrupt (clock) ───────────────────────────────────────────────────

#[test]
fn do_not_disrupt_true_is_active() {
    let mut p = pod();
    p.annotations
        .insert("karpenter.sh/do-not-disrupt".to_string(), "true".to_string());
    assert!(is_do_not_disrupt_active(&p, T0));
}

#[test]
fn do_not_disrupt_duration_window() {
    let mut p = Pod { start_time: Some(T0), ..pod() };
    p.annotations
        .insert("karpenter.sh/do-not-disrupt".to_string(), "1h".to_string());
    // within the hour → active; past the hour → inactive
    assert!(is_do_not_disrupt_active(&p, T0 + 1800));
    assert!(!is_do_not_disrupt_active(&p, T0 + 3700));
}

#[test]
fn do_not_disrupt_invalid_is_inactive() {
    let mut p = pod();
    p.annotations
        .insert("karpenter.sh/do-not-disrupt".to_string(), "notaduration".to_string());
    assert!(!is_do_not_disrupt_active(&p, T0));
}

#[test]
fn do_not_disrupt_absent_is_inactive() {
    assert!(!is_do_not_disrupt_active(&pod(), T0));
}

// ── disruptable / drainable ──────────────────────────────────────────────────

#[test]
fn is_disruptable_when_not_protected() {
    let p = Pod { phase: PodPhase::Running, ..pod() };
    assert!(is_disruptable(&p, T0));
    let mut protected = Pod { phase: PodPhase::Running, ..pod() };
    protected
        .annotations
        .insert("karpenter.sh/do-not-disrupt".to_string(), "true".to_string());
    assert!(!is_disruptable(&protected, T0));
}

#[test]
fn drainable_unless_tolerates_disrupted_taint_or_stuck_or_node_owned() {
    let p = Pod { phase: PodPhase::Running, ..pod() };
    assert!(is_drainable(&p, T0));

    // tolerates karpenter.sh/disruption:NoSchedule → not drainable
    let tolerating = Pod {
        tolerations: vec![Toleration {
            key: Some("karpenter.sh/disruption".to_string()),
            operator: Operator::Exists,
            value: None,
            effect: Some(Effect::NoSchedule),
        }],
        ..pod()
    };
    assert!(!is_drainable(&tolerating, T0));
}
