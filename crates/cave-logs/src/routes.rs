//! HTTP routes for cave-logs.

use crate::alerting::{evaluate_all_alerts, AlertFiring};
use crate::ingestion::{ingest_batch, ingest_log, IngestRequest};
use crate::models::{
    AlertCondition, AlertSeverity, DashboardPanel, LogAlert, LogDashboard, LogEntry, LogPipeline,
    LogQuery, LogQueryOp, LogStream, ParseRule, RetentionPolicy,
};
use crate::query::{execute_query, QueryResult};
use crate::LogsState;
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

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-logs",
        "status": "ok",
        "upstream": "ELK Stack / Grafana Loki"
    }))
}
