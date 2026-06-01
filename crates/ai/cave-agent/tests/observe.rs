// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Self-improvement step 1 — observability data analysis: summary statistics
//! over a metric series and baseline-vs-recent regression detection.

use cave_agent::observe::{detect_regression, Series};

#[test]
fn series_summary_statistics() {
    let s = Series::from(vec![2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]);
    assert_eq!(s.len(), 8);
    assert_eq!(s.mean(), 5.0);
    assert_eq!(s.min(), 2.0);
    assert_eq!(s.max(), 9.0);
    // population standard deviation of the classic dataset is exactly 2.0
    assert!((s.stddev() - 2.0).abs() < 1e-9, "stddev was {}", s.stddev());
}

#[test]
fn series_percentile_nearest_rank() {
    let s = Series::from((1..=10).map(|v| v as f64).collect::<Vec<_>>());
    assert_eq!(s.percentile(50.0), 5.0);
    assert_eq!(s.percentile(90.0), 9.0);
    assert_eq!(s.percentile(100.0), 10.0);
}

#[test]
fn empty_series_is_safe() {
    let s = Series::from(vec![]);
    assert_eq!(s.len(), 0);
    assert_eq!(s.mean(), 0.0);
    assert_eq!(s.stddev(), 0.0);
    assert_eq!(s.percentile(95.0), 0.0);
}

#[test]
fn regression_detected_when_recent_much_worse() {
    // baseline latency ~100ms, tight; recent jumps to ~200ms.
    let baseline = vec![98.0, 100.0, 102.0, 99.0, 101.0];
    let recent = vec![195.0, 205.0, 200.0];
    let r = detect_regression(&baseline, &recent, 3.0);
    assert!(r.regressed, "z={} should exceed threshold", r.z_score);
    assert!(r.recent_mean > r.baseline_mean);
    assert!(r.delta_pct > 90.0 && r.delta_pct < 110.0, "delta {}", r.delta_pct);
}

#[test]
fn no_regression_when_recent_similar() {
    let baseline = vec![100.0, 101.0, 99.0, 100.0, 100.0];
    let recent = vec![100.0, 99.0, 101.0];
    let r = detect_regression(&baseline, &recent, 3.0);
    assert!(!r.regressed);
}

#[test]
fn improvement_is_not_flagged_as_regression() {
    let baseline = vec![200.0, 205.0, 195.0, 200.0];
    let recent = vec![100.0, 98.0, 102.0];
    let r = detect_regression(&baseline, &recent, 3.0);
    assert!(!r.regressed, "a speed-up must not be a regression");
    assert!(r.delta_pct < 0.0);
}
