//! Loki-compatible HTTP routes.
//!
//! POST /loki/api/v1/push
//! GET  /loki/api/v1/query
//! GET  /loki/api/v1/query_range
//! GET  /loki/api/v1/labels
//! GET  /loki/api/v1/label/:name/values
//! GET  /loki/api/v1/series
//! GET  /loki/api/v1/tail       (WebSocket)

use crate::logql::{eval::Evaluator, parser::parse};
use crate::models::{LokiResponse, PushRequest, QueryData};
use crate::push::{ingest_json, ingest_proto};
use crate::tail::handle_tail;
use crate::LogsState;
use axum::{
    body::Bytes,
    extract::{Path, Query, State, WebSocketUpgrade},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use std::sync::Arc;
use tracing::warn;

pub fn create_router(state: Arc<LogsState>) -> Router {
    Router::new()
        .route("/loki/api/v1/push", post(push_handler))
        .route("/loki/api/v1/query", get(query_handler))
        .route("/loki/api/v1/query_range", get(query_range_handler))
        .route("/loki/api/v1/labels", get(labels_handler))
        .route("/loki/api/v1/label/{name}/values", get(label_values_handler))
        .route("/loki/api/v1/series", get(series_handler))
        .route("/loki/api/v1/tail", get(tail_handler))
        .with_state(state)
}

// ─── Tenant extraction ────────────────────────────────────────────────────────

fn extract_tenant(headers: &HeaderMap) -> Option<String> {
    headers
        .get("X-Scope-OrgID")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

// ─── Push ─────────────────────────────────────────────────────────────────────

async fn push_handler(
    State(state): State<Arc<LogsState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let tenant = extract_tenant(&headers);
    let content_type = headers
        .get("Content-Type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json");

    if content_type.contains("application/x-protobuf") {
        match ingest_proto(&state.store, body, tenant) {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(e) => {
                warn!(error = %e, "proto push failed");
                (StatusCode::BAD_REQUEST, e).into_response()
            }
        }
    } else {
        match serde_json::from_slice::<PushRequest>(&body) {
            Ok(req) => {
                ingest_json(&state.store, req, tenant);
                StatusCode::NO_CONTENT.into_response()
            }
            Err(e) => {
                warn!(error = %e, "json push failed");
                (StatusCode::BAD_REQUEST, e.to_string()).into_response()
            }
        }
    }
}

// ─── Instant query ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct QueryParams {
    query: String,
    #[serde(default)]
    time: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    direction: Option<String>,
}

async fn query_handler(
    State(state): State<Arc<LogsState>>,
    headers: HeaderMap,
    Query(params): Query<QueryParams>,
) -> Response {
    let tenant = extract_tenant(&headers);
    let time = params.time.as_deref().and_then(parse_time).unwrap_or_else(Utc::now);
    let start = time - Duration::hours(1);
    let limit = params.limit.unwrap_or(state.default_limit);
    let forward = params.direction.as_deref() != Some("backward");

    match parse(&params.query) {
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
        Ok(expr) => {
            let eval = Evaluator::new(&state.store);
            let data = match &expr {
                crate::logql::ast::Expr::Log(ls) => {
                    let result = eval.eval_log(ls, start, time, limit, tenant.as_deref());
                    QueryData::Streams { result }
                }
                crate::logql::ast::Expr::Metric(m) => {
                    let result = eval.eval_metric(m, start, time, 60, tenant.as_deref());
                    QueryData::Matrix { result }
                }
            };
            Json(LokiResponse::success(data)).into_response()
        }
    }
}

// ─── Range query ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct QueryRangeParams {
    query: String,
    #[serde(default)]
    start: Option<String>,
    #[serde(default)]
    end: Option<String>,
    #[serde(default)]
    step: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    direction: Option<String>,
}

async fn query_range_handler(
    State(state): State<Arc<LogsState>>,
    headers: HeaderMap,
    Query(params): Query<QueryRangeParams>,
) -> Response {
    let tenant = extract_tenant(&headers);
    let now = Utc::now();
    let start = params.start.as_deref().and_then(parse_time).unwrap_or(now - Duration::hours(1));
    let end = params.end.as_deref().and_then(parse_time).unwrap_or(now);
    let step_secs = params
        .step
        .as_deref()
        .and_then(parse_step_secs)
        .unwrap_or(60);
    let limit = params.limit.unwrap_or(state.default_limit);

    match parse(&params.query) {
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
        Ok(expr) => {
            let eval = Evaluator::new(&state.store);
            let data = match &expr {
                crate::logql::ast::Expr::Log(ls) => {
                    let result = eval.eval_log(ls, start, end, limit, tenant.as_deref());
                    QueryData::Streams { result }
                }
                crate::logql::ast::Expr::Metric(m) => {
                    let result = eval.eval_metric(m, start, end, step_secs, tenant.as_deref());
                    QueryData::Matrix { result }
                }
            };
            Json(LokiResponse::success(data)).into_response()
        }
    }
}

