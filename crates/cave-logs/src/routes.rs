// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Full Loki-compatible HTTP API.
//!
//! Endpoints:
//!   POST /loki/api/v1/push                     — JSON + protobuf+snappy ingestion
//!   POST /otlp/v1/logs                         — OTLP HTTP/JSON ingestion
//!   GET  /loki/api/v1/query                    — instant query
//!   GET  /loki/api/v1/query_range              — range query
//!   GET  /loki/api/v1/labels                   — label names
//!   GET  /loki/api/v1/label/{name}/values      — label values
//!   GET  /loki/api/v1/series                   — series matching
//!   GET  /loki/api/v1/index/stats              — index statistics
//!   GET  /loki/api/v1/tail                     — WebSocket live tail
//!   GET  /ready                                — readiness probe
//!   GET  /metrics                              — Prometheus metrics

use axum::{
    Router,
    extract::{Path, Query, State, WebSocketUpgrade},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
};
use chrono::Utc;
use serde_json::json;
use std::sync::Arc;
use tracing::{info, warn};

use crate::ingest::loki_push::{ingest_json, ingest_protobuf};
use crate::ingest::otlp::ingest_otlp_json;
use crate::limits::{LimitError, LimitsRegistry};
use crate::logql::ast::Query as LogQuery;
use crate::logql::eval::Evaluator;
use crate::logql::parser::Parser;
use crate::models::{
    Direction, IndexStats, LabelParams, QueryData, QueryParams, QueryRangeParams, QueryResponse,
    SeriesParams, TailParams,
};
use crate::store::LogStore;
use crate::tail::handle_tail;

// ── Shared state ──────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<LogStore>,
    pub limits: Arc<LimitsRegistry>,
}

// ── Tenant extraction ─────────────────────────────────────────────────────────

fn tenant_from_headers(headers: &HeaderMap) -> String {
    headers
        .get("x-scope-orgid")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty() && *s != "fake")
        .unwrap_or("anonymous")
        .to_owned()
}

// ── Time parsing helpers ──────────────────────────────────────────────────────

/// Parse a Loki time parameter (RFC3339, Unix seconds float, or Unix nanoseconds string).
fn parse_time_param(s: &str) -> Option<i64> {
    // Unix nanoseconds (large integer string)
    if s.len() > 13 {
        if let Ok(ns) = s.parse::<i64>() {
            return Some(ns);
        }
    }
    // Unix seconds (possibly with fractional part)
    if let Ok(f) = s.parse::<f64>() {
        return Some((f * 1e9) as i64);
    }
    // RFC 3339
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return dt.timestamp_nanos_opt();
    }
    None
}

fn step_to_ns(step: &str) -> i64 {
    // step can be a duration string or a plain number of seconds.
    if let Ok(secs) = step.parse::<f64>() {
        return (secs * 1e9) as i64;
    }
    // duration suffix
    let (n_str, suffix) =
        step.split_at(step.find(|c: char| c.is_alphabetic()).unwrap_or(step.len()));
    let base: f64 = n_str.parse().unwrap_or(1.0);
    let ns = match suffix {
        "ns" => base as i64,
        "us" => (base * 1_000.0) as i64,
        "ms" => (base * 1_000_000.0) as i64,
        "s" => (base * 1_000_000_000.0) as i64,
        "m" => (base * 60_000_000_000.0) as i64,
        "h" => (base * 3_600_000_000_000.0) as i64,
        "d" => (base * 86_400_000_000_000.0) as i64,
        _ => 1_000_000_000, // default 1s
    };
    ns
}

// ── Error responses ───────────────────────────────────────────────────────────

fn loki_error(status: StatusCode, msg: impl std::fmt::Display) -> Response {
    (
        status,
        Json(json!({ "status": "error", "error": msg.to_string() })),
    )
        .into_response()
}

fn limit_error_response(e: LimitError) -> Response {
    let status = match e.http_status() {
        429 => StatusCode::TOO_MANY_REQUESTS,
        _ => StatusCode::BAD_REQUEST,
    };
    loki_error(status, e)
}

// ── Push endpoint ─────────────────────────────────────────────────────────────

