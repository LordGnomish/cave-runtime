//! REST API routes for the scheduler.

use crate::models::*;
use crate::scheduler::{self, SchedulerState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;

pub fn create_router(state: Arc<SchedulerState>) -> Router {
    Router::new()
        .route("/api/scheduler/health", get(health))
        .route("/api/scheduler/nodes", get(list_nodes).post(register_node))
        .route("/api/scheduler/nodes/{name}", get(get_node).delete(unregister_node))
        .route("/api/scheduler/nodes/{name}/cordon", post(cordon_node))
        .route("/api/scheduler/nodes/{name}/uncordon", post(uncordon_node))
        .route("/api/scheduler/schedule", post(schedule_pod))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"module":"cave-scheduler","status":"ok","upstream":"kube-scheduler"}))
}

async fn list_nodes(State(s): State<Arc<SchedulerState>>) -> Json<Vec<Node>> {
    let nodes: Vec<Node> = s.nodes.iter().map(|r| r.value().clone()).collect();
    Json(nodes)
}

async fn register_node(State(s): State<Arc<SchedulerState>>, Json(node): Json<Node>) -> (StatusCode, Json<Node>) {
    s.nodes.insert(node.name.clone(), node.clone());
    tracing::info!(node = %node.name, "node registered");
    (StatusCode::CREATED, Json(node))
}

async fn get_node(State(s): State<Arc<SchedulerState>>, Path(name): Path<String>) -> Result<Json<Node>, (StatusCode, String)> {
    s.nodes.get(&name).map(|r| Json(r.value().clone())).ok_or((StatusCode::NOT_FOUND, format!("node {} not found", name)))
}

async fn unregister_node(State(s): State<Arc<SchedulerState>>, Path(name): Path<String>) -> Result<StatusCode, (StatusCode, String)> {
    s.nodes.remove(&name).map(|_| StatusCode::OK).ok_or((StatusCode::NOT_FOUND, format!("node {} not found", name)))
}

async fn cordon_node(State(s): State<Arc<SchedulerState>>, Path(name): Path<String>) -> Result<StatusCode, (StatusCode, String)> {
    s.nodes.get_mut(&name).map(|mut n| { n.status = NodeStatus::Cordoned; StatusCode::OK }).ok_or((StatusCode::NOT_FOUND, "not found".into()))
}

async fn uncordon_node(State(s): State<Arc<SchedulerState>>, Path(name): Path<String>) -> Result<StatusCode, (StatusCode, String)> {
    s.nodes.get_mut(&name).map(|mut n| { n.status = NodeStatus::Ready; StatusCode::OK }).ok_or((StatusCode::NOT_FOUND, "not found".into()))
}

async fn schedule_pod(State(s): State<Arc<SchedulerState>>, Json(req): Json<ScheduleRequest>) -> Json<ScheduleResult> {
    Json(scheduler::schedule(&req, &s))
}
