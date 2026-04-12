//! HTTP routes for cave-logs.

use crate::alerting::{evaluate_all_alerts, AlertFiring};
use crate::ingestion::{ingest_batch, ingest_log, IngestRequest};
use crate::models::{
    AlertCondition, AlertSeverity, DashboardPanel, LogAlert, LogDashboard, LogEntry, LogPipeline,
    LogQuery, LogQueryOp, LogStream, ParseRule, RetentionPolicy,
};
use crate::query::{execute_query, QueryResult};
use crate::LogsState;
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
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router(state: Arc<LogsState>) -> Router {
    Router::new()
        // Ingestion
        .route("/api/v1/logs/ingest", post(ingest_single))
        .route("/api/v1/logs/ingest/batch", post(ingest_batch_handler))
        // Query & tail
        .route("/api/v1/logs/query", get(query_logs))
        .route("/api/v1/logs/tail", get(tail_logs))
        // Streams CRUD
        .route("/api/v1/logs/streams", get(list_streams).post(create_stream))
        .route(
            "/api/v1/logs/streams/{id}",
            get(get_stream).put(update_stream).delete(delete_stream),
        )
        // Alerts — static "evaluate" must be registered before the dynamic {id} route
        .route("/api/v1/logs/alerts/evaluate", get(evaluate_alerts_handler))
        .route("/api/v1/logs/alerts", get(list_alerts).post(create_alert))
        .route(
            "/api/v1/logs/alerts/{id}",
            get(get_alert).put(update_alert).delete(delete_alert),
        )
        // Pipelines CRUD
        .route(
            "/api/v1/logs/pipelines",
            get(list_pipelines).post(create_pipeline),
        )
        .route(
            "/api/v1/logs/pipelines/{id}",
            get(get_pipeline).put(update_pipeline).delete(delete_pipeline),
        )
        // Dashboards CRUD
        .route(
            "/api/v1/logs/dashboards",
            get(list_dashboards).post(create_dashboard),
        )
        .route(
            "/api/v1/logs/dashboards/{id}",
            get(get_dashboard).delete(delete_dashboard),
        )
        // Health
        .route("/api/v1/logs/health", get(health))
        .with_state(state)
}

// ── Ingestion ─────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct IngestBody {
    raw: String,
    service: String,
    stream_id: Option<Uuid>,
    labels: Option<HashMap<String, String>>,
    pipeline_id: Option<Uuid>,
}

#[derive(Deserialize)]
struct BatchIngestBody {
    entries: Vec<IngestBody>,
}

#[derive(Serialize)]
struct BatchIngestResponse {
    ingested: usize,
}

async fn ingest_single(
    State(state): State<Arc<LogsState>>,
    Json(body): Json<IngestBody>,
) -> Json<LogEntry> {
    let entry = ingest_log(
        &state,
        IngestRequest {
            raw: body.raw,
            service: body.service,
            stream_id: body.stream_id,
            labels: body.labels.unwrap_or_default(),
            pipeline_id: body.pipeline_id,
        },
    );
    Json(entry)
}

async fn ingest_batch_handler(
    State(state): State<Arc<LogsState>>,
    Json(body): Json<BatchIngestBody>,
) -> Json<BatchIngestResponse> {
    let reqs: Vec<IngestRequest> = body
        .entries
        .into_iter()
        .map(|b| IngestRequest {
            raw: b.raw,
            service: b.service,
            stream_id: b.stream_id,
            labels: b.labels.unwrap_or_default(),
            pipeline_id: b.pipeline_id,
        })
        .collect();
    let ingested = reqs.len();
    ingest_batch(&state, reqs);
    Json(BatchIngestResponse { ingested })
}

// ── Query ─────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct QueryParams {
    expr: Option<String>,
    stream_id: Option<Uuid>,
    level: Option<String>,
    service: Option<String>,
    /// RFC 3339 timestamp string.
    start: Option<String>,
    /// RFC 3339 timestamp string.
    end: Option<String>,
    /// Regex filter on message.
    regex: Option<String>,
    /// Full-text substring search.
    q: Option<String>,
    /// Operation: filter | count_over_time | rate | top_k | search
    op: Option<String>,
    limit: Option<usize>,
    /// Bucket step in seconds (for count_over_time / rate).
    step: Option<u64>,
    top_k: Option<usize>,
}

async fn query_logs(
    State(state): State<Arc<LogsState>>,
    Query(params): Query<QueryParams>,
) -> Json<QueryResult> {
    let op = match params.op.as_deref().unwrap_or("filter") {
        "count_over_time" => LogQueryOp::CountOverTime,
        "rate" => LogQueryOp::Rate,
        "top_k" => LogQueryOp::TopK,
        "search" => LogQueryOp::FullTextSearch,
        _ => LogQueryOp::Filter,
    };

    let query = LogQuery {
        expr: params.expr.unwrap_or_default(),
        stream_id: params.stream_id,
        level: params.level,
        service: params.service,
        start: params
            .start
            .as_deref()
            .and_then(|s| s.parse().ok()),
        end: params
            .end
            .as_deref()
            .and_then(|s| s.parse().ok()),
        regex_filter: params.regex,
        full_text: params.q,
        operation: op,
        limit: params.limit,
        step_seconds: params.step,
        top_k: params.top_k,
    };

    Json(execute_query(&state, &query))
}

