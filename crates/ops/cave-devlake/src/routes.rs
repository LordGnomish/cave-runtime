// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP routes for cave-devlake — DORA metrics, deployments, pipelines, incidents, PRs, commits, issues, sprints.

use crate::State;
use axum::{
    Json, Router,
    extract::{Path, State as AxumState},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router(state: Arc<State>) -> Router {
    Router::new()
        // Health
        .route("/api/devlake/health", get(health))
        // DORA
        .route("/api/devlake/dora", get(dora_report))
        // Deployments
        .route("/api/devlake/deployments", get(list_deployments))
        .route("/api/devlake/deployments/{id}", get(get_deployment))
        // Pipelines
        .route("/api/devlake/pipelines", get(list_pipelines))
        .route("/api/devlake/pipelines/{id}", get(get_pipeline))
        // Incidents
        .route("/api/devlake/incidents", get(list_incidents))
        .route("/api/devlake/incidents/{id}", get(get_incident))
        // Pull Requests
        .route("/api/devlake/prs", get(list_prs))
        .route("/api/devlake/prs/{id}", get(get_pr))
        // Commits
        .route("/api/devlake/commits", get(list_commits))
        // Issues
        .route("/api/devlake/issues", get(list_issues))
        .route("/api/devlake/issues/{id}", get(get_issue))
        // Sprints
        .route("/api/devlake/sprints", get(list_sprints))
        .route("/api/devlake/sprints/{id}", get(get_sprint))
        .with_state(state)
}

// ── Health ────────────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-devlake",
        "status": "ok",
        "upstream": "Apache DevLake v0.21.1"
    }))
}

// ── DORA Report ───────────────────────────────────────────────────────────────

async fn dora_report(AxumState(state): AxumState<Arc<State>>) -> impl IntoResponse {
    let report = state.store.compute_dora_report(30);
    (StatusCode::OK, Json(report))
}

// ── Deployments ───────────────────────────────────────────────────────────────

async fn list_deployments(AxumState(state): AxumState<Arc<State>>) -> impl IntoResponse {
    let deployments = state.store.list_deployments();
    (StatusCode::OK, Json(deployments))
}

async fn get_deployment(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.store.get_deployment(id) {
        Some(d) => (StatusCode::OK, Json(serde_json::to_value(d).unwrap())),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "deployment not found"})),
        ),
    }
}

// ── Pipelines ─────────────────────────────────────────────────────────────────

async fn list_pipelines(AxumState(state): AxumState<Arc<State>>) -> impl IntoResponse {
    let pipelines = state.store.list_pipelines();
    (StatusCode::OK, Json(pipelines))
}

async fn get_pipeline(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.store.get_pipeline(id) {
        Some(p) => (StatusCode::OK, Json(serde_json::to_value(p).unwrap())),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "pipeline not found"})),
        ),
    }
}

// ── Incidents ─────────────────────────────────────────────────────────────────

async fn list_incidents(AxumState(state): AxumState<Arc<State>>) -> impl IntoResponse {
    let incidents = state.store.list_incidents();
    (StatusCode::OK, Json(incidents))
}

async fn get_incident(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.store.get_incident(id) {
        Some(i) => (StatusCode::OK, Json(serde_json::to_value(i).unwrap())),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "incident not found"})),
        ),
    }
}

// ── Pull Requests ─────────────────────────────────────────────────────────────

async fn list_prs(AxumState(state): AxumState<Arc<State>>) -> impl IntoResponse {
    let prs = state.store.list_prs();
    (StatusCode::OK, Json(prs))
}

async fn get_pr(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.store.get_pr(id) {
        Some(pr) => (StatusCode::OK, Json(serde_json::to_value(pr).unwrap())),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "pull request not found"})),
        ),
    }
}

// ── Commits ───────────────────────────────────────────────────────────────────

async fn list_commits(AxumState(state): AxumState<Arc<State>>) -> impl IntoResponse {
    let commits = state.store.list_commits();
    (StatusCode::OK, Json(commits))
}

// ── Issues ────────────────────────────────────────────────────────────────────

async fn list_issues(AxumState(state): AxumState<Arc<State>>) -> impl IntoResponse {
    let issues = state.store.list_issues();
    (StatusCode::OK, Json(issues))
}

async fn get_issue(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.store.get_issue(id) {
        Some(i) => (StatusCode::OK, Json(serde_json::to_value(i).unwrap())),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "issue not found"})),
        ),
    }
}

// ── Sprints ───────────────────────────────────────────────────────────────────

async fn list_sprints(AxumState(state): AxumState<Arc<State>>) -> impl IntoResponse {
    let sprints = state.store.list_sprints();
    (StatusCode::OK, Json(sprints))
}

async fn get_sprint(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.store.get_sprint(id) {
        Some(s) => (StatusCode::OK, Json(serde_json::to_value(s).unwrap())),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "sprint not found"})),
        ),
    }
}
