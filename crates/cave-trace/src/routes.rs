use crate::comparison::TraceComparer;
use crate::dependency::DependencyComputer;
use crate::otlp::OtlpReceiver;
use crate::query::QueryEngine;
use crate::sampling::{Sampler, SamplingStrategy};
use crate::storage::TraceStore;
use crate::types::TraceQuery;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
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
