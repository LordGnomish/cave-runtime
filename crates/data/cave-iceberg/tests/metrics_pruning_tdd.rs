// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Strict-TDD RED — manifest-time inclusive-metrics file pruning.
//!
//! Promotes parity partial #3 (`crates/iceberg/src/spec/partition_spec.rs`
//! "manifest-time bound pruning is not yet hooked into ScanBuilder") to a
//! real, tested mapped entry by line-porting upstream
//! `crates/iceberg/src/expr/visitors/inclusive_metrics_evaluator.rs`.
//!
//! The InclusiveMetricsEvaluator decides — purely from a DataFile's
//! per-column lower/upper bounds, value-counts and null-counts — whether a
//! file *might* contain rows matching a predicate. It never reads the file;
//! it returns `true` ("rows might match, scan it") or `false` ("rows cannot
//! match, prune it"). This is the core data-skipping algorithm. It is a
//! pure in-crate calculator with no cross-crate dependency.
//!
//! 2026-05-30 — Wave-4 honest TDD conversion.

use cave_iceberg::manifest::{DataFile, FileFormat};
use cave_iceberg::metrics_eval::InclusiveMetricsEvaluator;
use cave_iceberg::expr::{Predicate, Term};

/// Build a DataFile carrying column stats for field-id `fid`.
/// Bounds are Iceberg single-value binary, hex-encoded (as in the
/// manifest wire form): little-endian for int/long.
fn file_with_i64_bounds(fid: i32, record_count: i64, lower: i64, upper: i64) -> DataFile {
    let mut df = DataFile::new("s3://x/a.parquet", FileFormat::Parquet);
    df.record_count = record_count;
    df.value_counts.insert(fid, record_count);
    df.null_value_counts.insert(fid, 0);
    // 8-byte little-endian, hex-encoded — Iceberg long single-value form.
    df.lower_bounds
        .insert(fid, hex::encode_le_i64(lower));
    df.upper_bounds
        .insert(fid, hex::encode_le_i64(upper));
    df
}

// tiny local hex helper so the test does not depend on impl internals
mod hex {
    pub fn encode_le_i64(v: i64) -> String {
        let bytes = v.to_le_bytes();
        let mut s = String::with_capacity(16);
        for b in bytes {
            s.push_str(&format!("{:02x}", b));
        }
        s
    }
}

#[test]
fn eq_below_lower_bound_is_pruned() {
    // file holds field 1 in [10, 20]; predicate field==5 cannot match.
    let df = file_with_i64_bounds(1, 100, 10, 20);
    let pred = Predicate::eq(Term::ref_col("c"), Term::lit(5));
    let ev = InclusiveMetricsEvaluator::new(&pred, 1);
    assert_eq!(ev.eval(&df), false, "5 < lower(10) → file must be pruned");
}

#[test]
fn eq_inside_bounds_is_kept() {
    let df = file_with_i64_bounds(1, 100, 10, 20);
    let pred = Predicate::eq(Term::ref_col("c"), Term::lit(15));
    let ev = InclusiveMetricsEvaluator::new(&pred, 1);
    assert_eq!(ev.eval(&df), true, "15 ∈ [10,20] → file must be scanned");
}

#[test]
fn eq_above_upper_bound_is_pruned() {
    let df = file_with_i64_bounds(1, 100, 10, 20);
    let pred = Predicate::eq(Term::ref_col("c"), Term::lit(99));
    let ev = InclusiveMetricsEvaluator::new(&pred, 1);
    assert_eq!(ev.eval(&df), false, "99 > upper(20) → file must be pruned");
}

#[test]
fn less_than_lower_is_pruned() {
    // field in [10,20]; predicate field < 10 cannot match (lower==10).
    let df = file_with_i64_bounds(1, 100, 10, 20);
    let pred = Predicate::lt(Term::ref_col("c"), Term::lit(10));
    let ev = InclusiveMetricsEvaluator::new(&pred, 1);
    assert_eq!(ev.eval(&df), false, "lt(10) vs lower 10 → cannot match");
}

