// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Prometheus-compatible data models.
//!
//! Mirrors the Prometheus HTTP API response envelope and data types
//! so that any Prometheus-aware client can talk to cave-metrics unchanged.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Prometheus HTTP API response envelope
// ---------------------------------------------------------------------------

/// Top-level response returned by all query/metadata endpoints.
/// Matches: {"status":"success","data":{...}} or {"status":"error",...}
#[derive(Debug, Serialize)]
pub struct PromResponse<T: Serialize> {
    pub status: &'static str,
    pub data: T,
}

impl<T: Serialize> PromResponse<T> {
    pub fn success(data: T) -> Self {
        Self { status: "success", data }
    }
}

/// {"status":"error","errorType":"...","error":"..."}
#[derive(Debug, Serialize)]
pub struct PromError {
    pub status: &'static str,
    #[serde(rename = "errorType")]
    pub error_type: String,
    pub error: String,
}

// ---------------------------------------------------------------------------
// Query result types
// ---------------------------------------------------------------------------

/// resultType discriminant used in query responses.
#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ResultType {
    Matrix,
    Vector,
    Scalar,
    String,
}

/// Instant query response data: {"resultType":"vector","result":[...]}
#[derive(Debug, Serialize)]
pub struct VectorData {
    #[serde(rename = "resultType")]
    pub result_type: &'static str,
    pub result: Vec<InstantSample>,
}

/// Range query response data: {"resultType":"matrix","result":[...]}
#[derive(Debug, Serialize)]
pub struct MatrixData {
    #[serde(rename = "resultType")]
    pub result_type: &'static str,
    pub result: Vec<RangeSample>,
}

/// A single instant sample: {"metric":{labels},"value":[timestamp,"value"]}
#[derive(Debug, Serialize)]
pub struct InstantSample {
    pub metric: HashMap<String, String>,
    /// [unix_timestamp_float, "value_string"]
    pub value: (f64, String),
}

/// A range sample: {"metric":{labels},"values":[[ts,"v"],...]}
#[derive(Debug, Serialize)]
pub struct RangeSample {
    pub metric: HashMap<String, String>,
    /// [[unix_timestamp_float, "value_string"], ...]
    pub values: Vec<(f64, String)>,
}

// ---------------------------------------------------------------------------
// Series / label metadata
// ---------------------------------------------------------------------------

/// Response for /api/v1/series — list of label sets.
#[derive(Debug, Serialize)]
pub struct SeriesData {
    pub data: Vec<HashMap<String, String>>,
}

/// Response for /api/v1/labels — list of label names.
#[derive(Debug, Serialize)]
pub struct LabelsData {
    pub data: Vec<String>,
}

/// Response for /api/v1/label/{name}/values — list of label values.
#[derive(Debug, Serialize)]
pub struct LabelValuesData {
    pub data: Vec<String>,
}

// ---------------------------------------------------------------------------
// Query parameters (deserialised from query string)
// ---------------------------------------------------------------------------

/// Query parameters for GET /api/v1/query
#[derive(Debug, Deserialize)]
pub struct InstantQueryParams {
    /// PromQL expression
    pub query: String,
    /// Evaluation timestamp (RFC3339 or Unix seconds). Optional → now.
    pub time: Option<String>,
    /// Evaluation timeout, e.g. "30s". Optional.
    pub timeout: Option<String>,
}

/// Query parameters for GET /api/v1/query_range
#[derive(Debug, Deserialize)]
pub struct RangeQueryParams {
    /// PromQL expression
    pub query: String,
    /// Start of range (RFC3339 or Unix seconds)
    pub start: String,
    /// End of range (RFC3339 or Unix seconds)
    pub end: String,
    /// Resolution step, e.g. "15s" or "60"
    pub step: String,
    /// Evaluation timeout. Optional.
    pub timeout: Option<String>,
}

/// Query parameters for GET /api/v1/series
#[derive(Debug, Deserialize)]
pub struct SeriesParams {
    /// One or more series selectors, e.g. `match[]=up`
    #[serde(rename = "match[]")]
    pub matchers: Option<Vec<String>>,
    pub start: Option<String>,
    pub end: Option<String>,
}

/// Query parameters for GET /api/v1/labels
#[derive(Debug, Deserialize)]
pub struct LabelsParams {
    #[serde(rename = "match[]")]
    pub matchers: Option<Vec<String>>,
    pub start: Option<String>,
    pub end: Option<String>,
}

/// Query parameters for GET /api/v1/label/{name}/values
#[derive(Debug, Deserialize)]
pub struct LabelValuesParams {
    #[serde(rename = "match[]")]
    pub matchers: Option<Vec<String>>,
    pub start: Option<String>,
    pub end: Option<String>,
}
