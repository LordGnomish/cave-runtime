<<<<<<< HEAD
<<<<<<< HEAD
<<<<<<< HEAD
=======
>>>>>>> claude/elastic-ellis
use crate::models::{IngestResponse, IngestSpanRequest, SearchResponse, Span, TraceQuery};
use crate::{analyzer, collector, TraceState};
use axum::{
    extract::{Path, Query, State as AxumState},
<<<<<<< HEAD
=======
use crate::comparison::TraceComparer;
use crate::dependency::DependencyComputer;
use crate::otlp::OtlpReceiver;
use crate::query::QueryEngine;
use crate::sampling::{Sampler, SamplingStrategy};
use crate::storage::TraceStore;
use crate::types::TraceQuery;
use axum::{
    extract::{Path, Query, State},
>>>>>>> claude/dazzling-tesla
=======
>>>>>>> claude/elastic-ellis
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
<<<<<<< HEAD
<<<<<<< HEAD
=======
>>>>>>> claude/elastic-ellis
use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router(state: Arc<TraceState>) -> Router {
    Router::new()
        // Static routes first so they take priority over /:id
        .route("/api/v1/traces/search", get(search_handler))
        .route("/api/v1/traces/services", get(services_handler))
        .route("/api/v1/traces/service-map", get(service_map_handler))
        .route("/api/v1/traces/latency", get(latency_handler))
        // Ingest
        .route("/api/v1/traces/ingest", post(ingest_handler))
        // Parameterized last
        .route("/api/v1/traces/:id", get(get_trace_handler))
        .with_state(state)
}

// ── POST /api/v1/traces/ingest ────────────────────────────────────────────────

async fn ingest_handler(
    AxumState(state): AxumState<Arc<TraceState>>,
    Json(req): Json<IngestSpanRequest>,
) -> Json<IngestResponse> {
    let trace_id = req.trace_id.unwrap_or_else(Uuid::new_v4);
    let span_id = req.span_id.unwrap_or_else(Uuid::new_v4);

    let span = Span {
        trace_id,
        span_id,
        parent_span_id: req.parent_span_id,
        operation: req.operation,
        service: req.service,
        start_time: req.start_time.unwrap_or_else(Utc::now),
        duration_ms: req.duration_ms,
        status: req.status.unwrap_or_default(),
        tags: req.tags.unwrap_or_default(),
        events: req.events.unwrap_or_default(),
        links: req.links.unwrap_or_default(),
    };

    let mut store = state.store.lock().await;
    collector::ingest_span(&mut store, span);

    Json(IngestResponse {
        trace_id,
        span_id,
        ingested: true,
    })
}

// ── GET /api/v1/traces/:id ────────────────────────────────────────────────────

async fn get_trace_handler(
    AxumState(state): AxumState<Arc<TraceState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.lock().await;
    match collector::build_trace(id, &store.spans) {
        Some(trace) => (StatusCode::OK, Json(serde_json::to_value(trace).unwrap())).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "trace not found", "trace_id": id })),
        )
            .into_response(),
    }
}

// ── GET /api/v1/traces/search ─────────────────────────────────────────────────

async fn search_handler(
    AxumState(state): AxumState<Arc<TraceState>>,
    Query(query): Query<TraceQuery>,
) -> Json<SearchResponse> {
    let store = state.store.lock().await;

    // Collect all distinct trace IDs that pass the filter.
    let matching_trace_ids: Vec<Uuid> = {
        let mut ids: Vec<Uuid> = store
            .spans
            .iter()
            .filter(|s| {
                if let Some(ref svc) = query.service {
                    if &s.service != svc {
                        return false;
                    }
                }
                if let Some(ref op) = query.operation {
                    if &s.operation != op {
                        return false;
                    }
                }
                if let Some(min) = query.min_duration_ms {
                    if s.duration_ms < min {
                        return false;
                    }
                }
                if let Some(max) = query.max_duration_ms {
                    if s.duration_ms > max {
                        return false;
                    }
                }
                if let Some(ref st) = query.status {
                    if &s.status != st {
                        return false;
                    }
                }
                true
            })
            .map(|s| s.trace_id)
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        ids.sort();
        ids
    };

    let limit = query.limit.unwrap_or(100);
    let traces: Vec<_> = matching_trace_ids
        .iter()
        .take(limit)
        .filter_map(|&tid| collector::build_trace(tid, &store.spans))
        .collect();

    let total = traces.len();
    Json(SearchResponse { traces, total })
}