#[test]
fn less_than_above_lower_is_kept() {
    let df = file_with_i64_bounds(1, 100, 10, 20);
    let pred = Predicate::lt(Term::ref_col("c"), Term::lit(15));
    let ev = InclusiveMetricsEvaluator::new(&pred, 1);
    assert_eq!(ev.eval(&df), true, "lt(15) vs lower 10 → might match");
}

#[test]
fn greater_than_upper_is_pruned() {
    let df = file_with_i64_bounds(1, 100, 10, 20);
    let pred = Predicate::gt(Term::ref_col("c"), Term::lit(20));
    let ev = InclusiveMetricsEvaluator::new(&pred, 1);
    assert_eq!(ev.eval(&df), false, "gt(20) vs upper 20 → cannot match");
}

#[test]
fn all_null_column_prunes_eq() {
    // value_count == null_count → column is entirely null, eq cannot match.
    let mut df = file_with_i64_bounds(1, 100, 10, 20);
    df.null_value_counts.insert(1, 100); // every row null
    let pred = Predicate::eq(Term::ref_col("c"), Term::lit(15));
    let ev = InclusiveMetricsEvaluator::new(&pred, 1);
    assert_eq!(ev.eval(&df), false, "all-null column → eq cannot match");
}

#[test]
fn is_null_with_no_nulls_is_pruned() {
    let df = file_with_i64_bounds(1, 100, 10, 20); // null_count = 0
    let pred = Predicate::is_null(Term::ref_col("c"));
    let ev = InclusiveMetricsEvaluator::new(&pred, 1);
    assert_eq!(ev.eval(&df), false, "is_null but 0 nulls → cannot match");
}

#[test]
fn is_null_with_nulls_is_kept() {
    let mut df = file_with_i64_bounds(1, 100, 10, 20);
    df.null_value_counts.insert(1, 3);
    let pred = Predicate::is_null(Term::ref_col("c"));
    let ev = InclusiveMetricsEvaluator::new(&pred, 1);
    assert_eq!(ev.eval(&df), true, "is_null with 3 nulls → might match");
}

#[test]
fn is_not_null_all_null_is_pruned() {
    let mut df = file_with_i64_bounds(1, 100, 10, 20);
    df.null_value_counts.insert(1, 100);
    let pred = Predicate::is_not_null(Term::ref_col("c"));
    let ev = InclusiveMetricsEvaluator::new(&pred, 1);
    assert_eq!(ev.eval(&df), false, "is_not_null on all-null → cannot match");
}

#[test]
fn and_prunes_when_either_side_cannot_match() {
    // c ∈ [10,20]; (c==5 AND c==15): c==5 prunes → whole AND cannot match.
    let df = file_with_i64_bounds(1, 100, 10, 20);
    let pred = Predicate::eq(Term::ref_col("c"), Term::lit(5))
        .and(Predicate::eq(Term::ref_col("c"), Term::lit(15)));
    let ev = InclusiveMetricsEvaluator::new(&pred, 1);
    assert_eq!(ev.eval(&df), false, "AND with one impossible side → pruned");
}

#[test]
fn or_keeps_when_one_side_might_match() {
    let df = file_with_i64_bounds(1, 100, 10, 20);
    let pred = Predicate::eq(Term::ref_col("c"), Term::lit(5))
        .or(Predicate::eq(Term::ref_col("c"), Term::lit(15)));
    let ev = InclusiveMetricsEvaluator::new(&pred, 1);
    assert_eq!(ev.eval(&df), true, "OR with one possible side → kept");
}

#[test]
fn unknown_column_is_kept_conservatively() {
    // predicate references a field id with no stats → cannot prune, keep.
    let df = file_with_i64_bounds(1, 100, 10, 20);
    let pred = Predicate::eq(Term::ref_col("other"), Term::lit(5));
    // field id 2 has no bounds in the file
    let ev = InclusiveMetricsEvaluator::new(&pred, 2);
    assert_eq!(ev.eval(&df), true, "no stats for column → conservatively kept");
}