// ─── Labels ───────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct TimeRangeParams {
    #[serde(default)]
    start: Option<String>,
    #[serde(default)]
    end: Option<String>,
    #[serde(default)]
    query: Option<String>,
}

async fn labels_handler(
    State(state): State<Arc<LogsState>>,
    headers: HeaderMap,
    Query(params): Query<TimeRangeParams>,
) -> Json<LokiResponse<Vec<String>>> {
    let tenant = extract_tenant(&headers);
    let (start, end) = time_range(&params);
    let names = state.store.label_names(start, end, tenant.as_deref());
    Json(LokiResponse::success(names))
}

async fn label_values_handler(
    State(state): State<Arc<LogsState>>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Query(params): Query<TimeRangeParams>,
) -> Json<LokiResponse<Vec<String>>> {
    let tenant = extract_tenant(&headers);
    let (start, end) = time_range(&params);
    let values = state.store.label_values(&name, start, end, tenant.as_deref());
    Json(LokiResponse::success(values))
}

// ─── Series ───────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct SeriesParams {
    #[serde(rename = "match[]", default)]
    matches: Vec<String>,
    #[serde(default)]
    start: Option<String>,
    #[serde(default)]
    end: Option<String>,
}

async fn series_handler(
    State(state): State<Arc<LogsState>>,
    headers: HeaderMap,
    Query(params): Query<SeriesParams>,
) -> Response {
    let tenant = extract_tenant(&headers);
    let now = Utc::now();
    let start = params.start.as_deref().and_then(parse_time).unwrap_or(now - Duration::hours(1));
    let end = params.end.as_deref().and_then(parse_time).unwrap_or(now);

    // Parse all matchers from `match[]` parameters
    let mut all_matchers = vec![];
    for m in &params.matches {
        match parse(m) {
            Ok(crate::logql::ast::Expr::Log(ls)) => all_matchers.extend(ls.matchers),
            Ok(_) => {}
            Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
        }
    }

    let series = state.store.series(&all_matchers, start, end, tenant.as_deref());
    let result: Vec<_> = series.into_iter().map(|l| l.0).collect();
    Json(LokiResponse::success(result)).into_response()
}

// ─── WebSocket tail ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct TailParams {
    query: String,
    #[serde(default)]
    limit: Option<usize>,
}

async fn tail_handler(
    State(state): State<Arc<LogsState>>,
    headers: HeaderMap,
    Query(params): Query<TailParams>,
    ws: WebSocketUpgrade,
) -> Response {
    let tenant = extract_tenant(&headers);
    let limit = params.limit.unwrap_or(state.default_limit);
    let query = params.query.clone();
    ws.on_upgrade(move |socket| handle_tail(socket, Arc::clone(&state.store), query, limit, tenant))
}

// ─── Time parsing helpers ─────────────────────────────────────────────────────

/// Parse a time value — Unix epoch seconds (float), nanoseconds, or RFC3339.
fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    // Unix nanoseconds (> 1e15)
    if let Ok(ns) = s.parse::<i64>() {
        if ns > 1_000_000_000_000_000i64 {
            let secs = ns / 1_000_000_000;
            let nanos = (ns % 1_000_000_000) as u32;
            return DateTime::from_timestamp(secs, nanos);
        }
        return DateTime::from_timestamp(ns, 0);
    }
    // Unix float seconds
    if let Ok(f) = s.parse::<f64>() {
        return DateTime::from_timestamp(f as i64, ((f.fract()) * 1e9) as u32);
    }
    // RFC3339
    s.parse::<DateTime<Utc>>().ok()
}

fn parse_step_secs(s: &str) -> Option<i64> {
    // Accept "30s", "1m", "1h", plain integer seconds
    if let Ok(n) = s.parse::<i64>() {
        return Some(n);
    }
    let (num, suffix) = s.split_at(s.len().saturating_sub(1));
    let n: i64 = num.parse().ok()?;
    match suffix {
        "s" => Some(n),
        "m" => Some(n * 60),
        "h" => Some(n * 3600),
        "d" => Some(n * 86400),
        _ => None,
    }
}

fn time_range(params: &TimeRangeParams) -> (DateTime<Utc>, DateTime<Utc>) {
    let now = Utc::now();
    let start = params.start.as_deref().and_then(parse_time).unwrap_or(now - Duration::hours(1));
    let end = params.end.as_deref().and_then(parse_time).unwrap_or(now);
    (start, end)
}
