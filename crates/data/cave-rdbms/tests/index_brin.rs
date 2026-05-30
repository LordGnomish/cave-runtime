// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD port of PostgreSQL's BRIN access method
// (src/backend/access/brin/{brin.c,brin_minmax.c}).
//
// BRIN summarises each *page range* with the min/max of its keys. Scans are
// lossy: a range whose [min,max] overlaps the query is returned in full as a
// candidate set (the executor rechecks tuples); a range that cannot overlap is
// pruned. Faithful behaviours asserted:
//   * one minmax summary tuple per page range (brin_minmax.c add_value)
//   * a point/range query prunes non-overlapping ranges entirely
//   * the returned candidate set is a (lossy) superset of the true matches
//   * summaries track the true min/max even with non-monotonic keys

use cave_rdbms::storage::index::BrinIndex;
use cave_rdbms::types::SqlValue;

fn i4(n: i32) -> SqlValue {
    SqlValue::Int4(n)
}

#[test]
fn brin_summarises_one_tuple_per_page_range() {
    let mut brin = BrinIndex::new(5);
    for tid in 0..20usize {
        brin.insert(tid, i4(tid as i32));
    }
    // 20 tids / range_size 5 => 4 summary ranges
    let s = brin.summary();
    assert_eq!(s.len(), 4);
    assert_eq!((s[0].min.as_i32(), s[0].max.as_i32()), (Some(0), Some(4)));
    assert_eq!((s[3].min.as_i32(), s[3].max.as_i32()), (Some(15), Some(19)));
}

#[test]
fn brin_prunes_non_overlapping_ranges() {
    let mut brin = BrinIndex::new(5);
    for tid in 0..20usize {
        brin.insert(tid, i4(tid as i32));
    }
    // query [7,8] overlaps only the [5..=9] range → candidates 5..=9
    let cand = brin.search(Some(&i4(7)), Some(&i4(8)));
    assert_eq!(cand, vec![5, 6, 7, 8, 9]);
    // every true match is contained (lossy superset)
    assert!(cand.contains(&7) && cand.contains(&8));
}

#[test]
fn brin_minmax_tracks_unordered_keys() {
    let mut brin = BrinIndex::new(4);
    // first range holds 50,10,30,20 → min 10 max 50
    for (tid, k) in [50, 10, 30, 20, 5, 9, 7, 6].iter().enumerate() {
        brin.insert(tid, i4(*k));
    }
    let s = brin.summary();
    assert_eq!((s[0].min.as_i32(), s[0].max.as_i32()), (Some(10), Some(50)));
    assert_eq!((s[1].min.as_i32(), s[1].max.as_i32()), (Some(5), Some(9)));

    // query for key 25: only range 0 ([10,50]) can contain it
    let cand = brin.search(Some(&i4(25)), Some(&i4(25)));
    assert_eq!(cand, vec![0, 1, 2, 3]);
}

#[test]
fn brin_open_bounds_scan_all_non_pruned() {
    let mut brin = BrinIndex::new(5);
    for tid in 0..20usize {
        brin.insert(tid, i4(tid as i32));
    }
    // keys >= 13 → ranges [10..=14] and [15..=19] survive
    let cand = brin.search(Some(&i4(13)), None);
    assert_eq!(cand, (10..20).collect::<Vec<usize>>());
    assert_eq!(brin.range_count(), 4);
}
