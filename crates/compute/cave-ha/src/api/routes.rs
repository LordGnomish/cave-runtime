// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP API for cluster management and observability.
//!
//! Endpoints:
//!   GET  /health          — liveness probe
//!   GET  /status          — node & cluster status
//!   POST /propose         — submit a proposal (for testing)
//!   POST /members         — add a cluster member
//!   DELETE /members/{id}  — remove a cluster member
//!   POST /transfer        — transfer leadership
//!   POST /snapshot        — trigger snapshot
//!   GET  /metrics         — Prometheus metrics
//!   GET  /dr/status       — DR replication status
//!   POST /dr/failback     — initiate failback

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use prometheus_client::{encoding::text::encode, registry::Registry};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::dr::DrStatus;
use crate::error::HaError;
use crate::raft::node::RaftHandle;
use crate::raft::types::{NodeId, NodeInfo};

pub struct ApiState {
    pub node: RaftHandle,
    pub metrics_registry: Arc<RwLock<Registry>>,
    pub dr_status: Option<Arc<RwLock<DrStatus>>>,
}

pub fn router(state: Arc<ApiState>) -> Router {
    Router::new()
        .route("/api/ha/health", get(health))
        .route("/api/ha/status", get(status))
        .route("/api/ha/propose", post(propose))
        .route("/api/ha/members", post(add_member))
        .route("/members/{id}", delete(remove_member))
        .route("/api/ha/transfer", post(transfer_leadership))
        .route("/api/ha/snapshot", post(trigger_snapshot))
        .route("/api/ha/metrics", get(metrics))
        .route("/api/ha/dr/status", get(dr_status))
        .with_state(state)
}

// ── Handlers ──────────────────────────────────────────────────────────────

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

async fn status(State(s): State<Arc<ApiState>>) -> impl IntoResponse {
    match s.node.status().await {
        Ok(st) => Ok(Json(st)),
        Err(e) => Err(api_error(e)),
    }
}

#[derive(Deserialize)]
struct ProposeBody {
    data: String,
}

#[derive(Serialize)]
struct ProposeReply {
    log_index: u64,
}

async fn propose(
    State(s): State<Arc<ApiState>>,
    Json(body): Json<ProposeBody>,
) -> impl IntoResponse {
    match s.node.propose(body.data.into_bytes()).await {
        Ok(idx) => Ok(Json(ProposeReply { log_index: idx })),
        Err(e) => Err(api_error(e)),
    }
}

#[derive(Deserialize)]
struct AddMemberBody {
    id: NodeId,
    addr: String,
    is_learner: bool,
}

async fn add_member(
    State(s): State<Arc<ApiState>>,
    Json(body): Json<AddMemberBody>,
) -> impl IntoResponse {
    let node = NodeInfo {
        id: body.id,
        addr: body.addr,
        is_learner: body.is_learner,
    };
    match s.node.add_node(node).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(e) => Err(api_error(e)),
    }
}

async fn remove_member(
    State(s): State<Arc<ApiState>>,
    Path(id): Path<NodeId>,
) -> impl IntoResponse {
    match s.node.remove_node(id).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(e) => Err(api_error(e)),
    }
}

#[derive(Deserialize)]
struct TransferBody {
    to: NodeId,
}

async fn transfer_leadership(
    State(s): State<Arc<ApiState>>,
    Json(body): Json<TransferBody>,
) -> impl IntoResponse {
    match s.node.transfer_leadership(body.to).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(e) => Err(api_error(e)),
    }
}

async fn trigger_snapshot(State(s): State<Arc<ApiState>>) -> impl IntoResponse {
    match s.node.trigger_snapshot().await {
        Ok(meta) => Ok(Json(meta)),
        Err(e) => Err(api_error(e)),
    }
}

async fn metrics(State(s): State<Arc<ApiState>>) -> impl IntoResponse {
    let registry = s.metrics_registry.read().await;
    let mut buf = String::new();
    if encode(&mut buf, &*registry).is_ok() {
        (
            StatusCode::OK,
            [("content-type", "text/plain; version=0.0.4")],
            buf,
        )
    } else {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            [("content-type", "text/plain; version=0.0.4")],
            "encode error".to_string(),
        )
    }
}

async fn dr_status(State(s): State<Arc<ApiState>>) -> impl IntoResponse {
    match &s.dr_status {
        Some(dr) => Ok(Json(dr.read().await.clone())),
        None => Err((StatusCode::NOT_FOUND, "DR not configured").into_response()),
    }
}

// ── Error mapping ─────────────────────────────────────────────────────────

fn api_error(e: HaError) -> Response {
    let status = match &e {
        HaError::NotLeader { .. } => StatusCode::SERVICE_UNAVAILABLE,
        HaError::NoQuorum => StatusCode::SERVICE_UNAVAILABLE,
        HaError::Timeout => StatusCode::GATEWAY_TIMEOUT,
        HaError::NodeNotFound(_) => StatusCode::NOT_FOUND,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, Json(serde_json::json!({ "error": e.to_string() }))).into_response()
}
