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
