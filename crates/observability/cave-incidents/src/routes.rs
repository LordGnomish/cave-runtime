// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP routes for cave-incidents — full CRUD + lifecycle actions.

use crate::State;
use crate::engine;
use crate::models::{
    AcknowledgeRequest, CloseRequest, CreateIncidentRequest, CreatePostMortemRequest,
    PostMortem, ResolveRequest, AddResponderRequest, Responder,
};
use axum::{
    extract::{Path, State as AxumState},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router(state: Arc<State>) -> Router {
    Router::new()
        // Health + metrics (fixed paths before /:id)
        .route("/api/incidents/health", get(health))
        .route("/api/incidents/metrics", get(get_metrics))
        // Postmortem collection
        .route("/api/incidents/postmortems", post(create_postmortem))
        .route("/api/incidents/postmortems", get(list_postmortems))
        .route("/api/incidents/postmortems/:pm_id", get(get_postmortem))
        // Incident collection
        .route("/api/incidents", get(list_incidents))
        .route("/api/incidents", post(create_incident))
        // Incident item
        .route("/api/incidents/:id", get(get_incident))
        .route("/api/incidents/:id", delete(delete_incident))
        // Lifecycle actions
        .route("/api/incidents/:id/acknowledge", post(acknowledge_incident))
        .route("/api/incidents/:id/resolve", post(resolve_incident))
        .route("/api/incidents/:id/close", post(close_incident))
        // Responders
        .route("/api/incidents/:id/responders", post(add_responder))
        // Timeline
        .route("/api/incidents/:id/timeline", get(get_timeline))
        .with_state(state)
}

// ── Health ───────────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-incidents",
        "status": "ok",
        "upstream": "Grafana OnCall"
    }))
}

// ── Incident CRUD ─────────────────────────────────────────────────────────────

async fn create_incident(
    AxumState(state): AxumState<Arc<State>>,
    Json(req): Json<CreateIncidentRequest>,
) -> impl IntoResponse {
    let incident = engine::create_incident(req);
    state.store.create(incident.clone());
    (StatusCode::CREATED, Json(incident))
}

async fn list_incidents(
    AxumState(state): AxumState<Arc<State>>,
) -> Json<serde_json::Value> {
    let list = state.store.list();
    Json(serde_json::to_value(list).unwrap())
}

async fn get_incident(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.store.get(id) {
        Some(i) => (StatusCode::OK, Json(serde_json::to_value(i).unwrap())).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "incident not found", "id": id})),
        )
            .into_response(),
    }
}

async fn delete_incident(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    if state.store.delete(id) {
        StatusCode::NO_CONTENT.into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "incident not found"})),
        )
            .into_response()
    }
}

// ── Lifecycle Actions ─────────────────────────────────────────────────────────

async fn acknowledge_incident(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
    Json(req): Json<AcknowledgeRequest>,
) -> impl IntoResponse {
    match state.store.get(id) {
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "incident not found"})),
        )
            .into_response(),
        Some(mut incident) => match engine::acknowledge(&mut incident, req.user_id) {
            Ok(()) => {
                state.store.update(incident.clone());
                (StatusCode::OK, Json(serde_json::to_value(incident).unwrap())).into_response()
            }
            Err(e) => (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response(),
        },
    }
}

async fn resolve_incident(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
    Json(req): Json<ResolveRequest>,
) -> impl IntoResponse {
    match state.store.get(id) {
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "incident not found"})),
        )
            .into_response(),
        Some(mut incident) => {
            match engine::resolve(&mut incident, req.user_id, req.resolution) {
                Ok(()) => {
                    state.store.update(incident.clone());
                    (StatusCode::OK, Json(serde_json::to_value(incident).unwrap())).into_response()
                }
                Err(e) => (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({"error": e.to_string()})),
                )
                    .into_response(),
            }
        }
    }
}

async fn close_incident(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
    Json(req): Json<CloseRequest>,
) -> impl IntoResponse {
    match state.store.get(id) {
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "incident not found"})),
        )
            .into_response(),
        Some(mut incident) => match engine::close(&mut incident, req.user_id) {
            Ok(()) => {
                state.store.update(incident.clone());
                (StatusCode::OK, Json(serde_json::to_value(incident).unwrap())).into_response()
            }
            Err(e) => (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response(),
        },
    }
}

// ── Responders ────────────────────────────────────────────────────────────────

async fn add_responder(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
    Json(req): Json<AddResponderRequest>,
) -> impl IntoResponse {
    let responder = Responder {
        user_id: req.user_id,
        name: req.name,
        email: req.email,
        role: req.role,
        paged_at: Utc::now(),
        acknowledged_at: None,
    };
    if state.store.add_responder(id, responder.clone()) {
        (StatusCode::CREATED, Json(serde_json::to_value(responder).unwrap())).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "incident not found"})),
        )
            .into_response()
    }
}

// ── Timeline ──────────────────────────────────────────────────────────────────

async fn get_timeline(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.store.get(id) {
        Some(i) => (StatusCode::OK, Json(serde_json::to_value(i.timeline).unwrap())).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "incident not found"})),
        )
            .into_response(),
    }
}

// ── Metrics ───────────────────────────────────────────────────────────────────

async fn get_metrics(
    AxumState(state): AxumState<Arc<State>>,
) -> Json<serde_json::Value> {
    let m = state.store.metrics();
    Json(serde_json::to_value(m).unwrap())
}

// ── PostMortems ───────────────────────────────────────────────────────────────

async fn create_postmortem(
    AxumState(state): AxumState<Arc<State>>,
    Json(req): Json<CreatePostMortemRequest>,
) -> impl IntoResponse {
    let pm = PostMortem {
        id: Uuid::new_v4(),
        incident_id: req.incident_id,
        title: req.title,
        summary: req.summary,
        root_cause: req.root_cause,
        action_items: req.action_items,
        status: crate::models::PostMortemStatus::Draft,
        created_at: Utc::now(),
        published_at: None,
        author_id: req.author_id,
    };
    state.store.create_postmortem(pm.clone());
    (StatusCode::CREATED, Json(serde_json::to_value(pm).unwrap()))
}

async fn list_postmortems(
    AxumState(state): AxumState<Arc<State>>,
) -> Json<serde_json::Value> {
    let list = state.store.list_postmortems();
    Json(serde_json::to_value(list).unwrap())
}

async fn get_postmortem(
    AxumState(state): AxumState<Arc<State>>,
    Path(pm_id): Path<Uuid>,
) -> impl IntoResponse {
    match state.store.get_postmortem(pm_id) {
        Some(pm) => (StatusCode::OK, Json(serde_json::to_value(pm).unwrap())).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "postmortem not found"})),
        )
            .into_response(),
    }
}
