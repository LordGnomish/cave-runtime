// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! RED → GREEN TDD for the Splunk scaler line-port.
//!
//! Faithful port of kedacore/keda v2.16.1:
//!   - pkg/scalers/splunk/splunk.go  (SearchResponse.ToMetric, NewClient validation)
//!   - pkg/scalers/splunk_scaler.go  (parseSplunkMetadata host validation,
//!                                    GetMetricsAndActivity activation gate)
//!
//! These exercise only the pure in-crate decision logic: the value-field
//! extraction from a saved-search result map, the float parse, the
//! credential-combination validation, the host URL validation, and the
//! activation threshold — none of which touch the network.

use std::collections::HashMap;

use cave_keda::scaler::ScalerTrait;
use cave_keda::splunk_scaler::{SearchResponse, SplunkScaler, SplunkValidationError};

// ─── SearchResponse::to_metric (pkg/scalers/splunk/splunk.go ToMetric) ──────

#[test]
fn to_metric_extracts_named_value_field() {
    let mut result = HashMap::new();
    result.insert("count".to_string(), "42".to_string());
    let resp = SearchResponse { result };
    assert_eq!(resp.to_metric("count").unwrap(), 42.0);
}

#[test]
fn to_metric_missing_field_errors() {
    let resp = SearchResponse {
        result: HashMap::new(),
    };
    let err = resp.to_metric("count").unwrap_err();
    assert!(err.contains("not found"), "got: {err}");
}

#[test]
fn to_metric_non_float_value_errors() {
    let mut result = HashMap::new();
    result.insert("count".to_string(), "not-a-number".to_string());
    let resp = SearchResponse { result };
    let err = resp.to_metric("count").unwrap_err();
    assert!(err.contains("not a float"), "got: {err}");
}

// ─── credential validation (pkg/scalers/splunk/splunk.go NewClient) ─────────

#[test]
fn validate_credentials_requires_username() {
    let err = SplunkScaler::validate_credentials("", "tok", "").unwrap_err();
    assert_eq!(err, SplunkValidationError::UsernameNotSet);
}

#[test]
fn validate_credentials_rejects_token_and_password_together() {
    let err = SplunkScaler::validate_credentials("admin", "tok", "pw").unwrap_err();
    assert_eq!(err, SplunkValidationError::TokenAndPasswordBothSet);
}

#[test]
fn validate_credentials_accepts_username_with_token_only() {
    assert!(SplunkScaler::validate_credentials("admin", "tok", "").is_ok());
}

#[test]
fn validate_credentials_accepts_username_with_password_only() {
    assert!(SplunkScaler::validate_credentials("admin", "", "pw").is_ok());
}

// ─── host validation (pkg/scalers/splunk_scaler.go parseSplunkMetadata) ─────

#[test]
fn validate_host_requires_absolute_url() {
    assert!(SplunkScaler::validate_host("https://localhost:8089").is_ok());
    assert!(SplunkScaler::validate_host("not a url").is_err());
    assert!(SplunkScaler::validate_host("localhost:8089").is_err());
}

// ─── activation gate (pkg/scalers/splunk_scaler.go GetMetricsAndActivity) ───

#[test]
fn is_active_when_metric_exceeds_activation_value() {
    let mut s = SplunkScaler::new("admin", "search1", "count");
    s.activation_value = 10;
    s.observe(8.0);
    assert!(!s.is_active());
    s.observe(11.0);
    assert!(s.is_active());
}

#[test]
fn metric_value_returns_last_observation() {
    let mut s = SplunkScaler::new("admin", "search1", "count");
    s.observe(7.0);
    assert_eq!(s.metric_value(), Some(7.0));
}
