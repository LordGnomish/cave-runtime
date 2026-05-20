// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Edge coverage for cave-scheduler — models, Status helpers, priority queue,
//! scheduling gates.

use cave_scheduler::framework::{Code, Pod, Status};
use cave_scheduler::gates::SchedulingGates;
use cave_scheduler::extension_points::PreEnqueuePlugin;
use cave_scheduler::models::{
    NodeStatus, ResourceCapacity, ResourceRequest, TaintEffect,
};
use cave_scheduler::priority_queue::{PreemptionPolicy, PriorityQueue};
use chrono::{Duration, Utc};

fn pod(name: &str, priority: i32) -> Pod {
    let mut p = Pod::new("t", "ns", name);
    p.spec.priority = priority;
    p
}

// ---------------------------------------------------------------------------
// ResourceCapacity / ResourceRequest
// ---------------------------------------------------------------------------

#[test]
fn capacity_has_room_only_when_both_cpu_and_memory_fit() {
    let cap = ResourceCapacity {
        cpu_millicores: 1000,
        memory_bytes: 2000,
        pods: 10,
        ephemeral_storage_bytes: 0,
    };
    assert!(cap.has_room_for(&ResourceRequest { cpu_millicores: 500, memory_bytes: 1000, ..Default::default() }));
    // CPU exceeds capacity
    assert!(!cap.has_room_for(&ResourceRequest { cpu_millicores: 5000, memory_bytes: 100, ..Default::default() }));
    // Memory exceeds capacity
    assert!(!cap.has_room_for(&ResourceRequest { cpu_millicores: 100, memory_bytes: 99_999, ..Default::default() }));
}

#[test]
fn capacity_subtract_saturates_to_zero_no_underflow() {
    let mut cap = ResourceCapacity {
        cpu_millicores: 100,
        memory_bytes: 100,
        pods: 0,
        ephemeral_storage_bytes: 0,
    };
    let big = ResourceRequest {
        cpu_millicores: 1000,
        memory_bytes: 1000,
        ..Default::default()
    };
    cap.subtract(&big);
    assert_eq!(cap.cpu_millicores, 0, "saturates, no underflow");
    assert_eq!(cap.memory_bytes, 0);
    assert_eq!(cap.pods, 0, "pods saturates to 0");
}

#[test]
fn capacity_add_increments_pod_count_by_one() {
    let mut cap = ResourceCapacity::default();
    cap.add(&ResourceRequest { cpu_millicores: 1, memory_bytes: 1, ..Default::default() });
    cap.add(&ResourceRequest { cpu_millicores: 1, memory_bytes: 1, ..Default::default() });
    assert_eq!(cap.pods, 2);
}

#[test]
fn node_status_and_taint_effect_serde_distinct() {
    assert_eq!(serde_json::to_string(&NodeStatus::Ready).unwrap(), "\"Ready\"");
    assert_eq!(serde_json::to_string(&NodeStatus::Cordoned).unwrap(), "\"Cordoned\"");
    assert_eq!(serde_json::to_string(&TaintEffect::NoSchedule).unwrap(), "\"NoSchedule\"");
    assert_eq!(serde_json::to_string(&TaintEffect::NoExecute).unwrap(), "\"NoExecute\"");
}

// ---------------------------------------------------------------------------
// Status state machine
// ---------------------------------------------------------------------------

#[test]
fn status_success_is_success_not_rejected() {
    let s = Status::success("X");
    assert!(s.is_success());
    assert!(!s.is_rejected());
    assert!(!s.is_skip());
    assert!(!s.is_wait());
    assert!(!s.is_pending());
    assert!(!s.is_error());
    assert!(s.failed_plugin.is_none());
    assert!(s.reasons.is_empty());
}

#[test]
fn status_unschedulable_is_rejected_with_failed_plugin() {
    let s = Status::unschedulable("Filter", "no room");
    assert!(s.is_rejected());
    assert!(!s.is_success());
    assert_eq!(s.failed_plugin.as_deref(), Some("Filter"));
    assert_eq!(s.reasons, vec!["no room".to_string()]);
}

#[test]
fn status_unresolvable_is_rejected() {
    let s = Status::unresolvable("X", "permanent");
    assert!(s.is_rejected());
    assert_eq!(s.code, Code::UnschedulableAndUnresolvable);
}

#[test]
fn status_wait_carries_duration() {
    let s = Status::wait("Permit", "user-ack", Duration::seconds(30));
    assert!(s.is_wait());
    assert!(!s.is_success());
    assert_eq!(s.wait_duration, Some(Duration::seconds(30)));
}

#[test]
fn status_pending_is_neither_success_nor_rejected() {
    let s = Status::pending("Gates", "waiting on gates");
    assert!(s.is_pending());
    assert!(!s.is_success());
    assert!(!s.is_rejected());
}

