// SPDX-License-Identifier: AGPL-3.0-or-later
//! REST API routes for the kubelet.

use crate::agent::{self, KubeletState};
use crate::models::*;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router(state: Arc<KubeletState>) -> Router {
    Router::new()
        .route("/api/kubelet/health", get(health))
        .route("/api/kubelet/status", get(node_status))
        .route("/api/kubelet/pods", get(list_pods).post(assign_pod))
        .route("/api/kubelet/pods/{uid}/start", post(start_pod))
        .route("/api/kubelet/pods/{uid}/stop", post(stop_pod))
        .route("/api/kubelet/pods/{uid}", delete(remove_pod))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"module":"cave-kubelet","status":"ok","upstream":"kubelet"}))
}

async fn node_status(State(s): State<Arc<KubeletState>>) -> Json<NodeStatusReport> {
    Json(agent::node_status(&s))
}

async fn list_pods(State(s): State<Arc<KubeletState>>) -> Json<Vec<ManagedPod>> {
    let pods: Vec<ManagedPod> = s.pods.iter().map(|r| r.value().clone()).collect();
    Json(pods)
}

#[derive(Deserialize)]
struct AssignPodReq {
    name: String,
    namespace: String,
    containers: Vec<(String, String)>,
}

async fn assign_pod(State(s): State<Arc<KubeletState>>, Json(req): Json<AssignPodReq>) -> (StatusCode, Json<ManagedPod>) {
    let pod = agent::assign_pod(&s, &req.name, &req.namespace, req.containers);
    (StatusCode::CREATED, Json(pod))
}

async fn start_pod(State(s): State<Arc<KubeletState>>, Path(uid): Path<Uuid>) -> Result<Json<ManagedPod>, (StatusCode, String)> {
    agent::start_pod(&s, &uid).map(Json).ok_or((StatusCode::NOT_FOUND, "pod not found".into()))
}

async fn stop_pod(State(s): State<Arc<KubeletState>>, Path(uid): Path<Uuid>) -> Result<Json<ManagedPod>, (StatusCode, String)> {
    agent::stop_pod(&s, &uid).map(Json).ok_or((StatusCode::NOT_FOUND, "pod not found".into()))
}

async fn remove_pod(State(s): State<Arc<KubeletState>>, Path(uid): Path<Uuid>) -> Result<StatusCode, (StatusCode, String)> {
    agent::remove_pod(&s, &uid).map(|_| StatusCode::OK).ok_or((StatusCode::NOT_FOUND, "pod not found".into()))
}
