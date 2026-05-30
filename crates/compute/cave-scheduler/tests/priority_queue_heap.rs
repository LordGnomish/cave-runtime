// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! The active subqueue is backed by the key-indexed scheduling heap, so it
//! gains the upstream `SchedulingQueue` operations that a bare max-heap can't
//! express.
//!
//! Cite: kubernetes/kubernetes v1.36.0
//!   pkg/scheduler/internal/queue/scheduling_queue.go
//!   (Add / Update / Delete on the active `activeQ` heap)

use cave_scheduler::framework::Pod;
use cave_scheduler::priority_queue::PriorityQueue;

fn pod(name: &str, prio: i32) -> Pod {
    let mut p = Pod::new("t", "ns", name);
    p.spec.priority = prio;
    p
}

#[test]
fn add_dedups_by_uid_instead_of_duplicating() {
    let mut q = PriorityQueue::new();
    let p = pod("p", 5);
    q.add(p.clone());
    q.add(p.clone()); // re-add of the same pod must NOT create a second entry
    assert_eq!(q.active_len(), 1);
    assert!(q.pop().is_some());
    assert!(q.pop().is_none());
}

#[test]
fn update_changes_priority_in_place_and_reorders() {
    let mut q = PriorityQueue::new();
    let mut a = pod("a", 10);
    a.uid = "uid-a".into();
    let mut b = pod("b", 20);
    b.uid = "uid-b".into();
    q.add(a.clone());
    q.add(b.clone());
    // Promote a above b in place — same identity, higher priority.
    let mut a2 = a.clone();
    a2.spec.priority = 100;
    q.update(a2);
    assert_eq!(q.active_len(), 2);
    assert_eq!(q.pop().unwrap().name, "a");
    assert_eq!(q.pop().unwrap().name, "b");
}

#[test]
fn delete_removes_an_active_pod_by_uid() {
    let mut q = PriorityQueue::new();
    let mut a = pod("a", 10);
    a.uid = "uid-a".into();
    let mut b = pod("b", 20);
    b.uid = "uid-b".into();
    let mut c = pod("c", 30);
    c.uid = "uid-c".into();
    q.add(a);
    q.add(b.clone());
    q.add(c);
    assert!(q.delete(&b)); // remove the interior pod
    assert!(!q.delete(&b)); // already gone
    assert_eq!(q.active_len(), 2);
    assert_eq!(q.pop().unwrap().name, "c");
    assert_eq!(q.pop().unwrap().name, "a");
}