#[test]
fn status_error_distinct() {
    let s = Status::error("X", "internal");
    assert!(s.is_error());
    assert!(!s.is_success());
    assert!(!s.is_rejected());
}

#[test]
fn status_skip_distinct() {
    let s = Status::skip("PreFilter");
    assert!(s.is_skip());
    assert!(!s.is_success());
}

// ---------------------------------------------------------------------------
// Pod::new constructs uid from tenant/ns/name
// ---------------------------------------------------------------------------

#[test]
fn pod_new_uid_includes_tenant_namespace_name() {
    let p = Pod::new("acme", "default", "nginx");
    assert_eq!(p.uid, "acme-default-nginx");
    assert_eq!(p.tenant_id, "acme");
    assert_eq!(p.namespace, "default");
    assert_eq!(p.name, "nginx");
    assert_eq!(p.spec.priority, 0);
}

// ---------------------------------------------------------------------------
// SchedulingGates PreEnqueue
// ---------------------------------------------------------------------------

#[test]
fn gates_plugin_succeeds_when_no_gates() {
    let p = Pod::new("t", "ns", "p");
    let s = SchedulingGates.pre_enqueue(&p);
    assert!(s.is_success());
}

#[test]
fn gates_plugin_pending_when_any_gate_present() {
    let mut p = Pod::new("t", "ns", "p");
    p.spec.scheduling_gates.push("controller/quota".into());
    let s = SchedulingGates.pre_enqueue(&p);
    assert!(s.is_pending());
    assert!(s.reasons[0].contains("controller/quota"));
}

#[test]
fn gates_plugin_name() {
    assert_eq!(SchedulingGates.name(), "SchedulingGates");
}

// ---------------------------------------------------------------------------
// PriorityQueue state machine
// ---------------------------------------------------------------------------

#[test]
fn pq_empty_pops_none() {
    let mut q = PriorityQueue::new();
    assert!(q.is_empty());
    assert!(q.pop().is_none());
}

#[test]
fn pq_pop_in_priority_order() {
    let mut q = PriorityQueue::new();
    q.add(pod("low", 1));
    q.add(pod("high", 100));
    q.add(pod("mid", 50));
    assert_eq!(q.pop().unwrap().name, "high");
    assert_eq!(q.pop().unwrap().name, "mid");
    assert_eq!(q.pop().unwrap().name, "low");
}

#[test]
fn pq_len_counts_active_backoff_unschedulable() {
    let mut q = PriorityQueue::new();
    q.add(pod("a", 1));
    let now = Utc::now();
    q.mark_backoff(pod("b", 1), now);
    q.mark_unschedulable(pod("c", 1), now);
    assert_eq!(q.active_len(), 1);
    assert_eq!(q.backoff_len(), 1);
    assert_eq!(q.unschedulable_len(), 1);
    assert_eq!(q.len(), 3);
}

#[test]
fn pq_mark_backoff_grows_attempts_exponentially() {
    let mut q = PriorityQueue::new()
        .with_backoff(Duration::seconds(1), Duration::seconds(100));
    let p = pod("p", 1);
    let now = Utc::now();
    let after1 = q.mark_backoff(p.clone(), now);
    // remove from backoff via flush so the next mark goes through unschedulable bookkeeping
    // (mark_backoff also removes from unschedulable).
    let after2 = q.mark_backoff(p.clone(), now);
    let after3 = q.mark_backoff(p, now);
    assert!(after2 > after1, "second attempt's backoff window must be longer");
    assert!(after3 > after2);
}

#[test]
fn pq_flush_backoff_moves_expired_back_to_active() {
    let mut q = PriorityQueue::new()
        .with_backoff(Duration::seconds(1), Duration::seconds(2));
    let now = Utc::now();
    q.mark_backoff(pod("p", 5), now);
    assert_eq!(q.backoff_len(), 1);
    q.flush_backoff(now + Duration::seconds(10));
    assert_eq!(q.backoff_len(), 0);
    assert_eq!(q.active_len(), 1);
}

#[test]
fn pq_move_all_unschedulable_back_to_active() {
    let mut q = PriorityQueue::new();
    let now = Utc::now();
    q.mark_unschedulable(pod("a", 1), now);
    q.mark_unschedulable(pod("b", 1), now);
    assert_eq!(q.unschedulable_len(), 2);
    q.move_all_unschedulable(now);
    assert_eq!(q.unschedulable_len(), 0);
    assert_eq!(q.active_len(), 2);
}

#[test]
fn pq_default_equiv_new() {
    let a = PriorityQueue::default();
    let b = PriorityQueue::new();
    assert_eq!(a.len(), b.len());
}

// ---------------------------------------------------------------------------
// PreemptionPolicy
// ---------------------------------------------------------------------------

#[test]
fn preemption_policy_variants_distinct() {
    assert_ne!(PreemptionPolicy::PreemptLowerPriority, PreemptionPolicy::Never);
}