// ── Live Tail ─────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct TailParams {
    service: Option<String>,
    level: Option<String>,
    limit: Option<usize>,
}

#[derive(Serialize)]
struct TailResponse {
    entries: Vec<LogEntry>,
    returned: usize,
}

async fn tail_logs(
    State(state): State<Arc<LogsState>>,
    Query(params): Query<TailParams>,
) -> Json<TailResponse> {
    let limit = params.limit.unwrap_or(100).min(1_000);
    let entries = state.entries.lock().unwrap();
    let tail: Vec<LogEntry> = entries
        .iter()
        .rev()
        .filter(|e| {
            if let Some(svc) = &params.service {
                if !e.service.to_lowercase().contains(&svc.to_lowercase()) {
                    return false;
                }
            }
            if let Some(level) = &params.level {
                let entry_level = format!("{:?}", e.level).to_lowercase();
                if entry_level != level.to_lowercase() {
                    return false;
                }
            }
            true
        })
        .take(limit)
        .cloned()
        .collect();
    let returned = tail.len();
    Json(TailResponse { entries: tail, returned })
}

// ── Streams ───────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CreateStreamBody {
    name: String,
    description: Option<String>,
    labels_schema: Option<Vec<String>>,
    retention: Option<RetentionPolicy>,
}

async fn list_streams(State(state): State<Arc<LogsState>>) -> Json<Vec<LogStream>> {
    let streams = state.streams.lock().unwrap();
    Json(streams.values().cloned().collect())
}

async fn create_stream(
    State(state): State<Arc<LogsState>>,
    Json(body): Json<CreateStreamBody>,
) -> (StatusCode, Json<LogStream>) {
    let stream = LogStream {
        id: Uuid::new_v4(),
        name: body.name,
        description: body.description.unwrap_or_default(),
        labels_schema: body.labels_schema.unwrap_or_default(),
        retention: body.retention.unwrap_or_default(),
        created_at: Utc::now(),
    };
    state.streams.lock().unwrap().insert(stream.id, stream.clone());
    (StatusCode::CREATED, Json(stream))
}

