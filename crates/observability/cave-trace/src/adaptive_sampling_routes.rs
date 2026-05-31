// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Jaeger adaptive-sampling strategy REST surface.
//!
//! Endpoints
//! ─────────
//! • POST /api/sampling/calculate — recompute per-operation probabilities from a
//!   posted throughput batch (jaeger adaptive processor). Body:
//!   `{ "target_qps": f64, "interval_secs": f64, "initial_probability": f64?,
//!      "throughput": [ {service, operation, count}, ... ] }`
//!   Response: `{ service -> { operation -> probability } }`.
//!
//! Mirrors the jaeger adaptive strategy store's post-aggregation calculation
//! (plugin/sampling/strategystore/adaptive/processor.go) as a reachable surface.

use axum::{Json, Router, routing::post};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::TraceState;
use crate::adaptive_sampling::{
    AdaptiveSamplingProcessor, DEFAULT_INITIAL_SAMPLING_PROBABILITY, Throughput,
};

#[derive(Debug, Deserialize)]
struct ThroughputEntry {
    service: String,
    operation: String,
    count: u64,
}

#[derive(Debug, Deserialize)]
struct CalculateRequest {
    target_qps: f64,
    interval_secs: f64,
    #[serde(default)]
    initial_probability: Option<f64>,
    #[serde(default)]
    throughput: Vec<ThroughputEntry>,
}

#[derive(Debug, Serialize)]
struct CalculateResponse {
    probabilities: HashMap<String, HashMap<String, f64>>,
}

/// POST /api/sampling/calculate
async fn calculate(Json(req): Json<CalculateRequest>) -> Json<CalculateResponse> {
    let mut processor = AdaptiveSamplingProcessor::new(req.target_qps, req.interval_secs)
        .with_initial_probability(
            req.initial_probability
                .unwrap_or(DEFAULT_INITIAL_SAMPLING_PROBABILITY),
        );
    let tp: Vec<Throughput> = req
        .throughput
        .into_iter()
        .map(|e| Throughput::new(e.service, e.operation, e.count))
        .collect();
    let probabilities = processor.calculate_probabilities(&tp);
    Json(CalculateResponse { probabilities })
}

/// Router for the adaptive-sampling strategy surface.
pub fn create_router(state: Arc<TraceState>) -> Router {
    Router::new()
        .route("/api/sampling/calculate", post(calculate))
        .with_state(state)
}