async fn push_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let tenant = tenant_from_headers(&headers);
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json");

    // Rate limit check.
    if let Err(e) = state.limits.check_ingestion_rate(&tenant, body.len()) {
        return limit_error_response(e);
    }

    let result = if content_type.contains("x-protobuf") || content_type.contains("protobuf") {
        ingest_protobuf(&body, &tenant, &state.store)
    } else {
        ingest_json(&body, &tenant, &state.store)
    };

    match result {
        Ok(n) => {
            info!("ingested {} entries for tenant {}", n, tenant);
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => loki_error(StatusCode::BAD_REQUEST, e),
    }
}

// ── OTLP push ─────────────────────────────────────────────────────────────────

async fn otlp_push_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let tenant = tenant_from_headers(&headers);
    if let Err(e) = state.limits.check_ingestion_rate(&tenant, body.len()) {
        return limit_error_response(e);
    }
    match ingest_otlp_json(&body, &tenant, &state.store) {
        Ok(n) => {
            info!("OTLP: ingested {} entries for tenant {}", n, tenant);
            StatusCode::OK.into_response()
        }
        Err(e) => loki_error(StatusCode::BAD_REQUEST, e),
    }
}

// ── Instant query ─────────────────────────────────────────────────────────────

async fn query_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<QueryParams>,
) -> Response {
    let tenant = tenant_from_headers(&headers);
    let now_ns = Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let time_ns = params
        .time
        .as_deref()
        .and_then(parse_time_param)
        .unwrap_or(now_ns);

    // Instant query: [time - 1h, time]
    let start_ns = time_ns - 3_600_000_000_000i64;
    let end_ns = time_ns;

    let limit = match state
        .limits
        .check_query_limits(&tenant, params.limit, start_ns, end_ns)
    {
        Ok(l) => l,
        Err(e) => return limit_error_response(e),
    };

    let query_str = &params.query;
    let parsed = match Parser::parse_query(query_str) {
        Ok(q) => q,
        Err(e) => return loki_error(StatusCode::BAD_REQUEST, format!("parse error: {}", e)),
    };

    let eval = Evaluator::new(state.store);
    let data = match parsed {
        LogQuery::Log(lq) => {
            eval.eval_log_query(&tenant, &lq, start_ns, end_ns, limit, params.direction)
        }
        LogQuery::Metric(mq) => {
            eval.eval_metric_query(&tenant, &mq, start_ns, end_ns, 60_000_000_000)
        }
    };

    Json(QueryResponse {
        status: "success",
        data,
    })
    .into_response()
}

// ── Range query ───────────────────────────────────────────────────────────────

async fn query_range_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<QueryRangeParams>,
) -> Response {
    let tenant = tenant_from_headers(&headers);
    let now_ns = Utc::now().timestamp_nanos_opt().unwrap_or(0);

    let start_ns = params
        .start
        .as_deref()
        .and_then(parse_time_param)
        .unwrap_or(now_ns - 3_600_000_000_000);
    let end_ns = params
        .end
        .as_deref()
        .and_then(parse_time_param)
        .unwrap_or(now_ns);
    let step_ns = params
        .step
        .as_deref()
        .map(step_to_ns)
        .unwrap_or(1_000_000_000);

    let limit = match state
        .limits
        .check_query_limits(&tenant, params.limit, start_ns, end_ns)
    {
        Ok(l) => l,
        Err(e) => return limit_error_response(e),
    };

    let parsed = match Parser::parse_query(&params.query) {
        Ok(q) => q,
        Err(e) => return loki_error(StatusCode::BAD_REQUEST, format!("parse error: {}", e)),
    };

    let eval = Evaluator::new(state.store);
    let data = match parsed {
        LogQuery::Log(lq) => {
            eval.eval_log_query(&tenant, &lq, start_ns, end_ns, limit, params.direction)
        }
        LogQuery::Metric(mq) => eval.eval_metric_query(&tenant, &mq, start_ns, end_ns, step_ns),
    };

    Json(QueryResponse {
        status: "success",
        data,
    })
    .into_response()
}

// ── Labels ────────────────────────────────────────────────────────────────────

async fn labels_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(_params): Query<LabelParams>,
) -> Response {
    let tenant = tenant_from_headers(&headers);
    let names = state.store.label_names(&tenant);
    Json(json!({ "status": "success", "data": names })).into_response()
}

