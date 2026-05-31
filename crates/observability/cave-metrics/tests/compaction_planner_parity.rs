// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Parity tests for the TSDB leveled-compaction planner.
//!
//! Upstream: prometheus/prometheus `tsdb/compact.go` v3.12.0
//!   * `exponential_block_ranges(minSize, steps, stepSize)`
//!   * `LeveledCompactor.plan(dms)` — priority: overlapping → range-aligned → tombstones
//!   * `selectDirs` / `selectOverlappingDirs` / `splitByRange`
//!
//! The planner decides *which* on-disk blocks to merge next. It never touches
//! sample data — it operates purely on block metadata (mint/maxt/stats). These
//! tests pin the exact selection semantics: range alignment via integer
//! division, the "spans full range OR sits before the newest block" rule, the
//! overlap-first priority, and the >5%-tombstone rewrite fallback.

use cave_metrics::tsdb::planner::{
    exponential_block_ranges, BlockMeta, BlockStats, Planner,
};

fn meta(dir: &str, mint: i64, maxt: i64) -> BlockMeta {
    BlockMeta {
        dir: dir.to_string(),
        min_time: mint,
        max_time: maxt,
        stats: BlockStats::default(),
        compaction_failed: false,
    }
}

#[test]
fn exponential_block_ranges_matches_upstream() {
    // exponential_block_ranges(minSize, steps, stepSize):
    //   ranges[i] = minSize * stepSize^i
    let ranges = exponential_block_ranges(2 * 3_600_000, 6, 3);
    assert_eq!(
        ranges,
        vec![
            2 * 3_600_000,        // 2h
            6 * 3_600_000,        // 6h
            18 * 3_600_000,       // 18h
            54 * 3_600_000,       // 54h
            162 * 3_600_000,      // 162h
            486 * 3_600_000,      // 486h
        ]
    );
}

#[test]
fn exponential_block_ranges_step_two() {
    let ranges = exponential_block_ranges(1000, 4, 2);
    assert_eq!(ranges, vec![1000, 2000, 4000, 8000]);
}

#[test]
fn plan_selects_full_range_aligned_group() {
    // Three consecutive 2h blocks that together fill a 6h range, plus a newest
    // block that must be excluded from range selection. ranges = [2h, 6h, 18h].
    let two_h = 2 * 3_600_000;
    let planner = Planner::new(exponential_block_ranges(two_h, 3, 3));

    let blocks = vec![
        meta("b0", 0, two_h),               // [0h, 2h)
        meta("b1", two_h, 2 * two_h),       // [2h, 4h)
        meta("b2", 2 * two_h, 3 * two_h),   // [4h, 6h)  -> together span the 6h range
        meta("newest", 3 * two_h, 4 * two_h),
    ];

    let selected = planner.plan(blocks);
    // The 6h range [0,6h) is full (3 blocks) and the newest is excluded.
    assert_eq!(selected, vec!["b0", "b1", "b2"]);
}

#[test]
fn plan_returns_empty_when_no_full_range() {
    // Two 2h blocks only; the newest is dropped before selectDirs, leaving one
    // block — no group of len>1 can be formed, so nothing is planned.
    let two_h = 2 * 3_600_000;
    let planner = Planner::new(exponential_block_ranges(two_h, 3, 3));
    let blocks = vec![
        meta("b0", 0, two_h),
        meta("newest", two_h, 2 * two_h),
    ];
    assert!(planner.plan(blocks).is_empty());
}

#[test]
fn plan_prioritises_overlapping_blocks() {
    // b1 starts before b0 ends -> they overlap. Overlap selection wins over
    // range selection, and returns the consecutive overlapping run.
    let two_h = 2 * 3_600_000;
    let planner = Planner::new(exponential_block_ranges(two_h, 3, 3));
    let blocks = vec![
        meta("b0", 0, two_h + 500),
        meta("b1", two_h, 2 * two_h), // min_time (2h) < globalMaxt (2h+500) -> overlap
        meta("b2", 5 * two_h, 6 * two_h),
        meta("b3", 6 * two_h, 7 * two_h),
    ];
    let selected = planner.plan(blocks);
    assert_eq!(selected, vec!["b0", "b1"]);
}

#[test]
fn plan_rewrites_big_block_with_excess_tombstones() {
    // The 5%-tombstone rewrite only applies to blocks "big enough" — span
    // >= ranges[len/2] (= ranges[1] = 6h here). A 6h block that doesn't form a
    // larger compactable group but carries >5% tombstones is rewritten alone.
    let two_h = 2 * 3_600_000;
    let six_h = 6 * 3_600_000;
    let planner = Planner::new(exponential_block_ranges(two_h, 3, 3)); // [2h,6h,18h]
    let mut m = meta("tomb", 0, six_h); // span 6h >= 6h
    m.stats = BlockStats { num_series: 100, num_tombstones: 6 }; // 6/(100+1) > 0.05
    // selectDirs drops the newest; a lone 6h block forms no len>1 group.
    let blocks = vec![m, meta("newest", six_h, six_h + two_h)];
    let selected = planner.plan(blocks);
    assert_eq!(selected, vec!["tomb"]);
}

#[test]
fn plan_no_rewrite_below_tombstone_threshold() {
    let two_h = 2 * 3_600_000;
    let six_h = 6 * 3_600_000;
    let planner = Planner::new(exponential_block_ranges(two_h, 3, 3));
    let mut m = meta("tomb", 0, six_h);
    m.stats = BlockStats { num_series: 100, num_tombstones: 4 }; // 4/101 < 0.05
    let blocks = vec![m, meta("newest", six_h, six_h + two_h)];
    assert!(planner.plan(blocks).is_empty());
}

#[test]
fn plan_no_rewrite_small_block_partially_deleted() {
    // A SMALL block (span < ranges[len/2]) is NOT subject to the 5% rule and is
    // only rewritten when entirely deleted — here it breaks out untouched.
    let two_h = 2 * 3_600_000;
    let planner = Planner::new(exponential_block_ranges(two_h, 3, 3));
    let mut m = meta("small", 0, two_h); // span 2h < 6h
    m.stats = BlockStats { num_series: 100, num_tombstones: 50 }; // 50% but small block
    let blocks = vec![m, meta("newest", two_h, 2 * two_h)];
    assert!(planner.plan(blocks).is_empty());
}

#[test]
fn plan_rewrites_fully_deleted_small_block() {
    // num_tombstones >= num_series on a small block -> rewrite (entirely deleted).
    let two_h = 2 * 3_600_000;
    let planner = Planner::new(exponential_block_ranges(two_h, 3, 3));
    let mut m = meta("dead", 0, two_h); // span 2h < 6h
    m.stats = BlockStats { num_series: 10, num_tombstones: 10 };
    let blocks = vec![m, meta("newest", two_h, 2 * two_h)];
    assert_eq!(planner.plan(blocks), vec!["dead"]);
}
