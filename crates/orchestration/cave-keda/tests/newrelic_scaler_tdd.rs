// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! RED → GREEN TDD for the New Relic scaler line-port.
//!
//! Faithful port of kedacore/keda v2.16.1 pkg/scalers/newrelic_scaler.go:
//!   - `executeNewRelicQuery` result extraction (empty-results + noDataError
//!     handling, first-float-of-first-row selection)
//!   - `parseNewRelicMetadata` default region "US"
//!   - `GetMetricsAndActivity` activation gate (`val > activationThreshold`)
//!
//! Pure decision logic only — no NRDB network call.

use cave_keda::scaler::ScalerTrait;
use cave_keda::newrelic_scaler::{NewRelicScaler, NrdbValue};

// ─── executeNewRelicQuery result extraction ─────────────────────────────────

#[test]
fn extract_metric_picks_first_float_of_first_row() {
    // first row has a string then a float — the float wins (Go ranges the map
    // and returns the first value that type-asserts to float64; we model the
    // row as an ordered slice so "first float" is deterministic).
    let results = vec![vec![
        NrdbValue::Str("label".to_string()),
        NrdbValue::Float(12.5),
    ]];
    assert_eq!(
        NewRelicScaler::extract_metric(&results, false).unwrap(),
        12.5
    );
}

#[test]
fn extract_metric_uses_only_first_row() {
    let results = vec![vec![NrdbValue::Float(1.0)], vec![NrdbValue::Float(99.0)]];
    assert_eq!(NewRelicScaler::extract_metric(&results, false).unwrap(), 1.0);
}

#[test]
fn extract_metric_empty_results_no_data_error_false_returns_zero() {
    let results: Vec<Vec<NrdbValue>> = vec![];
    assert_eq!(NewRelicScaler::extract_metric(&results, false).unwrap(), 0.0);
}

#[test]
fn extract_metric_empty_results_no_data_error_true_errors() {
    let results: Vec<Vec<NrdbValue>> = vec![];
    assert!(NewRelicScaler::extract_metric(&results, true).is_err());
}

#[test]
fn extract_metric_no_float_in_row_no_data_error_false_returns_zero() {
    let results = vec![vec![NrdbValue::Str("x".to_string())]];
    assert_eq!(NewRelicScaler::extract_metric(&results, false).unwrap(), 0.0);
}

#[test]
fn extract_metric_no_float_in_row_no_data_error_true_errors() {
    let results = vec![vec![NrdbValue::Str("x".to_string())]];
    assert!(NewRelicScaler::extract_metric(&results, true).is_err());
}

// ─── default region ─────────────────────────────────────────────────────────

#[test]
fn default_region_is_us() {
    let s = NewRelicScaler::new(12345, "SELECT count(*) FROM Transaction");
    assert_eq!(s.region, "US");
}

// ─── activation gate ────────────────────────────────────────────────────────

#[test]
fn is_active_when_value_exceeds_activation_threshold() {
    let mut s = NewRelicScaler::new(1, "SELECT 1");
    s.activation_threshold = 10.0;
    s.observe(9.0);
    assert!(!s.is_active());
    s.observe(11.0);
    assert!(s.is_active());
}

#[test]
fn metric_value_returns_last_observation() {
    let mut s = NewRelicScaler::new(1, "SELECT 1");
    s.observe(3.0);
    assert_eq!(s.metric_value(), Some(3.0));
}
