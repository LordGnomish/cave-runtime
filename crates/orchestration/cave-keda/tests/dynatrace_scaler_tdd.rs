// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! RED → GREEN TDD for the Dynatrace scaler line-port.
//!
//! Faithful port of kedacore/keda v2.16.1 pkg/scalers/dynatrace_scaler.go:
//!   - `validateDynatraceResponse`  (three-layer nested-structure validation)
//!   - response value extraction     (`Result[0].Data[0].Values[0]`)
//!   - `GetMetricValue` URL building (TrimRight host + api path + default `from`)
//!   - `GetMetricsAndActivity`       (`val > activationThreshold`)
//!
//! Pure decision logic only — no network.

use cave_keda::scaler::ScalerTrait;
use cave_keda::dynatrace_scaler::{DynatraceResponse, DynatraceScaler, DynatraceSeries};

// ─── validateDynatraceResponse ──────────────────────────────────────────────

#[test]
fn validate_response_empty_results_errors() {
    let resp = DynatraceResponse { result: vec![] };
    let err = resp.validate().unwrap_err();
    assert!(err.contains("does not contain any results"), "got: {err}");
}

#[test]
fn validate_response_no_series_errors() {
    let resp = DynatraceResponse {
        result: vec![DynatraceSeries { data: vec![] }],
    };
    let err = resp.validate().unwrap_err();
    assert!(err.contains("does not contain any metric series"), "got: {err}");
}

#[test]
fn validate_response_no_values_errors() {
    let resp = DynatraceResponse {
        result: vec![DynatraceSeries {
            data: vec![vec![]],
        }],
    };
    let err = resp.validate().unwrap_err();
    assert!(err.contains("does not contain any values"), "got: {err}");
}

#[test]
fn validate_response_well_formed_ok() {
    let resp = DynatraceResponse {
        result: vec![DynatraceSeries {
            data: vec![vec![3.5, 9.0]],
        }],
    };
    assert!(resp.validate().is_ok());
}

// ─── value extraction (Result[0].Data[0].Values[0]) ─────────────────────────

#[test]
fn first_value_returns_leading_datapoint() {
    let resp = DynatraceResponse {
        result: vec![DynatraceSeries {
            data: vec![vec![7.25, 1.0, 2.0]],
        }],
    };
    assert_eq!(resp.first_value().unwrap(), 7.25);
}

#[test]
fn first_value_propagates_validation_error() {
    let resp = DynatraceResponse { result: vec![] };
    assert!(resp.first_value().is_err());
}

// ─── URL building (GetMetricValue) ──────────────────────────────────────────

#[test]
fn build_query_url_trims_trailing_slash_and_appends_api_path() {
    let url = DynatraceScaler::build_query_url(
        "https://abc.live.dynatrace.com/",
        "builtin:host.cpu.usage",
        "now-2h",
    );
    assert!(url.starts_with("https://abc.live.dynatrace.com/api/v2/metrics/query?"), "got: {url}");
    assert!(url.contains("metricSelector=builtin%3Ahost.cpu.usage"), "got: {url}");
    assert!(url.contains("from=now-2h"), "got: {url}");
}

// ─── activation gate ────────────────────────────────────────────────────────

#[test]
fn is_active_when_value_exceeds_activation_threshold() {
    let mut s = DynatraceScaler::new("https://x.dynatrace.com", "builtin:cpu", "token");
    s.activation_threshold = 5.0;
    s.observe(4.0);
    assert!(!s.is_active());
    s.observe(6.0);
    assert!(s.is_active());
}

#[test]
fn default_from_timestamp_is_now_minus_2h() {
    let s = DynatraceScaler::new("https://x.dynatrace.com", "builtin:cpu", "token");
    assert_eq!(s.from_timestamp, "now-2h");
}
