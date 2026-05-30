// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD port of PostgreSQL's nbtree access method
// (src/backend/access/nbtree/nbtinsert.c, nbtsearch.c).
//
// Faithful behaviours asserted:
//   * balanced multiway search tree (node splits keep height balanced)
//   * duplicate keys retain insertion order in their posting list
//   * equality probe returns the heap TIDs for a key
//   * ordered forward range scan with inclusive/open bounds (leaf walk)
//   * the tree stays sorted across many randomized inserts

use cave_rdbms::storage::index::BTreeIndex;
use cave_rdbms::types::SqlValue;

fn i4(n: i32) -> SqlValue {
    SqlValue::Int4(n)
}

#[test]
fn btree_equality_probe_returns_tids() {
    let mut idx = BTreeIndex::new();
    idx.insert(i4(5), 0);
    idx.insert(i4(2), 1);
    idx.insert(i4(8), 2);
    assert_eq!(idx.search(&i4(5)), vec![0]);
    assert_eq!(idx.search(&i4(2)), vec![1]);
    assert_eq!(idx.search(&i4(8)), vec![2]);
    assert!(idx.search(&i4(99)).is_empty());
}

#[test]
fn btree_duplicate_keys_preserve_insertion_order() {
    let mut idx = BTreeIndex::new();
    idx.insert(i4(5), 0);
    idx.insert(i4(5), 3);
    idx.insert(i4(5), 1);
    // posting list ordered by insertion (nbtree appends to the right)
    assert_eq!(idx.search(&i4(5)), vec![0, 3, 1]);
    assert_eq!(idx.len(), 3);
}

#[test]
fn btree_range_scan_is_ordered_and_inclusive() {
    let mut idx = BTreeIndex::new();
    for (k, t) in [(8, 0), (2, 1), (5, 2), (1, 3), (9, 4), (5, 5)] {
        idx.insert(i4(k), t);
    }
    // [2, 5] inclusive → (2,1),(5,2),(5,5)
    let got = idx.range_scan(Some(&i4(2)), Some(&i4(5)));
    let keys: Vec<i32> = got.iter().map(|(k, _)| k.as_i32().unwrap()).collect();
    let tids: Vec<usize> = got.iter().map(|(_, t)| *t).collect();
    assert_eq!(keys, vec![2, 5, 5]);
    assert_eq!(tids, vec![1, 2, 5]);
}

#[test]
fn btree_range_scan_open_bounds_walk_all_ordered() {
    let mut idx = BTreeIndex::new();
    for k in [50, 10, 30, 20, 40, 5, 60, 15, 25, 35, 45, 55] {
        idx.insert(i4(k), k as usize);
    }
    // unbounded → full ascending leaf walk
    let all = idx.range_scan(None, None);
    let keys: Vec<i32> = all.iter().map(|(k, _)| k.as_i32().unwrap()).collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted, "leaf walk must be globally sorted");
    assert_eq!(keys.len(), 12);

    // lower-bound-only scan: keys >= 40
    let hi = idx.range_scan(Some(&i4(40)), None);
    let hk: Vec<i32> = hi.iter().map(|(k, _)| k.as_i32().unwrap()).collect();
    assert_eq!(hk, vec![40, 45, 50, 55, 60]);
}

#[test]
fn btree_stays_balanced_and_sorted_under_many_inserts() {
    let mut idx = BTreeIndex::new();
    // deterministic pseudo-shuffle to force many node splits
    let mut x: i64 = 1;
    for _ in 0..500 {
        x = (x * 1103515245 + 12345) & 0x7fff_ffff;
        let k = (x % 1000) as i32;
        idx.insert(i4(k), 0);
    }
    assert_eq!(idx.len(), 500);
    let all = idx.range_scan(None, None);
    let keys: Vec<i32> = all.iter().map(|(k, _)| k.as_i32().unwrap()).collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted, "500 randomized inserts must remain sorted");
    // a balanced B-tree of 500 entries with min-degree>=2 must stay shallow
    assert!(idx.height() <= 12, "tree too deep: {}", idx.height());
}
