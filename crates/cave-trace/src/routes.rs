use crate::models::{IngestResponse, IngestSpanRequest, SearchResponse, Span, TraceQuery};
use crate::{analyzer, collector, TraceState};
use axum::{
    extract::{Path, Query, State as AxumState},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
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
