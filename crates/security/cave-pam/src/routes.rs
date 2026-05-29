// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP routes for cave-pam.
//!
//! Exposes the PAM API surface: sessions, access requests, node inventory,
//! and audit log queries.

use crate::State;
use axum::{
    Json, Router,
    extract::{Path, State as AxumState},
    http::StatusCode,
    routing::{get, post},
};
use std::sync::Arc;

pub fn create_router(state: Arc<State>) -> Router {
    Router::new()
        .route("/api/pam/health", get(health))
        .route("/api/pam/sessions", get(list_sessions))
        .route("/api/pam/sessions/:id", get(get_session))
        .route("/api/pam/requests", get(list_requests).post(create_request))
        .route("/api/pam/requests/:id/decide", post(decide_request))
        .route("/api/pam/nodes", get(list_nodes))
        .route("/api/pam/nodes/:id", get(get_node))
        .route("/api/pam/audit", get(query_audit))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-pam",
        "status": "ok",
        "upstream": "Teleport CE"
    }))
}

/// GET /api/pam/sessions — list active PAM sessions
async fn list_sessions(AxumState(_state): AxumState<Arc<State>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "sessions": [],
        "total": 0
    }))
}

/// GET /api/pam/sessions/:id — get session details
async fn get_session(
    AxumState(_state): AxumState<Arc<State>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // In full implementation this queries the session store by UUID.
    let _ = id;
    Err(StatusCode::NOT_FOUND)
}

/// GET /api/pam/requests — list pending access requests
async fn list_requests(AxumState(_state): AxumState<Arc<State>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "requests": [],
        "pending": 0
    }))
}

/// POST /api/pam/requests — submit a new access request
async fn create_request(
    AxumState(_state): AxumState<Arc<State>>,
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    // In full implementation this validates the body and calls
    // AccessRequestStore::create.
    Json(serde_json::json!({
        "status": "accepted",
        "request_id": null,
        "body": body
    }))
}

/// POST /api/pam/requests/:id/decide — approve or deny a request
async fn decide_request(
    AxumState(_state): AxumState<Arc<State>>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "request_id": id,
        "decision": body.get("decision").cloned().unwrap_or(serde_json::json!("unknown"))
    }))
}

/// GET /api/pam/nodes — list enrolled nodes
async fn list_nodes(AxumState(_state): AxumState<Arc<State>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "nodes": [],
        "total": 0
    }))
}

/// GET /api/pam/nodes/:id — get node details
async fn get_node(
    AxumState(_state): AxumState<Arc<State>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let _ = id;
    Err(StatusCode::NOT_FOUND)
}

/// GET /api/pam/audit — query the audit log
async fn query_audit(AxumState(_state): AxumState<Arc<State>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "events": [],
        "total": 0
    }))
}
