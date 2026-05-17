// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! /api/v1/query and /api/v1/query_range handlers.

use axum::{
    extract::{Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use crate::model::QueryResult;
use crate::promql::parse;
use crate::state::MetricsState;

// ─── Request / response types ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct InstantQueryParams {
    pub query: String,
    pub time: Option<String>,
    pub timeout: Option<String>,
    pub limit: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct RangeQueryParams {
    pub query: String,
    pub start: String,
    pub end: String,
    pub step: String,
    pub timeout: Option<String>,
    pub limit: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct ExemplarParams {
    pub query: String,
    pub start: Option<String>,
    pub end: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub status: String,
    pub data: T,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "errorType")]
    pub error_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<String>>,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn success(data: T) -> Self {
        Self { status: "success".into(), data, error: None, error_type: None, warnings: None }
    }
}

fn api_error(err_type: &str, msg: &str) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "error",
        "errorType": err_type,
        "error": msg,
    }))
}

// ─── Handlers ────────────────────────────────────────────────────────────────

pub async fn instant_query(
    State(state): State<Arc<MetricsState>>,
    Query(params): Query<InstantQueryParams>,
) -> Json<serde_json::Value> {
    let ts_ms = parse_time_param(params.time.as_deref()).unwrap_or_else(now_ms);

    let ast = match parse(&params.query) {
        Ok(a) => a,
        Err(e) => return api_error("bad_data", &e.to_string()),
    };

    match state.engine.eval_instant(&ast, ts_ms) {
        Ok(result) => Json(serde_json::json!({
            "status": "success",
            "data": query_result_to_json(result, ts_ms),
        })),
        Err(e) => api_error("execution", &e.to_string()),
    }
}

pub async fn range_query(
    State(state): State<Arc<MetricsState>>,
    Query(params): Query<RangeQueryParams>,
) -> Json<serde_json::Value> {
    let start_ms = match parse_time_param(Some(&params.start)) {
        Some(t) => t,
        None => return api_error("bad_data", "invalid start"),
    };
    let end_ms = match parse_time_param(Some(&params.end)) {
        Some(t) => t,
        None => return api_error("bad_data", "invalid end"),
    };
    let step_ms = match parse_duration_param(&params.step) {
        Some(d) => d,
        None => return api_error("bad_data", "invalid step"),
    };

    let ast = match parse(&params.query) {
        Ok(a) => a,
        Err(e) => return api_error("bad_data", &e.to_string()),
    };

    match state.engine.eval_range(&ast, start_ms, end_ms, step_ms) {
        Ok(steps) => {
            // Group by series fingerprint across steps
            let mut series_map: std::collections::HashMap<u64, (crate::model::Labels, Vec<[serde_json::Value; 2]>)> = HashMap::new();
            for (ts_ms, result) in steps {
                if let QueryResult::InstantVector(iv) = result {
                    for (labels, val) in iv {
                        let fp = labels.fingerprint();
                        let entry = series_map.entry(fp).or_insert_with(|| (labels, Vec::new()));
                        entry.1.push([
                            serde_json::json!(ts_ms as f64 / 1000.0),
                            serde_json::json!(val.to_string()),
                        ]);
                    }
                }
            }

            let result_vec: Vec<serde_json::Value> = series_map.into_values().map(|(labels, values)| {
                serde_json::json!({
                    "metric": labels.0,
                    "values": values,
                })
            }).collect();

            Json(serde_json::json!({
                "status": "success",
                "data": {
                    "resultType": "matrix",
                    "result": result_vec,
                }
            }))
        }
        Err(e) => api_error("execution", &e.to_string()),
    }
}

pub async fn exemplars(Query(_params): Query<ExemplarParams>) -> Json<serde_json::Value> {
    // Exemplars not yet stored; return empty result.
    Json(serde_json::json!({ "status": "success", "data": [] }))
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn query_result_to_json(result: QueryResult, ts_ms: i64) -> serde_json::Value {
    match result {
        QueryResult::Scalar(v) => serde_json::json!({
            "resultType": "scalar",
            "result": [ts_ms as f64 / 1000.0, v.to_string()],
        }),
        QueryResult::String(s) => serde_json::json!({
            "resultType": "string",
            "result": [ts_ms as f64 / 1000.0, s],
        }),
        QueryResult::InstantVector(iv) => {
            let result: Vec<serde_json::Value> = iv.into_iter().map(|(labels, val)| {
                serde_json::json!({
                    "metric": labels.0,
                    "value": [ts_ms as f64 / 1000.0, val.to_string()],
                })
            }).collect();
            serde_json::json!({ "resultType": "vector", "result": result })
        }
        QueryResult::RangeVector(rv) => {
            let result: Vec<serde_json::Value> = rv.into_iter().map(|(labels, samps)| {
                let values: Vec<serde_json::Value> = samps.iter().map(|s| {
                    serde_json::json!([s.timestamp_ms as f64 / 1000.0, s.value.to_string()])
                }).collect();
                serde_json::json!({ "metric": labels.0, "values": values })
            }).collect();
            serde_json::json!({ "resultType": "matrix", "result": result })
        }
    }
}

/// Parse a time parameter: Unix seconds (float) or RFC3339.
fn parse_time_param(s: Option<&str>) -> Option<i64> {
    let s = s?;
    if let Ok(f) = s.parse::<f64>() {
        return Some((f * 1000.0) as i64);
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp_millis());
    }
    None
}

/// Parse a duration parameter: seconds (float) or Prometheus duration string.
fn parse_duration_param(s: &str) -> Option<i64> {
    if let Ok(f) = s.parse::<f64>() {
        return Some((f * 1000.0) as i64);
    }
    // Prometheus duration: 15s, 1m, 5m, 1h, 1d, etc.
    parse_prometheus_duration(s)
}

fn parse_prometheus_duration(s: &str) -> Option<i64> {
    let mut total_ms: i64 = 0;
    let mut num_buf = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            num_buf.push(c);
        } else {
            let n: i64 = num_buf.parse().ok()?;
            num_buf.clear();
            let ms = match c {
                's' => n * 1_000,
                'm' => n * 60_000,
                'h' => n * 3_600_000,
                'd' => n * 86_400_000,
                'w' => n * 604_800_000,
                'y' => n * 31_536_000_000,
                _   => return None,
            };
            total_ms += ms;
        }
    }
    if total_ms == 0 && !num_buf.is_empty() {
        return num_buf.parse::<i64>().ok().map(|n| n * 1000);
    }
    if total_ms > 0 { Some(total_ms) } else { None }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
