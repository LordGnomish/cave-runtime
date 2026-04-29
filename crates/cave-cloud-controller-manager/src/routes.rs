//! REST routes for cave-cloud-controller-manager.

use crate::controllers;
use crate::models::*;
use crate::state::CloudControllerState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;

pub fn create_router(state: Arc<CloudControllerState>) -> Router {
    Router::new()
        .route("/api/ccm/health", get(health))
        .route("/api/ccm/nodes", get(list_cloud_nodes))
        .route("/api/ccm/nodes/{name}/reconcile", post(reconcile_node_handler))
        .route("/api/ccm/loadbalancers", get(list_load_balancers))
        .route("/api/ccm/loadbalancers/{ns}/{name}/reconcile", post(reconcile_load_balancer_handler))
        .route("/api/ccm/loadbalancers/{ns}/{name}/delete", post(delete_load_balancer_handler))
        .route("/api/ccm/routes", get(list_routes).post(create_route))
        .route("/api/ccm/routes/{name}", post(delete_route))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"module":"cave-cloud-controller-manager","status":"ok","upstream":"cloud-controller-manager"}))
}

async fn list_cloud_nodes(State(s): State<Arc<CloudControllerState>>) -> Json<Vec<CloudNode>> {
    Json(s.nodes.iter().map(|r| r.value().clone()).collect())
}

async fn reconcile_node_handler(
    State(s): State<Arc<CloudControllerState>>,
    Path(name): Path<String>,
) -> StatusCode {
    if controllers::reconcile_node(&s, &name) {
        StatusCode::OK
    } else {
        StatusCode::NOT_MODIFIED
    }
}

async fn list_load_balancers(State(s): State<Arc<CloudControllerState>>) -> Json<Vec<LoadBalancer>> {
    Json(s.load_balancers.iter().map(|r| r.value().clone()).collect())
}

#[derive(serde::Deserialize)]
struct ReconcileLBReq { backends: Vec<String> }

async fn reconcile_load_balancer_handler(
    State(s): State<Arc<CloudControllerState>>,
    Path((ns, name)): Path<(String, String)>,
    Json(req): Json<ReconcileLBReq>,
) -> Json<LoadBalancerStatus> {
    Json(controllers::reconcile_service_load_balancer(&s, &ns, &name, req.backends))
}

async fn delete_load_balancer_handler(
    State(s): State<Arc<CloudControllerState>>,
    Path((ns, name)): Path<(String, String)>,
) -> StatusCode {
    if controllers::delete_service_load_balancer(&s, &ns, &name) {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

async fn list_routes(State(s): State<Arc<CloudControllerState>>) -> Json<Vec<CloudRoute>> {
    Json(s.routes.iter().map(|r| r.value().clone()).collect())
}

async fn create_route(
    State(s): State<Arc<CloudControllerState>>,
    Json(route): Json<CloudRoute>,
) -> StatusCode {
    if controllers::reconcile_route(&s, route) {
        StatusCode::CREATED
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

async fn delete_route(
    State(s): State<Arc<CloudControllerState>>,
    Path(name): Path<String>,
) -> StatusCode {
    if controllers::remove_route(&s, &name) {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}
