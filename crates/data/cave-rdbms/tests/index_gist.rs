// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD port of PostgreSQL's GiST access method
// (src/backend/access/gist/{gist.c,gistproc.c,gistsplit.c}) instantiated for
// the 1-D interval opclass (the shape `range_ops` / `box_ops` use).
//
// A GiST is a balanced tree where every entry carries a *predicate* (here the
// bounding interval [lo,hi]). The opclass support functions drive it:
//   * consistent — does an entry's bbox overlap the query? (subtree pruning)
//   * union      — bounding interval covering a set of children
//   * penalty    — bbox enlargement used to choose the insertion subtree
//   * picksplit  — partition an overflowing node into two
//
// Faithful behaviours asserted:
//   * an overlap query returns exactly the intervals that overlap (leaf-exact)
//   * non-overlapping subtrees are pruned (query far outside → empty)
//   * the tree stays balanced (shallow) and lossless under many inserts
//   * point containment is a degenerate overlap query

use cave_rdbms::storage::index::GistIndex;

#[test]
fn gist_overlap_query_is_leaf_exact() {
    let mut g = GistIndex::new();
    g.insert(0, 1, 5);
    g.insert(1, 4, 9);
    g.insert(2, 10, 12);
    g.insert(3, 6, 7);
    // overlap [4,6] → intervals [1,5],[4,9],[6,7] → tids 0,1,3
    let mut got = g.search_overlap(4, 6);
    got.sort();
    assert_eq!(got, vec![0, 1, 3]);
}

#[test]
fn gist_prunes_disjoint_query() {
    let mut g = GistIndex::new();
    g.insert(0, 1, 5);
    g.insert(1, 10, 20);
    assert!(g.search_overlap(100, 200).is_empty());
    assert!(g.search_overlap(6, 9).is_empty());
}

#[test]
fn gist_point_containment() {
    let mut g = GistIndex::new();
    g.insert(0, 0, 10);
    g.insert(1, 5, 15);
    g.insert(2, 20, 30);
    let mut got = g.search_contains_point(7);
    got.sort();
    assert_eq!(got, vec![0, 1]);
    assert_eq!(g.search_contains_point(25), vec![2]);
}

#[test]
fn gist_stays_balanced_and_lossless_under_many_inserts() {
    let mut g = GistIndex::new();
    let mut x: i64 = 7;
    let mut truth: Vec<(usize, i64, i64)> = Vec::new();
    for tid in 0..300usize {
        x = (x * 1103515245 + 12345) & 0x7fff_ffff;
        let lo = (x % 1000) as i64;
        let hi = lo + (x % 50) as i64;
        g.insert(tid, lo, hi);
        truth.push((tid, lo, hi));
    }
    // exhaustive ground truth for a fixed query window
    let (qlo, qhi) = (400, 450);
    let mut expect: Vec<usize> = truth
        .iter()
        .filter(|(_, lo, hi)| *lo <= qhi && *hi >= qlo)
        .map(|(t, _, _)| *t)
        .collect();
    expect.sort();
    let mut got = g.search_overlap(qlo, qhi);
    got.sort();
    assert_eq!(got, expect, "GiST overlap must be leaf-exact");
    // a balanced tree of 300 entries with fanout>=4 stays shallow
    assert!(g.height() <= 8, "tree too deep: {}", g.height());
}
