// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP routes for cave-slo.
//!
//! API surface:
//!   GET    /api/slo/health             — liveness probe
//!   GET    /api/slo/slos               — list all SLOs
//!   POST   /api/slo/slos               — create SLO
//!   GET    /api/slo/slos/:id           — get one SLO
//!   PUT    /api/slo/slos/:id           — update SLO
//!   DELETE /api/slo/slos/:id           — delete SLO
//!   GET    /api/slo/slos/:id/budget    — compute current error budget
//!   GET    /api/slo/stats              — aggregate stats

use crate::{
    engine::{calculate_error_budget, is_compliant},
    models::{MetricType, SloStatus, SLO},
    State,
};
use axum::{
    Json, Router,
    extract::{Path, State as AxumState},
    http::StatusCode,
    routing::{delete, get, post, put},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router(state: Arc<State>) -> Router {
    Router::new()
        .route("/api/slo/health", get(health))
        .route("/api/slo/slos", get(list_slos).post(create_slo))
        .route(
            "/api/slo/slos/{id}",
            get(get_slo).put(update_slo).delete(delete_slo),
        )
        .route("/api/slo/slos/{id}/budget", get(get_budget))
        .route("/api/slo/stats", get(get_stats))
        .with_state(state)
}

// ── Handlers ─────────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(json!({
        "module": "cave-slo",
        "status": "ok",
        "upstream": "nobl9/nobl9-go v0.126.1"
    }))
}

#[derive(Debug, Deserialize)]
struct CreateSloRequest {
    name: String,
    description: String,
    target_percentage: f64,
    window_days: u32,
    metric_type: MetricType,
}

async fn create_slo(
    AxumState(state): AxumState<Arc<State>>,
    Json(req): Json<CreateSloRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let slo = SLO {
        id: Uuid::new_v4(),
        name: req.name,
        description: req.description,
        target_percentage: req.target_percentage,
        window_days: req.window_days,
        metric_type: req.metric_type,
        created_at: Utc::now(),
        current_sli: req.target_percentage, // starts at target — no data yet
        status: SloStatus::Unknown,
    };
    let json = serde_json::to_value(&slo).unwrap();
    state.store.insert(slo);
    (StatusCode::CREATED, Json(json))
}

async fn list_slos(
    AxumState(state): AxumState<Arc<State>>,
) -> Json<serde_json::Value> {
    let slos = state.store.list();
    Json(serde_json::to_value(slos).unwrap())
}

async fn get_slo(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    state
        .store
        .get(id)
        .map(|slo| Json(serde_json::to_value(slo).unwrap()))
        .ok_or(StatusCode::NOT_FOUND)
}

#[derive(Debug, Deserialize)]
struct UpdateSloRequest {
    name: Option<String>,
    description: Option<String>,
    target_percentage: Option<f64>,
    window_days: Option<u32>,
    metric_type: Option<MetricType>,
}

async fn update_slo(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateSloRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let mut slo = state.store.get(id).ok_or(StatusCode::NOT_FOUND)?;
    if let Some(name) = req.name { slo.name = name; }
    if let Some(description) = req.description { slo.description = description; }
    if let Some(target) = req.target_percentage { slo.target_percentage = target; }
    if let Some(days) = req.window_days { slo.window_days = days; }
    if let Some(mt) = req.metric_type { slo.metric_type = mt; }
    let json = serde_json::to_value(&slo).unwrap();
    state.store.update(slo);
    Ok(Json(json))
}

async fn delete_slo(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    if state.store.delete(id) {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

async fn get_budget(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let slo = state.store.get(id).ok_or(StatusCode::NOT_FOUND)?;
    // Use current_sli to derive good/total for the budget calculation.
    // When no real metric data has been ingested, we treat the SLO as
    // "perfect" (current_sli == target → 0 budget consumed).
    let total_requests: u64 = 1_000_000;
    let sli_fraction = (slo.current_sli / 100.0).clamp(0.0, 1.0);
    let good_requests = (sli_fraction * total_requests as f64).round() as u64;
    let budget = calculate_error_budget(&slo, good_requests, total_requests);
    let compliant = is_compliant(&budget);
    let mut val = serde_json::to_value(&budget).unwrap();
    val["compliant"] = serde_json::Value::Bool(compliant);
    Ok(Json(val))
}

async fn get_stats(
    AxumState(state): AxumState<Arc<State>>,
) -> Json<serde_json::Value> {
    let stats = state.store.compute_stats();
    Json(serde_json::to_value(stats).unwrap())
}
