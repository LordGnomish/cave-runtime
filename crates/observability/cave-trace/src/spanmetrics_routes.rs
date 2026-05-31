// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tempo span-metrics REST surface.
//!
//! • POST /tempo/api/metrics/spanmetrics — aggregate a posted span batch and
//!   return the Prometheus exposition of the resulting RED metrics.
//!   Body: `{ "enable_size": bool?, "spans": [ {service, span_name, span_kind,
//!   status_code, duration_secs, size_bytes?}, ... ] }`.

use axum::{Router, response::IntoResponse, routing::post, Json};
use serde::Deserialize;
use std::sync::Arc;

use crate::TraceState;
use crate::spanmetrics::{SpanMetricsConfig, SpanMetricsProcessor};

#[derive(Debug, Deserialize)]
struct SpanEntry {
    service: String,
    span_name: String,
    span_kind: String,
    status_code: String,
    duration_secs: f64,
    #[serde(default)]
    size_bytes: u64,
}

#[derive(Debug, Deserialize)]
struct SpanMetricsRequest {
    #[serde(default)]
    enable_size: bool,
    #[serde(default)]
    spans: Vec<SpanEntry>,
}

/// POST /tempo/api/metrics/spanmetrics → Prometheus text exposition.
async fn spanmetrics(Json(req): Json<SpanMetricsRequest>) -> impl IntoResponse {
    let mut p = SpanMetricsProcessor::new(SpanMetricsConfig {
        enable_size: req.enable_size,
        ..Default::default()
    });
    for s in &req.spans {
        p.record_span(
            &s.service,
            &s.span_name,
            &s.span_kind,
            &s.status_code,
            s.duration_secs,
            s.size_bytes,
        );
    }
    p.expose_prometheus()
}

pub fn create_router(state: Arc<TraceState>) -> Router {
    Router::new()
        .route("/tempo/api/metrics/spanmetrics", post(spanmetrics))
        .with_state(state)
}