// ── GET /api/v1/traces/services ───────────────────────────────────────────────

async fn services_handler(
    AxumState(state): AxumState<Arc<TraceState>>,
) -> Json<serde_json::Value> {
    let store = state.store.lock().await;
    let mut services: Vec<String> = store
        .spans
        .iter()
        .map(|s| s.service.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    services.sort();
    Json(serde_json::json!({ "services": services, "count": services.len() }))
}

// ── GET /api/v1/traces/service-map ───────────────────────────────────────────

async fn service_map_handler(
    AxumState(state): AxumState<Arc<TraceState>>,
) -> Json<serde_json::Value> {
    let store = state.store.lock().await;
    let map = analyzer::service_dependency_map(&store.spans);
    Json(serde_json::to_value(map).unwrap())
}

// ── GET /api/v1/traces/latency ────────────────────────────────────────────────

async fn latency_handler(
    AxumState(state): AxumState<Arc<TraceState>>,
    Query(query): Query<LatencyQuery>,
) -> Json<serde_json::Value> {
    let store = state.store.lock().await;

    let spans: Vec<&Span> = store
        .spans
        .iter()
        .filter(|s| {
            query
                .service
                .as_ref()
                .map(|svc| &s.service == svc)
                .unwrap_or(true)
        })
        .collect();

    let owned: Vec<Span> = spans.into_iter().cloned().collect();
    let stats = analyzer::latency_breakdown(&owned);
    let anomalies = collector::detect_anomalous_spans(&owned);

    Json(serde_json::json!({
        "stats": stats,
        "anomalous_span_count": anomalies.len(),
        "bottlenecks": analyzer::bottleneck_detection(&owned),
    }))
}

#[derive(serde::Deserialize, Default)]
struct LatencyQuery {
    service: Option<String>,
}
<<<<<<< HEAD
=======
use serde::Deserialize;
use std::sync::{Arc, Mutex};

pub struct TraceState {
    pub store: Arc<TraceStore>,
    pub query_engine: QueryEngine,
    pub sampler: Mutex<Sampler>,
}

impl TraceState {
    pub fn new() -> Self {
        let store = Arc::new(TraceStore::new(10_000));
        let query_engine = QueryEngine::new(store.clone());
        TraceState {
            store,
            query_engine,
            sampler: Mutex::new(Sampler::new(SamplingStrategy::Probabilistic {
                sampling_rate: 1.0,
            })),
        }
    }
}

impl Default for TraceState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn router(state: Arc<TraceState>) -> Router {
    Router::new()
        .route("/api/trace/health", get(health))
        .route("/api/trace/v1/traces", post(ingest_otlp))
        .route("/api/trace/services", get(list_services))
        .route(
            "/api/trace/services/:service/operations",
            get(list_operations),
        )
        .route("/api/trace/traces", get(find_traces))
        .route(
            "/api/trace/traces/:trace_id",
            get(get_trace).delete(delete_trace),
        )
        .route(
            "/api/trace/traces/:trace_id/spans/:span_id",
            get(get_span),
        )
        .route("/api/trace/dependencies", get(get_dependencies))
        .route("/api/trace/compare", get(compare_traces))
        .route("/api/trace/slowest", get(slowest_traces))
        .route("/api/trace/errors", get(error_traces))
        .route("/api/trace/sampling/strategy", get(get_sampling_strategy))
        .route("/api/trace/stats", get(get_stats))
        .with_state(state)
}

// -------------------------------------------------------------------------
// Handlers
// -------------------------------------------------------------------------

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({"status": "ok", "module": crate::MODULE_NAME}))
}

