// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Ingestion HTTP endpoints.
//!
//! Endpoints
//! ─────────
//! POST /v1/traces                        — OTLP HTTP/JSON
//! POST /api/v2/spans                     — Zipkin v2 JSON
//! POST /api/traces                       — Jaeger HTTP collector JSON
//! POST /oc/v1/traces                     — OpenCensus JSON
//! POST /v1/traces (x-protobuf)           — OTLP HTTP/proto   → 501
//! POST /api/traces (application/x-thrift)— Jaeger Thrift binary

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::{
    ingestion::{jaeger, opencensus, otlp, zipkin},
    multi_tenant::tenant_from_headers,
    sampling::Sampler,
    TraceState,
};

pub fn create_router(state: Arc<TraceState>) -> Router {
    Router::new()
        // OTLP HTTP
        .route("/v1/traces", post(ingest_otlp))
        // Zipkin v2
        .route("/api/v2/spans", post(ingest_zipkin))
        // Jaeger HTTP collector
        .route("/api/traces", post(ingest_jaeger))
        // OpenCensus
        .route("/oc/v1/traces", post(ingest_opencensus))
        .with_state(state)
}

// ─── OTLP ─────────────────────────────────────────────────────────────────

async fn ingest_otlp(
    State(state): State<Arc<TraceState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json");

    let tenant_id = tenant_from_headers(&headers);

    if content_type.contains("application/x-protobuf") || content_type.contains("application/grpc") {
        return (
            StatusCode::NOT_IMPLEMENTED,
            Json(serde_json::json!({
                "error": "OTLP protobuf encoding requires compiled proto definitions. Use application/json instead."
            }))
        ).into_response();
    }

    match otlp::parse_otlp_json(&body, &tenant_id) {
        Ok(spans) => {
            let accepted = ingest_spans_with_sampling(&state, spans).await;
            Json(otlp::ExportTraceServiceResponse::ok()).into_response()
        }
        Err(e) => {
            tracing::warn!("OTLP ingestion error: {}", e);
            (StatusCode::BAD_REQUEST, format!("{}", e)).into_response()
        }
    }
}

// ─── Zipkin ────────────────────────────────────────────────────────────────

async fn ingest_zipkin(
    State(state): State<Arc<TraceState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let tenant_id = tenant_from_headers(&headers);

    match zipkin::parse_zipkin_json(&body, &tenant_id) {
        Ok(spans) => {
            ingest_spans_with_sampling(&state, spans).await;
            StatusCode::ACCEPTED.into_response()
        }
        Err(e) => {
            tracing::warn!("Zipkin ingestion error: {}", e);
            (StatusCode::BAD_REQUEST, format!("{}", e)).into_response()
        }
    }
}

// ─── Jaeger ────────────────────────────────────────────────────────────────

async fn ingest_jaeger(
    State(state): State<Arc<TraceState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let tenant_id = tenant_from_headers(&headers);

    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json");

    let result = if content_type.contains("application/x-thrift") {
        jaeger::parse_jaeger_thrift_binary(&body, &tenant_id)
    } else {
        jaeger::parse_jaeger_json(&body, &tenant_id)
    };

    match result {
        Ok(spans) => {
            ingest_spans_with_sampling(&state, spans).await;
            StatusCode::ACCEPTED.into_response()
        }
        Err(e) => {
            tracing::warn!("Jaeger ingestion error: {}", e);
            (StatusCode::BAD_REQUEST, format!("{}", e)).into_response()
        }
    }
}

// ─── OpenCensus ────────────────────────────────────────────────────────────

async fn ingest_opencensus(
    State(state): State<Arc<TraceState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let tenant_id = tenant_from_headers(&headers);

    match opencensus::parse_opencensus_json(&body, &tenant_id) {
        Ok(spans) => {
            ingest_spans_with_sampling(&state, spans).await;
            StatusCode::ACCEPTED.into_response()
        }
        Err(e) => {
            tracing::warn!("OpenCensus ingestion error: {}", e);
            (StatusCode::BAD_REQUEST, format!("{}", e)).into_response()
        }
    }
}

// ─── Shared ingestion helper ───────────────────────────────────────────────

async fn ingest_spans_with_sampling(state: &TraceState, spans: Vec<crate::types::Span>) -> usize {
    if spans.is_empty() { return 0; }

    // Group spans by trace_id to apply head-based sampling
    use std::collections::HashMap;
    let mut trace_groups: HashMap<crate::types::TraceId, Vec<crate::types::Span>> = HashMap::new();
    for span in spans {
        trace_groups.entry(span.trace_id).or_default().push(span);
    }

    let mut accepted_spans = Vec::new();
    for (trace_id, trace_spans) in trace_groups {
        // Find a root span or use the first one for sampling decision
        let root_span = trace_spans
            .iter()
            .find(|s| s.is_root())
            .unwrap_or(&trace_spans[0]);

        if state.sampler.should_sample(trace_id, root_span).is_sample() {
            state.spm_registry.record_spans(&trace_spans);
            accepted_spans.extend(trace_spans);
        }
    }

    let count = accepted_spans.len();
    if count > 0 {
        let mut store = state.store.write().await;
        store.ingest_spans(accepted_spans);
    }
    count
}
