// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Edge coverage for cave-kubelet — agent CRUD, node lease, node conditions,
//! models.

use cave_kubelet::agent::{assign_pod, node_status, remove_pod, start_pod, stop_pod, KubeletState};
use cave_kubelet::models::{ContainerState, NodeStatusReport, PodPhase};
use cave_kubelet::node_lease::{
    ConditionStatus, LeaseError, NodeConditionTracker, NodeLease,
};
use chrono::{Duration, Utc};
use uuid::Uuid;

#[test]
fn assign_pod_creates_with_pending_phase() {
    let s = KubeletState::new("n");
    let pod = assign_pod(&s, "p", "ns", vec![("c".into(), "img".into())]);
    assert_eq!(pod.status, PodPhase::Pending);
    assert!(pod.started_at.is_none());
    assert_eq!(pod.containers.len(), 1);
    assert!(matches!(pod.containers[0].state, ContainerState::Waiting { .. }));
}

#[test]
fn assign_pod_assigns_distinct_uids() {
    let s = KubeletState::new("n");
    let a = assign_pod(&s, "a", "ns", vec![]);
    let b = assign_pod(&s, "b", "ns", vec![]);
    assert_ne!(a.uid, b.uid);
}

#[test]
fn start_pod_marks_running_and_ready() {
    let s = KubeletState::new("n");
    let p = assign_pod(&s, "p", "ns", vec![("c".into(), "img".into())]);
    let started = start_pod(&s, &p.uid).unwrap();
    assert_eq!(started.status, PodPhase::Running);
    assert!(started.started_at.is_some());
    assert!(started.containers[0].ready);
    assert!(started.containers[0].container_id.is_some());
}

#[test]
fn start_pod_unknown_uid_returns_none() {
    let s = KubeletState::new("n");
    assert!(start_pod(&s, &Uuid::new_v4()).is_none());
}

#[test]
fn stop_pod_marks_succeeded_and_terminated() {
    let s = KubeletState::new("n");
    let p = assign_pod(&s, "p", "ns", vec![("c".into(), "img".into())]);
    start_pod(&s, &p.uid);
    let stopped = stop_pod(&s, &p.uid).unwrap();
    assert_eq!(stopped.status, PodPhase::Succeeded);
    assert!(matches!(
        stopped.containers[0].state,
        ContainerState::Terminated { exit_code: 0, .. }
    ));
    assert!(!stopped.containers[0].ready);
}

#[test]
fn remove_pod_returns_removed_or_none() {
    let s = KubeletState::new("n");
    let p = assign_pod(&s, "p", "ns", vec![]);
    let removed = remove_pod(&s, &p.uid).unwrap();
    assert_eq!(removed.name, "p");
    assert!(remove_pod(&s, &p.uid).is_none(), "double-remove yields None");
}

#[test]
fn node_status_reports_pod_count() {
    let s = KubeletState::new("worker-7");
    for i in 0..4 {
        assign_pod(&s, &format!("p{}", i), "ns", vec![]);
    }
    let r = node_status(&s);
    assert_eq!(r.pod_count, 4);
    assert_eq!(r.node_name, "worker-7");
    assert!(r.ready);
}

#[test]
fn node_status_healthy_template_has_four_conditions() {
    let s = NodeStatusReport::healthy("node-x");
    assert_eq!(s.conditions.len(), 4);
    let types: Vec<&String> = s.conditions.iter().map(|c| &c.condition_type).collect();
    for needed in ["Ready", "MemoryPressure", "DiskPressure", "PIDPressure"] {
        assert!(types.iter().any(|t| *t == needed), "missing {}", needed);
    }
}

#[test]
fn node_status_healthy_template_defaults_capacity() {
    let s = NodeStatusReport::healthy("n");
    assert_eq!(s.pod_count, 0);
    assert_eq!(s.pod_capacity, 110);
    assert_eq!(s.cpu_used_millicores, 0);
}

#[test]
fn pod_phase_serializes_as_string_variants() {
    assert_eq!(serde_json::to_string(&PodPhase::Pending).unwrap(), "\"Pending\"");
    assert_eq!(serde_json::to_string(&PodPhase::Running).unwrap(), "\"Running\"");
    assert_eq!(serde_json::to_string(&PodPhase::Succeeded).unwrap(), "\"Succeeded\"");
    assert_eq!(serde_json::to_string(&PodPhase::Failed).unwrap(), "\"Failed\"");
    assert_eq!(serde_json::to_string(&PodPhase::Unknown).unwrap(), "\"Unknown\"");
}

