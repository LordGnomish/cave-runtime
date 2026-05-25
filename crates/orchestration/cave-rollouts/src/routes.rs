// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! REST API for the rollouts module.
//!
//! Implements Argo Rollouts + Flagger-compatible endpoints for creating,
//! querying, and controlling progressive delivery rollouts.

use axum::{
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{delete, get, post, put},
};
use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    RolloutsState,
    engine::{advance_canary, apply_canary_action, initial_status},
    models::*,
};

// ── Router ────────────────────────────────────────────────────────────────────

pub fn create_router(state: Arc<RolloutsState>) -> Router {
    Router::new()
        // Health
        .route("/api/rollouts/health", get(health))
        // Rollouts CRUD
        .route("/api/rollouts", get(list_rollouts).post(create_rollout))
        .route(
            "/api/rollouts/{rollout_id}",
            get(get_rollout).put(update_rollout).delete(delete_rollout),
        )
        .route("/api/rollouts/{rollout_id}/status", get(rollout_status))
        .route("/api/rollouts/{rollout_id}/action", post(rollout_action))
        // Analysis templates
        .route(
            "/api/rollouts/analysis/templates",
            get(list_analysis_templates).post(create_analysis_template),
        )
        .route(
            "/api/rollouts/analysis/templates/{template_id}",
            get(get_analysis_template).delete(delete_analysis_template),
        )
        // Analysis runs
        .route(
            "/api/rollouts/{rollout_id}/analysis/runs",
            get(list_analysis_runs),
        )
        .route(
            "/api/rollouts/{rollout_id}/analysis/runs/{run_id}",
            get(get_analysis_run),
        )
        .with_state(state)
}

// ── Health ────────────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-rollouts",
        "status": "ok",
        "upstream": ["flagger", "argo-rollouts"],
        "upstream_tracked_versions": {
            "flagger": "1.x",
            "argo-rollouts": "1.x"
        }
    }))
}

// ── Rollouts ──────────────────────────────────────────────────────────────────

async fn list_rollouts(
    State(_state): State<Arc<RolloutsState>>,
    Query(q): Query<PaginationQuery>,
) -> Json<Vec<Rollout>> {
    let _ = q;
    // TODO: query DB
    Json(vec![])
}

async fn create_rollout(
    State(_state): State<Arc<RolloutsState>>,
    Json(req): Json<CreateRolloutRequest>,
) -> impl IntoResponse {
    let now = Utc::now();
    let status = initial_status(&req.strategy);
    let rollout = Rollout {
        id: Uuid::new_v4(),
        name: req.name,
        namespace: req.namespace,
        workload_ref: req.workload_ref,
        strategy: req.strategy,
        status,
        traffic: req.traffic,
        analysis: req.analysis,
        notifications: req.notifications.unwrap_or_default(),
        created_at: now,
        updated_at: now,
    };
    // TODO: persist
    (StatusCode::CREATED, Json(rollout))
}

async fn get_rollout(
    State(_state): State<Arc<RolloutsState>>,
    Path(rollout_id): Path<Uuid>,
) -> impl IntoResponse {
    let _ = rollout_id;
    StatusCode::NOT_FOUND
}

async fn update_rollout(
    State(_state): State<Arc<RolloutsState>>,
    Path(rollout_id): Path<Uuid>,
    Json(_req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let _ = rollout_id;
    StatusCode::OK
}

async fn delete_rollout(
    State(_state): State<Arc<RolloutsState>>,
    Path(rollout_id): Path<Uuid>,
) -> StatusCode {
    let _ = rollout_id;
    StatusCode::NO_CONTENT
}

async fn rollout_status(
    State(_state): State<Arc<RolloutsState>>,
    Path(rollout_id): Path<Uuid>,
) -> impl IntoResponse {
    let _ = rollout_id;
    StatusCode::NOT_FOUND
}

/// POST /api/rollouts/{id}/action — promote, abort, pause, resume, retry
async fn rollout_action(
    State(_state): State<Arc<RolloutsState>>,
    Path(rollout_id): Path<Uuid>,
    Json(req): Json<RolloutActionRequest>,
) -> impl IntoResponse {
    let _ = rollout_id;
    // TODO: load rollout from DB, call apply_canary_action / blue_green equivalent
    let _ = req;
    StatusCode::ACCEPTED
}

// ── Analysis Templates ────────────────────────────────────────────────────────

async fn list_analysis_templates(
    State(_state): State<Arc<RolloutsState>>,
    Query(_q): Query<PaginationQuery>,
) -> Json<Vec<AnalysisTemplate>> {
    Json(vec![])
}

async fn create_analysis_template(
    State(_state): State<Arc<RolloutsState>>,
    Json(req): Json<CreateAnalysisTemplateRequest>,
) -> impl IntoResponse {
    let tmpl = AnalysisTemplate {
        id: Uuid::new_v4(),
        name: req.name,
        namespace: req.namespace,
        metrics: req.metrics,
        dry_run_metrics: vec![],
        args: req.args,
        created_at: Utc::now(),
    };
    (StatusCode::CREATED, Json(tmpl))
}

async fn get_analysis_template(
    State(_state): State<Arc<RolloutsState>>,
    Path(template_id): Path<Uuid>,
) -> impl IntoResponse {
    let _ = template_id;
    StatusCode::NOT_FOUND
}

async fn delete_analysis_template(
    State(_state): State<Arc<RolloutsState>>,
    Path(template_id): Path<Uuid>,
) -> StatusCode {
    let _ = template_id;
    StatusCode::NO_CONTENT
}

// ── Analysis Runs ─────────────────────────────────────────────────────────────

async fn list_analysis_runs(
    State(_state): State<Arc<RolloutsState>>,
    Path(rollout_id): Path<Uuid>,
) -> Json<Vec<AnalysisRun>> {
    let _ = rollout_id;
    Json(vec![])
}

async fn get_analysis_run(
    State(_state): State<Arc<RolloutsState>>,
    Path((_rollout_id, _run_id)): Path<(Uuid, Uuid)>,
) -> impl IntoResponse {
    StatusCode::NOT_FOUND
}
