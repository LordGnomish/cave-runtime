//! HTTP routes for cave-keda.

use crate::models::*;
use crate::KedaState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;

pub fn create_router(state: Arc<KedaState>) -> Router {
    Router::new()
        // ScaledObjects
        .route("/api/v1/keda/{namespace}/scaledobjects", get(list_scaled_objects).post(create_scaled_object))
        .route("/api/v1/keda/{namespace}/scaledobjects/{name}", get(get_scaled_object).delete(delete_scaled_object))
        .route("/api/v1/keda/{namespace}/scaledobjects/{name}/scale", post(scale_object))
        .route("/api/v1/keda/{namespace}/scaledobjects/{name}/metrics", get(get_metrics).post(record_metrics))
        // ScaledJobs
        .route("/api/v1/keda/{namespace}/scaledjobs", get(list_scaled_jobs).post(create_scaled_job))
        .route("/api/v1/keda/{namespace}/scaledjobs/{name}", get(get_scaled_job).delete(delete_scaled_job))
        // TriggerAuthentication
        .route("/api/v1/keda/{namespace}/triggerauths", get(list_trigger_auths).post(create_trigger_auth))
        .route("/api/v1/keda/{namespace}/triggerauths/{name}", get(get_trigger_auth).delete(delete_trigger_auth))
        .with_state(state)
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<serde_json::Value>)>;

fn err(code: StatusCode, msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (code, Json(serde_json::json!({ "error": msg })))
}

fn map_err(e: crate::error::KedaError) -> (StatusCode, Json<serde_json::Value>) {
    err(StatusCode::from_u16(e.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), &e.to_string())
}

async fn list_scaled_objects(Path(namespace): Path<String>, State(state): State<Arc<KedaState>>) -> Json<serde_json::Value> {
    let objects = state.scaled_objects.list(&namespace);
    Json(serde_json::json!({ "scaled_objects": objects, "count": objects.len() }))
}

async fn create_scaled_object(
    Path(namespace): Path<String>,
    State(state): State<Arc<KedaState>>,
    Json(mut req): Json<CreateScaledObjectRequest>,
) -> ApiResult<serde_json::Value> {
    req.namespace = namespace;
    state.scaled_objects.create(req).map(|o| Json(serde_json::json!({ "scaled_object": o }))).map_err(map_err)
}

async fn get_scaled_object(Path((namespace, name)): Path<(String, String)>, State(state): State<Arc<KedaState>>) -> ApiResult<serde_json::Value> {
    state.scaled_objects.get(&namespace, &name).map(|o| Json(serde_json::json!({ "scaled_object": o }))).map_err(map_err)
}

async fn delete_scaled_object(Path((namespace, name)): Path<(String, String)>, State(state): State<Arc<KedaState>>) -> ApiResult<serde_json::Value> {
    state.scaled_objects.delete(&namespace, &name).map(|_| Json(serde_json::json!({ "deleted": true }))).map_err(map_err)
}

async fn scale_object(
    Path((namespace, name)): Path<(String, String)>,
    State(state): State<Arc<KedaState>>,
    Json(req): Json<ScaleRequest>,
) -> ApiResult<serde_json::Value> {
    if state.cooldown.is_in_cooldown(&namespace, &name, 300) {
        let remaining = state.cooldown.remaining_secs(&namespace, &name, 300);
        return Err(err(StatusCode::TOO_MANY_REQUESTS, &format!("in cooldown, {remaining}s remaining")));
    }
    state.scaled_objects.scale(&namespace, &name, req.desired_replicas)
        .map(|o| { state.cooldown.record_scale(&namespace, &name); Json(serde_json::json!({ "scaled_object": o })) })
        .map_err(map_err)
}

async fn get_metrics(Path((namespace, name)): Path<(String, String)>, State(state): State<Arc<KedaState>>) -> Json<serde_json::Value> {
    let metrics = state.scaled_objects.get_metrics(&namespace, &name);
    Json(serde_json::json!({ "metrics": metrics }))
}

async fn record_metrics(
    Path((namespace, name)): Path<(String, String)>,
    State(state): State<Arc<KedaState>>,
    Json(metrics): Json<Vec<MetricValue>>,
) -> Json<serde_json::Value> {
    state.scaled_objects.record_metrics(&namespace, &name, metrics);
    Json(serde_json::json!({ "ok": true }))
}

async fn list_scaled_jobs(Path(namespace): Path<String>, State(state): State<Arc<KedaState>>) -> Json<serde_json::Value> {
    let jobs = state.scaled_jobs.list(&namespace);
    Json(serde_json::json!({ "scaled_jobs": jobs, "count": jobs.len() }))
}

async fn create_scaled_job(
    Path(namespace): Path<String>,
    State(state): State<Arc<KedaState>>,
    Json(mut req): Json<CreateScaledJobRequest>,
) -> ApiResult<serde_json::Value> {
    req.namespace = namespace;
    state.scaled_jobs.create(req).map(|j| Json(serde_json::json!({ "scaled_job": j }))).map_err(map_err)
}

async fn get_scaled_job(Path((namespace, name)): Path<(String, String)>, State(state): State<Arc<KedaState>>) -> ApiResult<serde_json::Value> {
    state.scaled_jobs.get(&namespace, &name).map(|j| Json(serde_json::json!({ "scaled_job": j }))).map_err(map_err)
}

async fn delete_scaled_job(Path((namespace, name)): Path<(String, String)>, State(state): State<Arc<KedaState>>) -> ApiResult<serde_json::Value> {
    state.scaled_jobs.delete(&namespace, &name).map(|_| Json(serde_json::json!({ "deleted": true }))).map_err(map_err)
}

async fn list_trigger_auths(Path(namespace): Path<String>, State(state): State<Arc<KedaState>>) -> Json<serde_json::Value> {
    let auths = state.trigger_auths.list(&namespace);
    Json(serde_json::json!({ "trigger_auths": auths, "count": auths.len() }))
}

async fn create_trigger_auth(
    Path(namespace): Path<String>,
    State(state): State<Arc<KedaState>>,
    Json(mut req): Json<CreateTriggerAuthRequest>,
) -> ApiResult<serde_json::Value> {
    req.namespace = namespace;
    state.trigger_auths.create(req).map(|a| Json(serde_json::json!({ "trigger_auth": a }))).map_err(map_err)
}

async fn get_trigger_auth(Path((namespace, name)): Path<(String, String)>, State(state): State<Arc<KedaState>>) -> ApiResult<serde_json::Value> {
    state.trigger_auths.get(&namespace, &name).map(|a| Json(serde_json::json!({ "trigger_auth": a }))).map_err(map_err)
}

async fn delete_trigger_auth(Path((namespace, name)): Path<(String, String)>, State(state): State<Arc<KedaState>>) -> ApiResult<serde_json::Value> {
    state.trigger_auths.delete(&namespace, &name).map(|_| Json(serde_json::json!({ "deleted": true }))).map_err(map_err)
}
