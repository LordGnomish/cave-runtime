//! HTTP routes for cave-logs.
//!
//! Exposes two route groups:
//!   /api/logs/*          — cave-native management API
//!   /loki/api/v1/*       — Loki-compatible API (drop-in replacement)

use crate::{models::*, LogsState};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;

pub fn create_router(state: Arc<LogsState>) -> Router {
    Router::new()
        // ── cave-native ────────────────────────────────────────────────────
        .route("/api/logs/health", get(health))
        // ── Loki push API ─────────────────────────────────────────────────
        // POST /loki/api/v1/push — ingest log streams
        .route("/loki/api/v1/push", post(push))
        // ── Loki query API ────────────────────────────────────────────────
        // Instant query: GET /loki/api/v1/query?query=<logql>&limit=<n>
        .route("/loki/api/v1/query", get(instant_query))
        // Range query:   GET /loki/api/v1/query_range?query=<logql>&start=...&end=...
        .route("/loki/api/v1/query_range", get(range_query))
        // ── Loki label API ────────────────────────────────────────────────
        // Label names:   GET /loki/api/v1/labels
        .route("/loki/api/v1/labels", get(labels))
        // Label values:  GET /loki/api/v1/label/:name/values
        .route("/loki/api/v1/label/:name/values", get(label_values))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// cave-native
// ---------------------------------------------------------------------------

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-logs",
        "status": "ok",
        "upstream": "loki",
        "upstream_tracked_version": "3.x",
        "compat": ["loki_push_v1", "logql_api_v1"]
    }))
}

// ---------------------------------------------------------------------------
// Loki push — POST /loki/api/v1/push
// ---------------------------------------------------------------------------

/// Accept a Loki-format push payload and ingest log streams.
///
/// Clients send either JSON (Content-Type: application/json) or
/// snappy-compressed protobuf (Content-Type: application/x-protobuf).
/// We accept both; JSON is the common path for structured logging.
async fn push(
    State(_state): State<Arc<LogsState>>,
    Json(req): Json<PushRequest>,
) -> StatusCode {
    let total_lines: usize = req.streams.iter().map(|s| s.values.len()).sum();
    tracing::debug!(
        streams = req.streams.len(),
        lines   = total_lines,
        "loki push received"
    );
    // TODO: persist to log store (PostgreSQL / object storage)
    StatusCode::NO_CONTENT
}

// ---------------------------------------------------------------------------
// Loki instant query — GET /loki/api/v1/query
// ---------------------------------------------------------------------------

async fn instant_query(
    State(_state): State<Arc<LogsState>>,
    Query(params): Query<InstantQueryParams>,
) -> Json<serde_json::Value> {
    tracing::debug!(query = %params.query, "loki instant_query");
    // TODO: evaluate LogQL against log store
    Json(serde_json::json!({
        "status": "success",
        "data": {
            "resultType": "streams",
            "result": [],
            "stats": {
                "summary": {
                    "bytesProcessedPerSecond": 0,
                    "linesProcessedPerSecond": 0,
                    "totalBytesProcessed": 0,
                    "totalLinesProcessed": 0,
                    "execTime": 0.0
                }
            }
        }
    }))
}

// ---------------------------------------------------------------------------
// Loki range query — GET /loki/api/v1/query_range
// ---------------------------------------------------------------------------

async fn range_query(
    State(_state): State<Arc<LogsState>>,
    Query(params): Query<RangeQueryParams>,
) -> Json<serde_json::Value> {
    tracing::debug!(
        query = %params.query,
        start = ?params.start,
        end   = ?params.end,
        "loki range_query"
    );
    // TODO: evaluate LogQL range query against log store
    Json(serde_json::json!({
        "status": "success",
        "data": {
            "resultType": "streams",
            "result": [],
            "stats": {
                "summary": {
                    "bytesProcessedPerSecond": 0,
                    "linesProcessedPerSecond": 0,
                    "totalBytesProcessed": 0,
                    "totalLinesProcessed": 0,
                    "execTime": 0.0
                }
            }
        }
    }))
}

// ---------------------------------------------------------------------------
// Loki labels — GET /loki/api/v1/labels
// ---------------------------------------------------------------------------

async fn labels(
    State(_state): State<Arc<LogsState>>,
    Query(_params): Query<LabelParams>,
) -> Json<serde_json::Value> {
    // TODO: enumerate distinct label names from log store
    let data: Vec<String> = vec!["app".to_string(), "level".to_string(), "namespace".to_string()];
    Json(serde_json::json!({
        "status": "success",
        "data": data
    }))
}

// ---------------------------------------------------------------------------
// Loki label values — GET /loki/api/v1/label/:name/values
// ---------------------------------------------------------------------------

async fn label_values(
    State(_state): State<Arc<LogsState>>,
    Path(name): Path<String>,
    Query(_params): Query<LabelParams>,
) -> Json<serde_json::Value> {
    tracing::debug!(label = %name, "loki label_values");
    // TODO: enumerate distinct values for this label from log store
    let data: Vec<String> = vec![];
    Json(serde_json::json!({
        "status": "success",
        "data": data
    }))
}
