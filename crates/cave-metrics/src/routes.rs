//! HTTP routes for cave-metrics.
//!
//! Exposes two route groups:
//!   /api/metrics/*  — cave-native management API
//!   /api/v1/*       — Prometheus-compatible HTTP API (drop-in replacement)
//!   /metrics        — Prometheus exposition format (self-metrics)

use crate::{
    models::*,
    MetricsState,
};
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use std::{collections::HashMap, sync::Arc};

pub fn create_router(state: Arc<MetricsState>) -> Router {
    Router::new()
        // ── cave-native ────────────────────────────────────────────────────
        .route("/api/metrics/health", get(health))
        // ── Prometheus remote_write ────────────────────────────────────────
        // POST /api/v1/write — accepts snappy-compressed protobuf body
        // (Prometheus remote_write protocol v1 / v2)
        .route("/api/v1/write", post(remote_write))
        // ── Prometheus query API ───────────────────────────────────────────
        // Instant query:  GET /api/v1/query?query=<promql>&time=<ts>
        .route("/api/v1/query", get(instant_query))
        // Range query:    GET /api/v1/query_range?query=<promql>&start=<ts>&end=<ts>&step=<dur>
        .route("/api/v1/query_range", get(range_query))
        // ── Prometheus metadata API ────────────────────────────────────────
        // Series:         GET /api/v1/series?match[]=<selector>
        .route("/api/v1/series", get(series))
        // Label names:    GET /api/v1/labels
        .route("/api/v1/labels", get(labels))
        // Label values:   GET /api/v1/label/:name/values
        .route("/api/v1/label/:name/values", get(label_values))
        // ── Self-metrics (Prometheus exposition format) ────────────────────
        .route("/metrics", get(self_metrics))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// cave-native endpoints
// ---------------------------------------------------------------------------

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-metrics",
        "status": "ok",
        "upstream": "prometheus",
        "upstream_tracked_version": "2.x",
        "compat": ["remote_write_v1", "http_api_v1"]
    }))
}

// ---------------------------------------------------------------------------
// Prometheus remote_write — POST /api/v1/write
// ---------------------------------------------------------------------------

/// Accept Prometheus remote_write payload (snappy-encoded protobuf).
///
/// Real implementation will decode the WriteRequest protobuf and persist
/// samples to the TSDB. For now we accept the bytes and return 204, which
/// is the correct response code per the remote_write spec.
async fn remote_write(
    State(_state): State<Arc<MetricsState>>,
    body: Bytes,
) -> StatusCode {
    // TODO: decode snappy + protobuf WriteRequest, ingest into TSDB
    tracing::debug!(bytes = body.len(), "remote_write received");
    StatusCode::NO_CONTENT
}

// ---------------------------------------------------------------------------
// Prometheus instant query — GET /api/v1/query
// ---------------------------------------------------------------------------

async fn instant_query(
    State(_state): State<Arc<MetricsState>>,
    Query(params): Query<InstantQueryParams>,
) -> Json<serde_json::Value> {
    tracing::debug!(query = %params.query, "instant_query");
    // TODO: evaluate PromQL against TSDB
    Json(serde_json::json!({
        "status": "success",
        "data": {
            "resultType": "vector",
            "result": []
        }
    }))
}

// ---------------------------------------------------------------------------
// Prometheus range query — GET /api/v1/query_range
// ---------------------------------------------------------------------------

async fn range_query(
    State(_state): State<Arc<MetricsState>>,
    Query(params): Query<RangeQueryParams>,
) -> Json<serde_json::Value> {
    tracing::debug!(
        query = %params.query,
        start = %params.start,
        end   = %params.end,
        step  = %params.step,
        "range_query"
    );
    // TODO: evaluate PromQL range query against TSDB
    Json(serde_json::json!({
        "status": "success",
        "data": {
            "resultType": "matrix",
            "result": []
        }
    }))
}

// ---------------------------------------------------------------------------
// Series metadata — GET /api/v1/series
// ---------------------------------------------------------------------------

async fn series(
    State(_state): State<Arc<MetricsState>>,
    Query(_params): Query<SeriesParams>,
) -> Json<serde_json::Value> {
    // TODO: query TSDB for matching series
    let result: Vec<HashMap<String, String>> = vec![];
    Json(serde_json::json!({
        "status": "success",
        "data": result
    }))
}

// ---------------------------------------------------------------------------
// Label names — GET /api/v1/labels
// ---------------------------------------------------------------------------

async fn labels(
    State(_state): State<Arc<MetricsState>>,
    Query(_params): Query<LabelsParams>,
) -> Json<serde_json::Value> {
    // TODO: enumerate all label names from TSDB
    let data: Vec<String> = vec!["__name__".to_string(), "job".to_string(), "instance".to_string()];
    Json(serde_json::json!({
        "status": "success",
        "data": data
    }))
}

// ---------------------------------------------------------------------------
// Label values — GET /api/v1/label/:name/values
// ---------------------------------------------------------------------------

async fn label_values(
    State(_state): State<Arc<MetricsState>>,
    Path(name): Path<String>,
    Query(_params): Query<LabelValuesParams>,
) -> Json<serde_json::Value> {
    tracing::debug!(label = %name, "label_values");
    // TODO: query TSDB for distinct values of this label
    let data: Vec<String> = vec![];
    Json(serde_json::json!({
        "status": "success",
        "data": data
    }))
}

// ---------------------------------------------------------------------------
// Self-metrics — GET /metrics  (Prometheus exposition format)
// ---------------------------------------------------------------------------

/// Return cave-metrics own operational metrics in Prometheus text format.
/// Content-Type must be text/plain; version=0.0.4 per the exposition spec.
async fn self_metrics(State(_state): State<Arc<MetricsState>>) -> (StatusCode, [(& 'static str, &'static str); 1], String) {
    // TODO: expose real counters/gauges via prometheus-client crate
    let body = "# HELP cave_metrics_requests_total Total requests processed\n\
                # TYPE cave_metrics_requests_total counter\n\
                cave_metrics_requests_total{handler=\"remote_write\"} 0\n\
                cave_metrics_requests_total{handler=\"query\"} 0\n\
                # HELP cave_metrics_series_total Total time series stored\n\
                # TYPE cave_metrics_series_total gauge\n\
                cave_metrics_series_total 0\n"
        .to_string();
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
}
