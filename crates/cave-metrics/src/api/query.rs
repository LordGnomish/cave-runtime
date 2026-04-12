//! Instant and range query API handlers.

use std::sync::Arc;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use crate::promql::{parser::parse, EvalContext, QueryValue};
use crate::state::MetricsState;

#[derive(Deserialize)]
pub struct InstantQueryParams {
    pub query: String,
    pub time: Option<f64>,
}

#[derive(Deserialize)]
pub struct RangeQueryParams {
    pub query: String,
    pub start: f64,
    pub end: f64,
    pub step: f64,
}

pub async fn instant_query(
    State(state): State<Arc<MetricsState>>,
    Query(params): Query<InstantQueryParams>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let ts_ms = params.time
        .map(|t| (t * 1000.0) as i64)
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());

    let expr = parse(&params.query).map_err(|e| {
        (StatusCode::BAD_REQUEST, Json(json!({"status":"error","error":e.to_string()})))
    })?;

    let ctx = EvalContext {
        timestamp_ms: ts_ms,
        lookback_ms: 5 * 60 * 1000,
        step_ms: 0,
        start_ms: ts_ms,
        end_ms: ts_ms,
    };

    let result = state.engine.eval_instant(&expr, &ctx, &state.tsdb).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"status":"error","error":e.to_string()})))
    })?;

    let data = format_instant_result(result, ts_ms);
    Ok(Json(json!({"status":"success","data":data})))
}

pub async fn range_query(
    State(state): State<Arc<MetricsState>>,
    Query(params): Query<RangeQueryParams>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let start_ms = (params.start * 1000.0) as i64;
    let end_ms = (params.end * 1000.0) as i64;
    let step_ms = (params.step * 1000.0) as i64;

    let expr = parse(&params.query).map_err(|e| {
        (StatusCode::BAD_REQUEST, Json(json!({"status":"error","error":e.to_string()})))
    })?;

    let ctx = EvalContext {
        timestamp_ms: start_ms,
        lookback_ms: step_ms.max(5 * 60 * 1000),
        step_ms,
        start_ms,
        end_ms,
    };

    let steps = state.engine.eval_range(&expr, &ctx, &state.tsdb).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"status":"error","error":e.to_string()})))
    })?;

    // Build matrix format
    let mut series_map: std::collections::HashMap<u64, (serde_json::Map<String, Value>, Vec<Value>)> = std::collections::HashMap::new();
    for (ts, val) in steps {
        if let QueryValue::InstantVector(iv) = val {
            for s in iv {
                let fp = s.labels.fingerprint();
                let entry = series_map.entry(fp).or_insert_with(|| {
                    let metric: serde_json::Map<String, Value> = s.labels.0.iter()
                        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                        .collect();
                    (metric, Vec::new())
                });
                entry.1.push(json!([ts as f64 / 1000.0, s.value.to_string()]));
            }
        }
    }

    let result: Vec<Value> = series_map.into_values().map(|(metric, values)| {
        json!({"metric": metric, "values": values})
    }).collect();

    Ok(Json(json!({
        "status": "success",
        "data": {
            "resultType": "matrix",
            "result": result
        }
    })))
}

fn format_instant_result(val: QueryValue, ts_ms: i64) -> Value {
    match val {
        QueryValue::InstantVector(iv) => {
            let result: Vec<Value> = iv.into_iter().map(|s| {
                let metric: serde_json::Map<String, Value> = s.labels.0.into_iter()
                    .map(|(k, v)| (k, Value::String(v)))
                    .collect();
                json!({
                    "metric": metric,
                    "value": [s.timestamp as f64 / 1000.0, s.value.to_string()]
                })
            }).collect();
            json!({"resultType": "vector", "result": result})
        }
        QueryValue::Scalar(n) => {
            json!({"resultType": "scalar", "result": [ts_ms as f64 / 1000.0, n.to_string()]})
        }
        QueryValue::Str(s) => {
            json!({"resultType": "string", "result": [ts_ms as f64 / 1000.0, s]})
        }
        QueryValue::RangeVector(_) => {
            json!({"resultType": "matrix", "result": []})
        }
    }
}
