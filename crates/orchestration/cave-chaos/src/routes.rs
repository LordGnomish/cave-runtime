// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP routes for cave-chaos — Chaos Mesh-compatible REST API.

use crate::models::{
    BlastRadius, ChaosExperiment, ExperimentParams, ExperimentStatus, SafetyGuard,
};
use crate::State;
use axum::{
    Json, Router,
    extract::{Path, State as AxumState},
    http::StatusCode,
    routing::{delete, get, post},
};
use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router(state: Arc<State>) -> Router {
    Router::new()
        .route("/api/chaos/health", get(health))
        .route("/api/chaos/metrics", get(metrics))
        .route("/api/chaos/experiments", get(list_experiments))
        .route("/api/chaos/experiments", post(create_experiment))
        .route("/api/chaos/experiments/{id}", get(get_experiment))
        .route("/api/chaos/experiments/{id}", delete(delete_experiment))
        .route("/api/chaos/experiments/{id}/start", post(start_experiment))
        .route("/api/chaos/experiments/{id}/stop", post(stop_experiment))
        .with_state(state)
}

// ─── Health ───────────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-chaos",
        "status": "ok",
        "upstream": "Chaos Mesh"
    }))
}

// ─── Prometheus metrics ─────────────────────────────────────────────────────────

/// `GET /api/chaos/metrics` — Prometheus text exposition of experiment counters.
async fn metrics(AxumState(state): AxumState<Arc<State>>) -> ([(axum::http::HeaderName, &'static str); 1], String) {
    let body = crate::metrics::render_prometheus(&state.store.list());
    (
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        body,
    )
}

// ─── List experiments ─────────────────────────────────────────────────────────

async fn list_experiments(
    AxumState(state): AxumState<Arc<State>>,
) -> Json<Vec<ChaosExperiment>> {
    let mut list = state.store.list();
    list.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Json(list)
}

// ─── Create experiment ────────────────────────────────────────────────────────

/// Request body for creating an experiment.
#[derive(serde::Deserialize)]
pub struct CreateExperimentRequest {
    pub name: String,
    pub experiment_type: crate::models::ExperimentType,
    pub target: crate::models::ChaosTarget,
    pub parameters: ExperimentParams,
    pub duration_secs: u32,
    pub blast_radius: Option<BlastRadius>,
    pub safety_guard: Option<SafetyGuard>,
    pub annotations: Option<std::collections::HashMap<String, String>>,
}

async fn create_experiment(
    AxumState(state): AxumState<Arc<State>>,
    Json(req): Json<CreateExperimentRequest>,
) -> (StatusCode, Json<ChaosExperiment>) {
    let exp = ChaosExperiment {
        id: Uuid::new_v4(),
        name: req.name,
        experiment_type: req.experiment_type,
        target: req.target,
        parameters: req.parameters,
        status: ExperimentStatus::Draft,
        created_at: Utc::now(),
        started_at: None,
        ended_at: None,
        duration_secs: req.duration_secs,
        blast_radius: req.blast_radius.unwrap_or_default(),
        safety_guard: req.safety_guard.unwrap_or_default(),
        result: None,
        annotations: req.annotations.unwrap_or_default(),
    };
    state.store.insert(exp.clone());
    (StatusCode::CREATED, Json(exp))
}

// ─── Get experiment ───────────────────────────────────────────────────────────

async fn get_experiment(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ChaosExperiment>, StatusCode> {
    state
        .store
        .get(id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

// ─── Delete experiment ────────────────────────────────────────────────────────

async fn delete_experiment(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    if state.store.remove(id).is_some() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

// ─── Start experiment ─────────────────────────────────────────────────────────

async fn start_experiment(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ChaosExperiment>, StatusCode> {
    let mut exp = state.store.get(id).ok_or(StatusCode::NOT_FOUND)?;

    // Execute the experiment (synchronous simulation)
    let _result = state.executor.execute(&mut exp);

    // Persist updated experiment
    state.store.update(exp.clone());
    Ok(Json(exp))
}

// ─── Stop experiment ──────────────────────────────────────────────────────────

async fn stop_experiment(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ChaosExperiment>, StatusCode> {
    let mut exp = state.store.get(id).ok_or(StatusCode::NOT_FOUND)?;

    // Rollback (abort) a running experiment
    let _result = state.executor.rollback(&mut exp);
    state.store.update(exp.clone());
    Ok(Json(exp))
}