async fn ingest_otlp(
    State(state): State<Arc<TraceState>>,
    body: String,
) -> impl IntoResponse {
    match OtlpReceiver::parse_export(&body) {
        Ok(spans) => match state.store.ingest_spans(spans) {
            Ok(()) => (
                StatusCode::OK,
                Json(serde_json::json!({"status": "accepted"})),
            ),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            ),
        },
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

async fn list_services(State(state): State<Arc<TraceState>>) -> impl IntoResponse {
    let services = state.store.list_services();
    Json(serde_json::json!({"services": services}))
}

async fn list_operations(
    State(state): State<Arc<TraceState>>,
    Path(service): Path<String>,
) -> impl IntoResponse {
    let ops = state.store.list_operations(&service);
    Json(serde_json::json!({"service": service, "operations": ops}))
}

#[derive(Deserialize)]
struct FindTracesQuery {
    service: Option<String>,
    operation: Option<String>,
    limit: Option<usize>,
    min_duration: Option<i64>,
    max_duration: Option<i64>,
}

async fn find_traces(
    State(state): State<Arc<TraceState>>,
    Query(q): Query<FindTracesQuery>,
) -> impl IntoResponse {
    let query = TraceQuery {
        service_name: q.service,
        operation_name: q.operation,
        min_duration_us: q.min_duration,
        max_duration_us: q.max_duration,
        limit: q.limit.or(Some(20)),
        ..Default::default()
    };
    let traces = state.query_engine.find_traces(&query);
    Json(serde_json::to_value(traces).unwrap())
}

async fn get_trace(
    State(state): State<Arc<TraceState>>,
    Path(trace_id): Path<String>,
) -> impl IntoResponse {
    match state.query_engine.get_trace(&trace_id) {
        Ok(trace) => (StatusCode::OK, Json(serde_json::to_value(trace).unwrap())),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

async fn get_span(
    State(state): State<Arc<TraceState>>,
    Path((_trace_id, span_id)): Path<(String, String)>,
) -> impl IntoResponse {
    match state.store.get_span(&span_id) {
        Ok(span) => (StatusCode::OK, Json(serde_json::to_value(span).unwrap())),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

async fn delete_trace(
    State(state): State<Arc<TraceState>>,
    Path(trace_id): Path<String>,
) -> impl IntoResponse {
    match state.store.delete_trace(&trace_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"deleted": trace_id})),
        ),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

async fn get_dependencies(State(state): State<Arc<TraceState>>) -> impl IntoResponse {
    let traces = state.store.all_traces();
    let graph = DependencyComputer::compute(&traces);
    Json(serde_json::to_value(graph).unwrap())
}

#[derive(Deserialize)]
struct CompareQuery {
    a: String,
    b: String,
}

async fn compare_traces(
    State(state): State<Arc<TraceState>>,
    Query(q): Query<CompareQuery>,
) -> impl IntoResponse {
    let trace_a = match state.store.get_trace(&q.a) {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": e.to_string()})),
            );
        }
    };
    let trace_b = match state.store.get_trace(&q.b) {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": e.to_string()})),
            );
        }
    };
    let cmp = TraceComparer::compare(&trace_a, &trace_b);
    (StatusCode::OK, Json(serde_json::to_value(cmp).unwrap()))
}

#[derive(Deserialize)]
struct SlowestQuery {
    service: Option<String>,
    limit: Option<usize>,
}

async fn slowest_traces(
    State(state): State<Arc<TraceState>>,
    Query(q): Query<SlowestQuery>,
) -> impl IntoResponse {
    let traces = state
        .query_engine
        .slowest_traces(q.service.as_deref(), q.limit.unwrap_or(10));
    Json(serde_json::to_value(traces).unwrap())
}

#[derive(Deserialize)]
struct ErrorQuery {
    service: Option<String>,
    limit: Option<usize>,
}

async fn error_traces(
    State(state): State<Arc<TraceState>>,
    Query(q): Query<ErrorQuery>,
) -> impl IntoResponse {
    let traces = state
        .query_engine
        .error_traces(q.service.as_deref(), q.limit.unwrap_or(10));
    Json(serde_json::to_value(traces).unwrap())
}