async fn label_values_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Query(_params): Query<LabelParams>,
) -> Response {
    let tenant = tenant_from_headers(&headers);
    let values = state.store.label_values(&name, &tenant);
    Json(json!({ "status": "success", "data": values })).into_response()
}

// ── Series ────────────────────────────────────────────────────────────────────

async fn series_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<SeriesParams>,
) -> Response {
    let tenant = tenant_from_headers(&headers);

    // Collect fps for all requested matchers.
    let mut fps: Vec<u64> = Vec::new();
    let matchers = params.matchers.as_deref().unwrap_or(&[]);

    if matchers.is_empty() {
        fps = state.store.matching_fps(&tenant, |_| true);
    } else {
        for matcher_str in matchers {
            use crate::logql::ast::StreamSelector;
            use crate::logql::eval::labels_match;
            use crate::logql::lexer::Lexer;
            use crate::logql::parser::Parser;
            if let Ok(LogQuery::Log(lq)) = Parser::parse_query(matcher_str) {
                let mut matched = state
                    .store
                    .matching_fps(&tenant, |l| labels_match(l, &lq.selector));
                fps.append(&mut matched);
            }
        }
        fps.dedup();
    }

    let series = state.store.series(&tenant, &fps);
    let data: Vec<serde_json::Value> = series
        .iter()
        .map(|l| serde_json::to_value(&l.0).unwrap_or_default())
        .collect();

    Json(json!({ "status": "success", "data": data })).into_response()
}

// ── Index stats ───────────────────────────────────────────────────────────────

async fn index_stats_handler(State(state): State<AppState>) -> Response {
    let stats = state.store.stats();
    Json(json!({
        "status": "success",
        "data": {
            "streams": stats.streams,
            "chunks": stats.chunks,
            "entries": stats.entries,
            "bytes": stats.bytes,
        }
    }))
    .into_response()
}

// ── Tail (WebSocket) ──────────────────────────────────────────────────────────

async fn tail_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<TailParams>,
    ws: WebSocketUpgrade,
) -> Response {
    let tenant = tenant_from_headers(&headers);
    let query = params.query;
    let store = state.store;

    ws.on_upgrade(move |socket| handle_tail(socket, query, tenant, store))
}

// ── Ready / metrics ───────────────────────────────────────────────────────────

async fn ready_handler() -> &'static str {
    "ready"
}

async fn metrics_handler(State(state): State<AppState>) -> Response {
    let stats = state.store.stats();
    let text = format!(
        "# HELP cave_logs_streams_total Total number of log streams\n\
         # TYPE cave_logs_streams_total gauge\n\
         cave_logs_streams_total {}\n\
         # HELP cave_logs_entries_total Total number of log entries\n\
         # TYPE cave_logs_entries_total gauge\n\
         cave_logs_entries_total {}\n\
         # HELP cave_logs_chunks_total Total number of sealed chunks\n\
         # TYPE cave_logs_chunks_total gauge\n\
         cave_logs_chunks_total {}\n\
         # HELP cave_logs_bytes_total Total compressed storage bytes\n\
         # TYPE cave_logs_bytes_total gauge\n\
         cave_logs_bytes_total {}\n",
        stats.streams, stats.entries, stats.chunks, stats.bytes
    );
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4")],
        text,
    )
        .into_response()
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router(state: AppState) -> Router {
    Router::new()
        // Loki push
        .route("/loki/api/v1/push", post(push_handler))
        // OTLP push
        .route("/otlp/v1/logs", post(otlp_push_handler))
        // Loki query
        .route("/loki/api/v1/query", get(query_handler))
        .route("/loki/api/v1/query_range", get(query_range_handler))
        // Label discovery
        .route("/loki/api/v1/labels", get(labels_handler))
        .route(
            "/loki/api/v1/label/{name}/values",
            get(label_values_handler),
        )
        // Series
        .route("/loki/api/v1/series", get(series_handler))
        // Index stats
        .route("/loki/api/v1/index/stats", get(index_stats_handler))
        // Tail (WebSocket)
        .route("/loki/api/v1/tail", get(tail_handler))
        // Health / metrics
        .route("/api/logs/ready", get(ready_handler))
        .route("/api/logs/metrics", get(metrics_handler))
        .with_state(state)
}
