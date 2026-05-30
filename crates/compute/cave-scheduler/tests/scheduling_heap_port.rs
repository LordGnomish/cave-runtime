// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Port-fidelity tests for the key-indexed scheduling heap.
//!
//! Cite: kubernetes/kubernetes v1.36.0
//!   pkg/scheduler/internal/heap/heap.go
//!
//! Upstream's scheduler heap is NOT a plain min-heap: it carries an
//! `items map[string]*heapItem` so the active queue can `Update` a pod's
//! position or `Delete` it by key in O(log n), and `AddIfNotPresent`
//! dedups re-adds. Go's `container/heap` cannot do arbitrary-key removal
//! and neither can Rust's `std::collections::BinaryHeap`; that is the
//! behavioral gap this module closes.

use cave_scheduler::scheduling_heap::Heap;

/// A test object: a named item with an integer priority.
#[derive(Debug, Clone, PartialEq)]
struct Item {
    key: String,
    prio: i32,
}

fn it(key: &str, prio: i32) -> Item {
    Item {
        key: key.to_string(),
        prio,
    }
}

/// Build a max-by-prio heap (higher prio pops first), keyed by name.
fn max_heap() -> Heap<Item> {
    Heap::new(
        |i: &Item| i.key.clone(),
        // less(a, b) == "a comes out before b" → higher prio first.
        |a: &Item, b: &Item| a.prio > b.prio,
    )
}

#[test]
fn pop_respects_less_ordering() {
    let mut h = max_heap();
    h.add(it("low", 1));
    h.add(it("high", 100));
    h.add(it("mid", 50));
    assert_eq!(h.len(), 3);
    assert_eq!(h.pop().unwrap().key, "high");
    assert_eq!(h.pop().unwrap().key, "mid");
    assert_eq!(h.pop().unwrap().key, "low");
    assert!(h.pop().is_none());
    assert!(h.is_empty());
}

#[test]
fn add_is_idempotent_on_key_and_updates_in_place() {
    let mut h = max_heap();
    h.add(it("a", 5));
    h.add(it("a", 5)); // same key → replace, not duplicate
    assert_eq!(h.len(), 1);
    // Re-add with higher prio updates the stored object and reorders.
    h.add(it("b", 1));
    h.add(it("a", 0)); // now a is lowest
    assert_eq!(h.len(), 2);
    assert_eq!(h.pop().unwrap().key, "b");
    assert_eq!(h.pop().unwrap().key, "a");
}

#[test]
fn add_if_not_present_never_overwrites() {
    let mut h = max_heap();
    h.add(it("a", 5));
    h.add_if_not_present(it("a", 999)); // ignored, a stays prio 5
    h.add_if_not_present(it("b", 1));
    assert_eq!(h.len(), 2);
    assert_eq!(h.get_by_key("a").unwrap().prio, 5);
    assert_eq!(h.get_by_key("b").unwrap().prio, 1);
}

#[test]
fn update_reorders_existing_item() {
    let mut h = max_heap();
    h.add(it("a", 10));
    h.add(it("b", 20));
    h.add(it("c", 30));
    // Promote a to the top.
    h.update(it("a", 100));
    assert_eq!(h.peek().unwrap().key, "a");
    assert_eq!(h.pop().unwrap().key, "a");
    // Demote c below b.
    h.update(it("c", 1));
    assert_eq!(h.pop().unwrap().key, "b");
    assert_eq!(h.pop().unwrap().key, "c");
}

#[test]
fn delete_by_key_removes_arbitrary_interior_node() {
    let mut h = max_heap();
    for (k, p) in [("a", 10), ("b", 20), ("c", 30), ("d", 40), ("e", 50)] {
        h.add(it(k, p));
    }
    // Remove an interior (non-root) element by key and confirm heap stays valid.
    assert!(h.delete_by_key("c"));
    assert!(!h.delete_by_key("c")); // already gone
    assert_eq!(h.len(), 4);
    assert!(h.get_by_key("c").is_none());
    // Remaining pop order is still strictly by prio.
    assert_eq!(h.pop().unwrap().key, "e");
    assert_eq!(h.pop().unwrap().key, "d");
    assert_eq!(h.pop().unwrap().key, "b");
    assert_eq!(h.pop().unwrap().key, "a");
}

#[test]
fn peek_does_not_consume() {
    let mut h = max_heap();
    h.add(it("x", 7));
    assert_eq!(h.peek().unwrap().key, "x");
    assert_eq!(h.peek().unwrap().key, "x");
    assert_eq!(h.len(), 1);
    assert!(max_heap().peek().is_none());
}

#[test]
fn get_and_list_expose_contents() {
    let mut h = max_heap();
    h.add(it("a", 1));
    h.add(it("b", 2));
    assert_eq!(h.get(&it("a", 1)).unwrap().prio, 1);
    assert!(h.get(&it("z", 0)).is_none());
    let mut keys: Vec<String> = h.list().into_iter().map(|i| i.key.clone()).collect();
    keys.sort();
    assert_eq!(keys, vec!["a".to_string(), "b".to_string()]);
}

#[test]
fn metric_recorder_counts_live_adds() {
    let mut h = max_heap();
    h.add(it("a", 1));
    h.add(it("b", 2));
    h.add(it("a", 9)); // update — not a new add
    assert_eq!(h.adds(), 2);
    h.pop();
    assert_eq!(h.adds(), 2); // pop does not change the add counter
}

#[test]
fn delete_obj_errors_when_absent() {
    let mut h = max_heap();
    h.add(it("a", 1));
    assert!(h.delete(&it("a", 1)).is_ok());
    assert!(h.delete(&it("a", 1)).is_err());
}

/// Stress: random-ish insert/update/delete leaves a valid heap whose
/// drain order is monotonic by prio.
#[test]
fn heap_property_holds_under_churn() {
    let mut h = max_heap();
    for i in 0..50 {
        h.add(it(&format!("k{i}"), (i * 7) % 23));
    }
    // Update and delete a scattering of keys.
    for i in (0..50).step_by(3) {
        h.update(it(&format!("k{i}"), 100 - i));
    }
    for i in (0..50).step_by(5) {
        h.delete_by_key(&format!("k{i}"));
    }
    let mut last = i32::MAX;
    let mut count = 0;
    while let Some(item) = h.pop() {
        assert!(item.prio <= last, "pop order violated heap property");
        last = item.prio;
        count += 1;
    }
    assert_eq!(count, h_expected_len());
}

fn h_expected_len() -> usize {
    // 50 inserted, minus those with i % 5 == 0 deleted (0,5,...,45 = 10).
    50 - 10
}