async fn get_sampling_strategy(State(state): State<Arc<TraceState>>) -> impl IntoResponse {
    let rate = state.sampler.lock().unwrap().sampling_rate();
    Json(serde_json::json!({"sampling_rate": rate}))
}

async fn get_stats(State(state): State<Arc<TraceState>>) -> impl IntoResponse {
    let count = state.store.trace_count();
    let services = state.store.list_services();
    Json(serde_json::json!({
        "trace_count": count,
        "service_count": services.len(),
        "services": services,
    }))
}
>>>>>>> claude/dazzling-tesla
=======
>>>>>>> claude/elastic-ellis
=======
//! HTTP routes for cave-trace.
//!
//! Exposes two route groups:
//!   /api/trace/*  — cave-native management API
//!   /v1/*         — OTLP/HTTP receiver (drop-in for otel-collector OTLP receiver)
//!
//! The OTLP endpoint accepts both JSON and binary-protobuf bodies:
//!   Content-Type: application/json            → decoded via serde_json
//!   Content-Type: application/x-protobuf      → raw bytes queued for proto decode

use crate::{models::ExportTraceServiceResponse, TraceState};
use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;

pub fn create_router(state: Arc<TraceState>) -> Router {
    Router::new()
        // ── cave-native ────────────────────────────────────────────────────
        .route("/api/trace/health", get(health))
        // ── OTLP/HTTP trace receiver ───────────────────────────────────────
        // POST /v1/traces — OTLP ExportTraceServiceRequest
        // Accepts: application/x-protobuf | application/json
        .route("/v1/traces", post(export_traces))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// cave-native
// ---------------------------------------------------------------------------

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-trace",
        "status": "ok",
        "upstream": "opentelemetry-collector",
        "upstream_tracked_version": "0.x",
        "compat": ["otlp_http_v1"]
    }))
}

// ---------------------------------------------------------------------------
// OTLP trace receiver — POST /v1/traces
// ---------------------------------------------------------------------------

/// Accept an OTLP ExportTraceServiceRequest.
///
/// Per the OTLP/HTTP spec:
///   - 200 OK with ExportTraceServiceResponse body on full success
///   - 200 OK with partialSuccess populated when some spans were dropped
///   - 400 Bad Request for malformed input
///   - 503 Service Unavailable when the backend is overloaded
///
/// Content negotiation:
///   application/json        → parse JSON, log span count
///   application/x-protobuf  → accept bytes, queue for async protobuf decode
async fn export_traces(
    State(_state): State<Arc<TraceState>>,
    request: Request,
) -> (StatusCode, Json<ExportTraceServiceResponse>) {
    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_string();

    // Collect the body bytes regardless of content-type
    let body_bytes = match axum::body::to_bytes(request.into_body(), usize::MAX).await {
        Ok(b) => b,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ExportTraceServiceResponse { partial_success: None }),
            );
        }
    };

    if content_type.contains("application/x-protobuf") {
        // TODO: decode prost ExportTraceServiceRequest from body_bytes
        tracing::debug!(bytes = body_bytes.len(), "otlp protobuf traces received");
    } else {
        // JSON path — parse to count spans for observability
        match serde_json::from_slice::<crate::models::ExportTraceServiceRequest>(&body_bytes) {
            Ok(req) => {
                let span_count: usize = req
                    .resource_spans
                    .iter()
                    .flat_map(|rs| &rs.scope_spans)
                    .map(|ss| ss.spans.len())
                    .sum();
                tracing::debug!(
                    resource_spans = req.resource_spans.len(),
                    spans          = span_count,
                    "otlp json traces received"
                );
                // TODO: persist spans to trace store
            }
            Err(e) => {
                tracing::warn!(err = %e, "failed to parse OTLP JSON traces");
            }
        }
    }

    // Full success response — empty ExportTraceServiceResponse per OTLP spec
    (
        StatusCode::OK,
        Json(ExportTraceServiceResponse { partial_success: None }),
    )
}
>>>>>>> claude/gallant-cartwright