async fn get_stream(
    State(state): State<Arc<LogsState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<LogStream>, StatusCode> {
    state
        .streams
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn update_stream(
    State(state): State<Arc<LogsState>>,
    Path(id): Path<Uuid>,
    Json(body): Json<CreateStreamBody>,
) -> Result<Json<LogStream>, StatusCode> {
    let mut streams = state.streams.lock().unwrap();
    let stream = streams.get_mut(&id).ok_or(StatusCode::NOT_FOUND)?;
    stream.name = body.name;
    if let Some(desc) = body.description {
        stream.description = desc;
    }
    if let Some(schema) = body.labels_schema {
        stream.labels_schema = schema;
    }
    if let Some(ret) = body.retention {
        stream.retention = ret;
    }
    Ok(Json(stream.clone()))
}

async fn delete_stream(
    State(state): State<Arc<LogsState>>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    if state.streams.lock().unwrap().remove(&id).is_some() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

// ── Alerts ────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CreateAlertBody {
    name: String,
    description: Option<String>,
    query: LogQuery,
    condition: AlertCondition,
    threshold: f64,
    window_seconds: u64,
    severity: AlertSeverity,
}

async fn list_alerts(State(state): State<Arc<LogsState>>) -> Json<Vec<LogAlert>> {
    let alerts = state.alerts.lock().unwrap();
    Json(alerts.values().cloned().collect())
}

async fn create_alert(
    State(state): State<Arc<LogsState>>,
    Json(body): Json<CreateAlertBody>,
) -> (StatusCode, Json<LogAlert>) {
    let alert = LogAlert {
        id: Uuid::new_v4(),
        name: body.name,
        description: body.description.unwrap_or_default(),
        query: body.query,
        condition: body.condition,
        threshold: body.threshold,
        window_seconds: body.window_seconds,
        severity: body.severity,
        enabled: true,
        created_at: Utc::now(),
        last_triggered: None,
    };
    state.alerts.lock().unwrap().insert(alert.id, alert.clone());
    (StatusCode::CREATED, Json(alert))
}

async fn get_alert(
    State(state): State<Arc<LogsState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<LogAlert>, StatusCode> {
    state
        .alerts
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn update_alert(
    State(state): State<Arc<LogsState>>,
    Path(id): Path<Uuid>,
    Json(body): Json<CreateAlertBody>,
) -> Result<Json<LogAlert>, StatusCode> {
    let mut alerts = state.alerts.lock().unwrap();
    let alert = alerts.get_mut(&id).ok_or(StatusCode::NOT_FOUND)?;
    alert.name = body.name;
    if let Some(desc) = body.description {
        alert.description = desc;
    }
    alert.query = body.query;
    alert.condition = body.condition;
    alert.threshold = body.threshold;
    alert.window_seconds = body.window_seconds;
    alert.severity = body.severity;
    Ok(Json(alert.clone()))
}

async fn delete_alert(
    State(state): State<Arc<LogsState>>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    if state.alerts.lock().unwrap().remove(&id).is_some() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

async fn evaluate_alerts_handler(
    State(state): State<Arc<LogsState>>,
) -> Json<Vec<AlertFiring>> {
    Json(evaluate_all_alerts(&state))
}

// ── Pipelines ─────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CreatePipelineBody {
    name: String,
    description: Option<String>,
    parse_rules: Option<Vec<ParseRule>>,
    label_extractors: Option<Vec<String>>,
    drop_labels: Option<Vec<String>>,
    filters: Option<Vec<String>>,
}

async fn list_pipelines(State(state): State<Arc<LogsState>>) -> Json<Vec<LogPipeline>> {
    let pipelines = state.pipelines.lock().unwrap();
    Json(pipelines.values().cloned().collect())
}

async fn create_pipeline(
    State(state): State<Arc<LogsState>>,
    Json(body): Json<CreatePipelineBody>,
) -> (StatusCode, Json<LogPipeline>) {
    let pipeline = LogPipeline {
        id: Uuid::new_v4(),
        name: body.name,
        description: body.description.unwrap_or_default(),
        parse_rules: body.parse_rules.unwrap_or_default(),
        label_extractors: body.label_extractors.unwrap_or_default(),
        drop_labels: body.drop_labels.unwrap_or_default(),
        filters: body.filters.unwrap_or_default(),
        enabled: true,
        created_at: Utc::now(),
    };
    state
        .pipelines
        .lock()
        .unwrap()
        .insert(pipeline.id, pipeline.clone());
    (StatusCode::CREATED, Json(pipeline))
}

async fn get_pipeline(
    State(state): State<Arc<LogsState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<LogPipeline>, StatusCode> {
    state
        .pipelines
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn update_pipeline(
    State(state): State<Arc<LogsState>>,
    Path(id): Path<Uuid>,
    Json(body): Json<CreatePipelineBody>,
) -> Result<Json<LogPipeline>, StatusCode> {
    let mut pipelines = state.pipelines.lock().unwrap();
    let pipeline = pipelines.get_mut(&id).ok_or(StatusCode::NOT_FOUND)?;
    pipeline.name = body.name;
    if let Some(desc) = body.description {
        pipeline.description = desc;
    }
    if let Some(rules) = body.parse_rules {
        pipeline.parse_rules = rules;
    }
    if let Some(ext) = body.label_extractors {
        pipeline.label_extractors = ext;
    }
    if let Some(drop) = body.drop_labels {
        pipeline.drop_labels = drop;
    }
    if let Some(filters) = body.filters {
        pipeline.filters = filters;
    }
    Ok(Json(pipeline.clone()))
}

async fn delete_pipeline(
    State(state): State<Arc<LogsState>>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    if state.pipelines.lock().unwrap().remove(&id).is_some() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

// ── Dashboards ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CreateDashboardBody {
    name: String,
    description: Option<String>,
    panels: Option<Vec<DashboardPanel>>,
}

async fn list_dashboards(State(state): State<Arc<LogsState>>) -> Json<Vec<LogDashboard>> {
    let dashboards = state.dashboards.lock().unwrap();
    Json(dashboards.values().cloned().collect())
}

async fn create_dashboard(
    State(state): State<Arc<LogsState>>,
    Json(body): Json<CreateDashboardBody>,
) -> (StatusCode, Json<LogDashboard>) {
    let dashboard = LogDashboard {
        id: Uuid::new_v4(),
        name: body.name,
        description: body.description.unwrap_or_default(),
        panels: body.panels.unwrap_or_default(),
        created_at: Utc::now(),
    };
    state
        .dashboards
        .lock()
        .unwrap()
        .insert(dashboard.id, dashboard.clone());
    (StatusCode::CREATED, Json(dashboard))
}

async fn get_dashboard(
    State(state): State<Arc<LogsState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<LogDashboard>, StatusCode> {
    state
        .dashboards
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn delete_dashboard(
    State(state): State<Arc<LogsState>>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    if state.dashboards.lock().unwrap().remove(&id).is_some() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

// ── Health ─────────────────────────────────────────────────────────────────────
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
        "upstream": "ELK Stack / Grafana Loki"
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
        "upstream": "ELK Stack / Grafana Loki"
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
        "upstream": "ELK Stack / Grafana Loki"
    }))
}
