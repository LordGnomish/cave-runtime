// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP routes for cave-upstream.

use crate::models::{HealthCheck, UpstreamAlert, UpstreamService, UpstreamStats};
use crate::store::UpstreamStore;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;
use uuid::Uuid;

pub struct UpstreamState {
    pub store: UpstreamStore,
}

pub fn create_router() -> Router {
    let state = Arc::new({
        let s = UpstreamState {
            store: UpstreamStore::new(),
        };
        s.store.seed_demo_data();
        s
    });

    Router::new()
        .route("/api/upstream/health", get(health))
        .route("/api/upstream/upstreams", get(list_upstreams).post(create_upstream))
        .route(
            "/api/upstream/upstreams/{id}",
            get(get_upstream).put(update_upstream).delete(delete_upstream),
        )
        .route("/api/upstream/upstreams/{id}/check", post(check_upstream))
        .route("/api/upstream/upstreams/{id}/health-history", get(health_history))
        .route("/api/upstream/upstreams/{id}/alerts", get(service_alerts))
        .route("/api/upstream/alerts", get(all_alerts))
        .route("/api/upstream/attention", get(attention))
        .route("/api/upstream/stats", get(stats))
        .with_state(state)
}

// ── Health ─────────────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-upstream",
        "status": "ok"
    }))
}

// ── Services ──────────────────────────────────────────────────────────────────

async fn list_upstreams(State(state): State<Arc<UpstreamState>>) -> Json<Vec<UpstreamService>> {
    Json(state.store.list_services())
}

async fn create_upstream(
    State(state): State<Arc<UpstreamState>>,
    Json(service): Json<UpstreamService>,
) -> (StatusCode, Json<UpstreamService>) {
    state.store.insert_service(service.clone());
    (StatusCode::CREATED, Json(service))
}

async fn get_upstream(
    State(state): State<Arc<UpstreamState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<UpstreamService>, StatusCode> {
    state
        .store
        .get_service(id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn update_upstream(
    State(state): State<Arc<UpstreamState>>,
    Path(id): Path<Uuid>,
    Json(service): Json<UpstreamService>,
) -> Result<Json<UpstreamService>, StatusCode> {
    state
        .store
        .update_service(id, service)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn delete_upstream(
    State(state): State<Arc<UpstreamState>>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    if state.store.delete_service(id) {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

// ── Health checks ─────────────────────────────────────────────────────────────

async fn check_upstream(
    State(state): State<Arc<UpstreamState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<HealthCheck>, StatusCode> {
    let service = state.store.get_service(id).ok_or(StatusCode::NOT_FOUND)?;
    let check = state.store.check_health_simulated(&service);
    Ok(Json(check))
}

async fn health_history(
    State(state): State<Arc<UpstreamState>>,
    Path(id): Path<Uuid>,
) -> Json<Vec<HealthCheck>> {
    Json(state.store.get_health_history(id))
}

// ── Alerts ────────────────────────────────────────────────────────────────────

async fn service_alerts(
    State(state): State<Arc<UpstreamState>>,
    Path(id): Path<Uuid>,
) -> Json<Vec<UpstreamAlert>> {
    Json(state.store.get_alerts(id))
}

async fn all_alerts(State(state): State<Arc<UpstreamState>>) -> Json<Vec<UpstreamAlert>> {
    Json(state.store.all_active_alerts())
}

// ── Attention / Stats ─────────────────────────────────────────────────────────

async fn attention(State(state): State<Arc<UpstreamState>>) -> Json<Vec<UpstreamService>> {
    Json(state.store.services_needing_attention())
}

async fn stats(State(state): State<Arc<UpstreamState>>) -> Json<UpstreamStats> {
    Json(state.store.compute_stats())
}
