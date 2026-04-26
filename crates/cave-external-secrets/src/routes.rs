//! HTTP routes for cave-external-secrets.

use crate::models::*;
use crate::ExternalSecretsState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;

pub fn create_router(state: Arc<ExternalSecretsState>) -> Router {
    Router::new()
        // SecretStore (namespaced)
        .route("/api/v1/external-secrets/{namespace}/secretstores", get(list_secret_stores).post(create_secret_store))
        .route("/api/v1/external-secrets/{namespace}/secretstores/{name}", get(get_secret_store).delete(delete_secret_store))
        // ClusterSecretStore
        .route("/api/v1/external-secrets/clustersecretstores", get(list_cluster_secret_stores).post(create_cluster_secret_store))
        .route("/api/v1/external-secrets/clustersecretstores/{name}", get(get_cluster_secret_store).delete(delete_cluster_secret_store))
        // ExternalSecret
        .route("/api/v1/external-secrets/{namespace}/externalsecrets", get(list_external_secrets).post(create_external_secret))
        .route("/api/v1/external-secrets/{namespace}/externalsecrets/{name}", get(get_external_secret).delete(delete_external_secret))
        .route("/api/v1/external-secrets/{namespace}/externalsecrets/{name}/sync", post(sync_secret))
        // PushSecret
        .route("/api/v1/external-secrets/{namespace}/pushsecrets", get(list_push_secrets).post(create_push_secret))
        .route("/api/v1/external-secrets/{namespace}/pushsecrets/{name}", get(get_push_secret).delete(delete_push_secret))
        .with_state(state)
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<serde_json::Value>)>;

fn err(code: StatusCode, msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (code, Json(serde_json::json!({ "error": msg })))
}
fn map_err(e: crate::error::EsoError) -> (StatusCode, Json<serde_json::Value>) {
    err(StatusCode::from_u16(e.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), &e.to_string())
}

async fn list_secret_stores(Path(namespace): Path<String>, State(state): State<Arc<ExternalSecretsState>>) -> Json<serde_json::Value> {
    let stores = state.secret_stores.list(Some(&namespace));
    Json(serde_json::json!({ "secret_stores": stores }))
}
async fn create_secret_store(Path(namespace): Path<String>, State(state): State<Arc<ExternalSecretsState>>, Json(mut req): Json<CreateSecretStoreRequest>) -> ApiResult<serde_json::Value> {
    req.namespace = Some(namespace); req.scope = Some(SecretStoreScope::Namespaced);
    state.secret_stores.create(req).map(|s| Json(serde_json::json!({ "secret_store": s }))).map_err(map_err)
}
async fn get_secret_store(Path((namespace, name)): Path<(String, String)>, State(state): State<Arc<ExternalSecretsState>>) -> ApiResult<serde_json::Value> {
    state.secret_stores.get(Some(&namespace), &name).map(|s| Json(serde_json::json!({ "secret_store": s }))).map_err(map_err)
}
async fn delete_secret_store(Path((namespace, name)): Path<(String, String)>, State(state): State<Arc<ExternalSecretsState>>) -> ApiResult<serde_json::Value> {
    state.secret_stores.delete(Some(&namespace), &name).map(|_| Json(serde_json::json!({ "deleted": true }))).map_err(map_err)
}

async fn list_cluster_secret_stores(State(state): State<Arc<ExternalSecretsState>>) -> Json<serde_json::Value> {
    let stores = state.secret_stores.list(None);
    Json(serde_json::json!({ "cluster_secret_stores": stores }))
}
async fn create_cluster_secret_store(State(state): State<Arc<ExternalSecretsState>>, Json(mut req): Json<CreateSecretStoreRequest>) -> ApiResult<serde_json::Value> {
    req.namespace = None; req.scope = Some(SecretStoreScope::Cluster);
    state.secret_stores.create(req).map(|s| Json(serde_json::json!({ "secret_store": s }))).map_err(map_err)
}
async fn get_cluster_secret_store(Path(name): Path<String>, State(state): State<Arc<ExternalSecretsState>>) -> ApiResult<serde_json::Value> {
    state.secret_stores.get(None, &name).map(|s| Json(serde_json::json!({ "secret_store": s }))).map_err(map_err)
}
async fn delete_cluster_secret_store(Path(name): Path<String>, State(state): State<Arc<ExternalSecretsState>>) -> ApiResult<serde_json::Value> {
    state.secret_stores.delete(None, &name).map(|_| Json(serde_json::json!({ "deleted": true }))).map_err(map_err)
}

async fn list_external_secrets(Path(namespace): Path<String>, State(state): State<Arc<ExternalSecretsState>>) -> Json<serde_json::Value> {
    let secrets = state.external_secrets.list(&namespace);
    Json(serde_json::json!({ "external_secrets": secrets }))
}
async fn create_external_secret(Path(namespace): Path<String>, State(state): State<Arc<ExternalSecretsState>>, Json(mut req): Json<CreateExternalSecretRequest>) -> ApiResult<serde_json::Value> {
    req.namespace = namespace;
    state.external_secrets.create(req).map(|s| Json(serde_json::json!({ "external_secret": s }))).map_err(map_err)
}
async fn get_external_secret(Path((namespace, name)): Path<(String, String)>, State(state): State<Arc<ExternalSecretsState>>) -> ApiResult<serde_json::Value> {
    state.external_secrets.get(&namespace, &name).map(|s| Json(serde_json::json!({ "external_secret": s }))).map_err(map_err)
}
async fn delete_external_secret(Path((namespace, name)): Path<(String, String)>, State(state): State<Arc<ExternalSecretsState>>) -> ApiResult<serde_json::Value> {
    state.external_secrets.delete(&namespace, &name).map(|_| Json(serde_json::json!({ "deleted": true }))).map_err(map_err)
}
async fn sync_secret(Path((namespace, name)): Path<(String, String)>, State(state): State<Arc<ExternalSecretsState>>) -> ApiResult<serde_json::Value> {
    state.external_secrets.simulate_sync(&namespace, &name).map(|r| Json(serde_json::json!({ "sync_result": r }))).map_err(map_err)
}

async fn list_push_secrets(Path(namespace): Path<String>, State(state): State<Arc<ExternalSecretsState>>) -> Json<serde_json::Value> {
    let secrets = state.push_secrets.list(&namespace);
    Json(serde_json::json!({ "push_secrets": secrets }))
}
async fn create_push_secret(Path(namespace): Path<String>, State(state): State<Arc<ExternalSecretsState>>, Json(mut req): Json<CreatePushSecretRequest>) -> ApiResult<serde_json::Value> {
    req.namespace = namespace;
    state.push_secrets.create(req).map(|s| Json(serde_json::json!({ "push_secret": s }))).map_err(map_err)
}
async fn get_push_secret(Path((namespace, name)): Path<(String, String)>, State(state): State<Arc<ExternalSecretsState>>) -> ApiResult<serde_json::Value> {
    state.push_secrets.get(&namespace, &name).map(|s| Json(serde_json::json!({ "push_secret": s }))).map_err(map_err)
}
async fn delete_push_secret(Path((namespace, name)): Path<(String, String)>, State(state): State<Arc<ExternalSecretsState>>) -> ApiResult<serde_json::Value> {
    state.push_secrets.delete(&namespace, &name).map(|_| Json(serde_json::json!({ "deleted": true }))).map_err(map_err)
}
