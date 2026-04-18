//! HTTP routes for cave-vcluster.

use crate::models::{CreateClusterRequest, VClusterStatus};
use crate::VClusterState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post, put},
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;

pub fn create_router(state: Arc<VClusterState>) -> Router {
    Router::new()
        .route("/api/v1/vclusters/{namespace}", get(list_clusters).post(create_cluster))
        .route("/api/v1/vclusters/{namespace}/{name}", get(get_cluster).delete(delete_cluster))
        .route("/api/v1/vclusters/{namespace}/{name}/status", put(update_status))
        .route("/api/v1/vclusters/{namespace}/{name}/kubeconfig", get(get_kubeconfig))
        .route("/api/v1/vclusters/{namespace}/quota", get(get_quota))
        .route("/api/v1/vclusters/{namespace}/{name}/sync", post(sync_resources))
        .with_state(state)
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<serde_json::Value>)>;

fn err(code: StatusCode, msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (code, Json(serde_json::json!({ "error": msg })))
}

async fn list_clusters(
    Path(namespace): Path<String>,
    State(state): State<Arc<VClusterState>>,
) -> Json<serde_json::Value> {
    let clusters = state.clusters.list(&namespace);
    Json(serde_json::json!({ "clusters": clusters, "count": clusters.len() }))
}

async fn create_cluster(
    Path(namespace): Path<String>,
    State(state): State<Arc<VClusterState>>,
    Json(mut req): Json<CreateClusterRequest>,
) -> ApiResult<serde_json::Value> {
    req.namespace = namespace.clone();
    let max = state.quota.get_max(&namespace);
    match state.clusters.create(req, max) {
        Ok(c) => Ok(Json(serde_json::json!({ "cluster": c }))),
        Err(e) => Err(err(StatusCode::from_u16(e.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), &e.to_string())),
    }
}

async fn get_cluster(
    Path((namespace, name)): Path<(String, String)>,
    State(state): State<Arc<VClusterState>>,
) -> ApiResult<serde_json::Value> {
    match state.clusters.get(&namespace, &name) {
        Ok(c) => Ok(Json(serde_json::json!({ "cluster": c }))),
        Err(e) => Err(err(StatusCode::NOT_FOUND, &e.to_string())),
    }
}

async fn delete_cluster(
    Path((namespace, name)): Path<(String, String)>,
    State(state): State<Arc<VClusterState>>,
) -> ApiResult<serde_json::Value> {
    state.syncer.delete_for_cluster(&name);
    match state.clusters.delete(&namespace, &name) {
        Ok(()) => Ok(Json(serde_json::json!({ "deleted": true, "name": name }))),
        Err(e) => Err(err(StatusCode::NOT_FOUND, &e.to_string())),
    }
}

#[derive(Deserialize)]
struct UpdateStatusReq { status: String }

async fn update_status(
    Path((namespace, name)): Path<(String, String)>,
    State(state): State<Arc<VClusterState>>,
    Json(req): Json<UpdateStatusReq>,
) -> ApiResult<serde_json::Value> {
    let status = match req.status.as_str() {
        "running" => VClusterStatus::Running,
        "suspended" => VClusterStatus::Suspended,
        "failed" => VClusterStatus::Failed,
        "provisioning" => VClusterStatus::Provisioning,
        _ => return Err(err(StatusCode::BAD_REQUEST, "unknown status")),
    };
    match state.clusters.update_status(&namespace, &name, status) {
        Ok(c) => Ok(Json(serde_json::json!({ "cluster": c }))),
        Err(e) => Err(err(StatusCode::NOT_FOUND, &e.to_string())),
    }
}

async fn get_kubeconfig(
    Path((namespace, name)): Path<(String, String)>,
    State(state): State<Arc<VClusterState>>,
) -> ApiResult<serde_json::Value> {
    match state.clusters.get(&namespace, &name) {
        Ok(c) => match c.kubeconfig {
            Some(kc) => Ok(Json(serde_json::json!({ "kubeconfig": kc }))),
            None => Err(err(StatusCode::NOT_FOUND, "kubeconfig not yet available")),
        },
        Err(e) => Err(err(StatusCode::NOT_FOUND, &e.to_string())),
    }
}

async fn get_quota(
    Path(namespace): Path<String>,
    State(state): State<Arc<VClusterState>>,
) -> Json<serde_json::Value> {
    let current = state.clusters.count_in_namespace(&namespace);
    let quota = state.quota.status(&namespace, current);
    Json(serde_json::json!({ "quota": quota }))
}

#[derive(Deserialize)]
struct SyncReq { kind: String, name: String, data: String }

async fn sync_resources(
    Path((namespace, cluster_name)): Path<(String, String)>,
    State(state): State<Arc<VClusterState>>,
    Json(req): Json<SyncReq>,
) -> Json<serde_json::Value> {
    let resource = state.syncer.sync(&cluster_name, &namespace, &req.kind, &req.name, &req.data);
    Json(serde_json::json!({ "synced": resource }))
}
