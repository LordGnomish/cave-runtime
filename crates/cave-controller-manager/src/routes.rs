//! REST routes for cave-controller-manager.

use crate::controllers;
use crate::models::*;
use crate::state::ControllerManagerState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;

pub fn create_router(state: Arc<ControllerManagerState>) -> Router {
    Router::new()
        .route("/api/cm/health", get(health))
        .route("/api/cm/deployments", get(list_deployments).post(create_deployment))
        .route("/api/cm/deployments/{ns}/{name}/reconcile", post(reconcile_deployment_handler))
        .route("/api/cm/replicasets", get(list_replicasets))
        .route("/api/cm/replicasets/{uid}/reconcile", post(reconcile_replicaset_handler))
        .route("/api/cm/jobs", get(list_jobs).post(create_job))
        .route("/api/cm/jobs/{ns}/{name}/reconcile", post(reconcile_job_handler))
        .route("/api/cm/namespaces", get(list_namespaces).post(create_namespace))
        .route("/api/cm/namespaces/{name}/reconcile", post(reconcile_namespace_handler))
        .route("/api/cm/endpointslices", get(list_endpoint_slices))
        .route("/api/cm/endpointslices/{ns}/{name}/reconcile", post(reconcile_endpoint_slice_handler))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"module":"cave-controller-manager","status":"ok","upstream":"kube-controller-manager"}))
}

async fn list_deployments(State(s): State<Arc<ControllerManagerState>>) -> Json<Vec<Deployment>> {
    Json(s.deployments.iter().map(|r| r.value().clone()).collect())
}

async fn create_deployment(State(s): State<Arc<ControllerManagerState>>, Json(d): Json<Deployment>) -> (StatusCode, Json<Deployment>) {
    s.deployments.insert((d.namespace.clone(), d.name.clone()), d.clone());
    (StatusCode::CREATED, Json(d))
}

async fn reconcile_deployment_handler(
    State(s): State<Arc<ControllerManagerState>>,
    Path((ns, name)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    controllers::reconcile_deployment(&s, &ns, &name)
        .map(|_| StatusCode::OK)
        .ok_or((StatusCode::NOT_FOUND, "deployment not found".into()))
}

async fn list_replicasets(State(s): State<Arc<ControllerManagerState>>) -> Json<Vec<ReplicaSet>> {
    Json(s.replicasets.iter().map(|r| r.value().clone()).collect())
}

async fn reconcile_replicaset_handler(
    State(s): State<Arc<ControllerManagerState>>,
    Path(uid): Path<uuid::Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    controllers::reconcile_replicaset(&s, &uid)
        .map(|_| StatusCode::OK)
        .ok_or((StatusCode::NOT_FOUND, "replicaset not found".into()))
}

async fn list_jobs(State(s): State<Arc<ControllerManagerState>>) -> Json<Vec<Job>> {
    Json(s.jobs.iter().map(|r| r.value().clone()).collect())
}

async fn create_job(State(s): State<Arc<ControllerManagerState>>, Json(j): Json<Job>) -> (StatusCode, Json<Job>) {
    s.jobs.insert((j.namespace.clone(), j.name.clone()), j.clone());
    (StatusCode::CREATED, Json(j))
}

async fn reconcile_job_handler(
    State(s): State<Arc<ControllerManagerState>>,
    Path((ns, name)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    controllers::reconcile_job(&s, &ns, &name)
        .map(|_| StatusCode::OK)
        .ok_or((StatusCode::NOT_FOUND, "job not found".into()))
}

async fn list_namespaces(State(s): State<Arc<ControllerManagerState>>) -> Json<Vec<Namespace>> {
    Json(s.namespaces.iter().map(|r| r.value().clone()).collect())
}

async fn create_namespace(State(s): State<Arc<ControllerManagerState>>, Json(n): Json<Namespace>) -> (StatusCode, Json<Namespace>) {
    s.namespaces.insert(n.name.clone(), n.clone());
    (StatusCode::CREATED, Json(n))
}

async fn reconcile_namespace_handler(
    State(s): State<Arc<ControllerManagerState>>,
    Path(name): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    controllers::reconcile_namespace(&s, &name)
        .map(|_| StatusCode::OK)
        .ok_or((StatusCode::NOT_FOUND, "namespace not found".into()))
}

async fn list_endpoint_slices(State(s): State<Arc<ControllerManagerState>>) -> Json<Vec<EndpointSlice>> {
    Json(s.endpoint_slices.iter().map(|r| r.value().clone()).collect())
}

#[derive(serde::Deserialize)]
struct ReconcileEndpointSliceReq {
    addresses: Vec<String>,
}

async fn reconcile_endpoint_slice_handler(
    State(s): State<Arc<ControllerManagerState>>,
    Path((ns, name)): Path<(String, String)>,
    Json(req): Json<ReconcileEndpointSliceReq>,
) -> Result<StatusCode, (StatusCode, String)> {
    controllers::reconcile_endpoint_slice(&s, &ns, &name, req.addresses)
        .map(|_| StatusCode::OK)
        .ok_or((StatusCode::NOT_FOUND, "endpoint slice not found".into()))
}
