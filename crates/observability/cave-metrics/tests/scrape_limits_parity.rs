// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Parity tests for scrape-time label and sample limits.
//!
//! Upstream: prometheus/prometheus `scrape/scrape.go` v3.12.0
//!   * `verifyLabelLimits(lset, *labelLimits)` — enforces `label_limit`,
//!     `label_name_length_limit`, `label_value_length_limit`. A limit of 0
//!     means "disabled". Errors carry the offending metric/label.
//!   * `limitAppender.Append` — enforces `sample_limit`: stale markers
//!     (StaleNaN) are exempt; once the running non-stale count would exceed
//!     the limit the append is rejected with `errSampleLimit`.

use cave_metrics::model::Labels;
use cave_metrics::scrape::limits::{
    is_stale_nan, verify_label_limits, LabelLimits, SampleLimiter, STALE_NAN,
};

fn lset(pairs: &[(&str, &str)]) -> Labels {
    Labels::from_pairs(pairs.iter().copied())
}

#[test]
fn label_limit_zero_is_disabled() {
    let l = lset(&[("__name__", "m"), ("a", "1"), ("b", "2"), ("c", "3")]);
    let limits = LabelLimits::default(); // all zero
    assert!(verify_label_limits(&l, &limits).is_ok());
}

#[test]
fn label_limit_exceeded() {
    // 4 labels including __name__ — limit of 3 must reject.
    let l = lset(&[("__name__", "m"), ("a", "1"), ("b", "2"), ("c", "3")]);
    let limits = LabelLimits { label_limit: 3, ..Default::default() };
    let err = verify_label_limits(&l, &limits).unwrap_err();
    assert!(err.to_string().contains("label_limit exceeded"), "got {err}");
}

#[test]
fn label_limit_exactly_at_bound_is_ok() {
    let l = lset(&[("__name__", "m"), ("a", "1"), ("b", "2")]); // 3 labels
    let limits = LabelLimits { label_limit: 3, ..Default::default() };
    assert!(verify_label_limits(&l, &limits).is_ok());
}

#[test]
fn label_name_length_limit_exceeded() {
    let l = lset(&[("__name__", "m"), ("toolongname", "v")]);
    let limits = LabelLimits { label_name_length_limit: 5, ..Default::default() };
    let err = verify_label_limits(&l, &limits).unwrap_err();
    assert!(
        err.to_string().contains("label_name_length_limit exceeded"),
        "got {err}"
    );
}

#[test]
fn label_value_length_limit_exceeded() {
    let l = lset(&[("__name__", "m"), ("k", "way_too_long_value")]);
    let limits = LabelLimits { label_value_length_limit: 5, ..Default::default() };
    let err = verify_label_limits(&l, &limits).unwrap_err();
    assert!(
        err.to_string().contains("label_value_length_limit exceeded"),
        "got {err}"
    );
}

#[test]
fn length_limits_within_bounds_ok() {
    let l = lset(&[("__name__", "m"), ("k", "v")]);
    let limits = LabelLimits {
        label_name_length_limit: 10,
        label_value_length_limit: 10,
        ..Default::default()
    };
    assert!(verify_label_limits(&l, &limits).is_ok());
}

// ─── sample_limit ────────────────────────────────────────────────────────────

#[test]
fn sample_limiter_accepts_up_to_limit() {
    let mut lim = SampleLimiter::new(3);
    assert!(lim.accept(1.0).is_ok());
    assert!(lim.accept(2.0).is_ok());
    assert!(lim.accept(3.0).is_ok());
    // 4th exceeds.
    let err = lim.accept(4.0).unwrap_err();
    assert!(err.to_string().contains("sample limit"), "got {err}");
}

#[test]
fn sample_limiter_zero_means_unlimited() {
    let mut lim = SampleLimiter::new(0);
    for i in 0..1000 {
        assert!(lim.accept(i as f64).is_ok());
    }
}

#[test]
fn stale_markers_are_exempt_from_sample_limit() {
    let mut lim = SampleLimiter::new(2);
    assert!(lim.accept(1.0).is_ok());
    assert!(lim.accept(2.0).is_ok());
    // Stale-NaN samples do not count toward the limit.
    assert!(lim.accept(STALE_NAN).is_ok());
    assert!(lim.accept(STALE_NAN).is_ok());
    // A real sample now still exceeds.
    assert!(lim.accept(3.0).is_err());
}

#[test]
fn stale_nan_recognised_but_ordinary_nan_is_not() {
    assert!(is_stale_nan(STALE_NAN));
    assert!(!is_stale_nan(f64::NAN));
    assert!(!is_stale_nan(1.0));
}