#[test]
fn container_state_variants_distinguish_via_equality() {
    let now = Utc::now();
    let w = ContainerState::Waiting { reason: "x".into() };
    let r = ContainerState::Running { started_at: now };
    let t_a = ContainerState::Terminated { exit_code: 1, reason: "OOM".into(), finished_at: now };
    let t_b = ContainerState::Terminated { exit_code: 137, reason: "OOM".into(), finished_at: now };
    assert_ne!(w, r);
    assert_ne!(r, t_a);
    assert_ne!(t_a, t_b, "different exit_code → not equal");
    assert_eq!(t_a, t_a.clone());
}

// ---------------------------------------------------------------------------
// NodeLease
// ---------------------------------------------------------------------------

#[test]
fn lease_new_records_acquire_and_renew_at_same_time() {
    let now = Utc::now();
    let l = NodeLease::new("node-a", "acme", 30, now);
    assert_eq!(l.acquire_time, l.renew_time);
    assert_eq!(l.expires_at(), now + Duration::seconds(30));
}

#[test]
fn lease_renew_interval_is_duration_div_4_min_1() {
    let l = NodeLease::new("n", "t", 40, Utc::now());
    assert_eq!(l.renew_interval(), Duration::seconds(10));
    let tiny = NodeLease::new("n", "t", 0, Utc::now());
    assert_eq!(tiny.renew_interval(), Duration::seconds(1), "min 1s guard");
}

#[test]
fn lease_renew_succeeds_within_validity_window() {
    let now = Utc::now();
    let mut l = NodeLease::new("n", "t", 60, now);
    l.renew("n", now + Duration::seconds(5)).unwrap();
    assert_eq!(l.renew_time, now + Duration::seconds(5));
    assert_eq!(l.acquire_time, now, "acquire_time stays put");
}

#[test]
fn lease_renew_wrong_holder_errors() {
    let now = Utc::now();
    let mut l = NodeLease::new("a", "t", 30, now);
    let err = l.renew("b", now).unwrap_err();
    assert!(matches!(err, LeaseError::HolderMismatch { .. }));
}

#[test]
fn lease_renew_after_expiry_errors() {
    let now = Utc::now();
    let mut l = NodeLease::new("a", "t", 10, now);
    let later = now + Duration::seconds(20);
    let err = l.renew("a", later).unwrap_err();
    assert_eq!(err, LeaseError::Expired);
}

#[test]
fn lease_is_valid_at_exact_expiry_boundary() {
    let now = Utc::now();
    let l = NodeLease::new("a", "t", 10, now);
    assert!(l.is_valid(now + Duration::seconds(10)), "boundary inclusive");
    assert!(!l.is_valid(now + Duration::seconds(11)));
}

// ---------------------------------------------------------------------------
// NodeConditionTracker
// ---------------------------------------------------------------------------

#[test]
fn condition_set_new_returns_true_first_time() {
    let mut t = NodeConditionTracker::default();
    let now = Utc::now();
    let transitioned = t.set("Ready", ConditionStatus::True, "Ok", "all good", now);
    assert!(transitioned);
    assert!(t.ready());
}

#[test]
fn condition_set_same_status_returns_false() {
    let mut t = NodeConditionTracker::default();
    let now = Utc::now();
    t.set("Ready", ConditionStatus::True, "Ok", "", now);
    let again = t.set("Ready", ConditionStatus::True, "still", "", now + Duration::seconds(1));
    assert!(!again, "no transition when status unchanged");
}

#[test]
fn condition_set_changed_status_bumps_transition_time() {
    let mut t = NodeConditionTracker::default();
    let t0 = Utc::now();
    t.set("Ready", ConditionStatus::True, "Ok", "", t0);
    let trans_before = t.conditions["Ready"].last_transition_time;
    let t1 = t0 + Duration::seconds(60);
    let transitioned = t.set("Ready", ConditionStatus::False, "Down", "", t1);
    assert!(transitioned);
    assert!(t.conditions["Ready"].last_transition_time > trans_before);
    assert!(!t.ready());
}

#[test]
fn condition_lost_heartbeat_lists_stale_conditions() {
    let mut t = NodeConditionTracker::default();
    let t0 = Utc::now();
    t.set("Ready", ConditionStatus::True, "", "", t0);
    t.set("DiskPressure", ConditionStatus::False, "", "", t0 + Duration::seconds(60));
    let threshold = t0 + Duration::seconds(30);
    let stale = t.lost_heartbeat_since(threshold);
    assert_eq!(stale, vec!["Ready".to_string()]);
}

#[test]
fn condition_unknown_status_means_not_ready() {
    let mut t = NodeConditionTracker::default();
    t.set("Ready", ConditionStatus::Unknown, "", "", Utc::now());
    assert!(!t.ready());
}
