//! K8s-compatible API routes.
//!
//! Implements /api/v1 and /apis paths for core resource CRUD.

use crate::resources::*;
use crate::store::ResourceStore;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use std::sync::Arc;

pub fn create_router(state: Arc<ResourceStore>) -> Router {
    Router::new()
        .route("/api/apiserver/health", get(health))
        // Namespaces
        .route("/api/v1/namespaces", get(list_namespaces).post(create_namespace))
        .route("/api/v1/namespaces/{name}", get(get_namespace).delete(delete_namespace))
        // Pods
        .route("/api/v1/namespaces/{ns}/pods", get(list_pods).post(create_pod))
        .route("/api/v1/namespaces/{ns}/pods/{name}", get(get_pod).delete(delete_pod))
        // Services
        .route("/api/v1/namespaces/{ns}/services", get(list_services).post(create_service))
        .route("/api/v1/namespaces/{ns}/services/{name}", get(get_service).delete(delete_service))
        // ConfigMaps
        .route("/api/v1/namespaces/{ns}/configmaps", get(list_configmaps).post(create_configmap))
        .route("/api/v1/namespaces/{ns}/configmaps/{name}", get(get_configmap).delete(delete_configmap))
        // Secrets
        .route("/api/v1/namespaces/{ns}/secrets", get(list_secrets).post(create_secret))
        .route("/api/v1/namespaces/{ns}/secrets/{name}", get(get_secret).delete(delete_secret))
        // Deployments
        .route("/apis/apps/v1/namespaces/{ns}/deployments", get(list_deployments).post(create_deployment))
        .route("/apis/apps/v1/namespaces/{ns}/deployments/{name}", get(get_deployment).delete(delete_deployment))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"module":"cave-apiserver","status":"ok","upstream":"kube-apiserver"}))
}

// --- Namespace ---
async fn list_namespaces(State(s): State<Arc<ResourceStore>>) -> Json<serde_json::Value> {
    let items = s.list("Namespace", "");
    Json(serde_json::json!({"kind":"NamespaceList","items":items}))
}
async fn create_namespace(State(s): State<Arc<ResourceStore>>, Json(ns): Json<Namespace>) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    let r = Resource::Namespace(ns);
    s.create(r).map(|r| (StatusCode::CREATED, Json(r))).map_err(|e| (StatusCode::CONFLICT, e.to_string()))
}
async fn get_namespace(State(s): State<Arc<ResourceStore>>, Path(name): Path<String>) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("Namespace", "", &name).map(Json).map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}
async fn delete_namespace(State(s): State<Arc<ResourceStore>>, Path(name): Path<String>) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("Namespace", "", &name).map(|_| StatusCode::OK).map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

// --- Pods ---
async fn list_pods(State(s): State<Arc<ResourceStore>>, Path(ns): Path<String>) -> Json<serde_json::Value> {
    Json(serde_json::json!({"kind":"PodList","items":s.list("Pod", &ns)}))
}
async fn create_pod(State(s): State<Arc<ResourceStore>>, Path(ns): Path<String>, Json(mut pod): Json<Pod>) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    pod.metadata.namespace = ns;
    s.create(Resource::Pod(pod)).map(|r| (StatusCode::CREATED, Json(r))).map_err(|e| (StatusCode::CONFLICT, e.to_string()))
}
async fn get_pod(State(s): State<Arc<ResourceStore>>, Path((ns, name)): Path<(String, String)>) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("Pod", &ns, &name).map(Json).map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}
async fn delete_pod(State(s): State<Arc<ResourceStore>>, Path((ns, name)): Path<(String, String)>) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("Pod", &ns, &name).map(|_| StatusCode::OK).map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

// --- Services ---
async fn list_services(State(s): State<Arc<ResourceStore>>, Path(ns): Path<String>) -> Json<serde_json::Value> {
    Json(serde_json::json!({"kind":"ServiceList","items":s.list("Service", &ns)}))
}
async fn create_service(State(s): State<Arc<ResourceStore>>, Path(ns): Path<String>, Json(mut svc): Json<Service>) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    svc.metadata.namespace = ns;
    s.create(Resource::Service(svc)).map(|r| (StatusCode::CREATED, Json(r))).map_err(|e| (StatusCode::CONFLICT, e.to_string()))
}
async fn get_service(State(s): State<Arc<ResourceStore>>, Path((ns, name)): Path<(String, String)>) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("Service", &ns, &name).map(Json).map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}
async fn delete_service(State(s): State<Arc<ResourceStore>>, Path((ns, name)): Path<(String, String)>) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("Service", &ns, &name).map(|_| StatusCode::OK).map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

// --- ConfigMaps ---
async fn list_configmaps(State(s): State<Arc<ResourceStore>>, Path(ns): Path<String>) -> Json<serde_json::Value> {
    Json(serde_json::json!({"kind":"ConfigMapList","items":s.list("ConfigMap", &ns)}))
}
async fn create_configmap(State(s): State<Arc<ResourceStore>>, Path(ns): Path<String>, Json(mut cm): Json<ConfigMap>) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    cm.metadata.namespace = ns;
    s.create(Resource::ConfigMap(cm)).map(|r| (StatusCode::CREATED, Json(r))).map_err(|e| (StatusCode::CONFLICT, e.to_string()))
}
async fn get_configmap(State(s): State<Arc<ResourceStore>>, Path((ns, name)): Path<(String, String)>) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("ConfigMap", &ns, &name).map(Json).map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}
async fn delete_configmap(State(s): State<Arc<ResourceStore>>, Path((ns, name)): Path<(String, String)>) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("ConfigMap", &ns, &name).map(|_| StatusCode::OK).map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

// --- Secrets ---
async fn list_secrets(State(s): State<Arc<ResourceStore>>, Path(ns): Path<String>) -> Json<serde_json::Value> {
    Json(serde_json::json!({"kind":"SecretList","items":s.list("Secret", &ns)}))
}
async fn create_secret(State(s): State<Arc<ResourceStore>>, Path(ns): Path<String>, Json(mut sec): Json<Secret>) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    sec.metadata.namespace = ns;
    s.create(Resource::Secret(sec)).map(|r| (StatusCode::CREATED, Json(r))).map_err(|e| (StatusCode::CONFLICT, e.to_string()))
}
async fn get_secret(State(s): State<Arc<ResourceStore>>, Path((ns, name)): Path<(String, String)>) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("Secret", &ns, &name).map(Json).map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}
async fn delete_secret(State(s): State<Arc<ResourceStore>>, Path((ns, name)): Path<(String, String)>) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("Secret", &ns, &name).map(|_| StatusCode::OK).map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

// --- Deployments ---
async fn list_deployments(State(s): State<Arc<ResourceStore>>, Path(ns): Path<String>) -> Json<serde_json::Value> {
    Json(serde_json::json!({"kind":"DeploymentList","items":s.list("Deployment", &ns)}))
}
async fn create_deployment(State(s): State<Arc<ResourceStore>>, Path(ns): Path<String>, Json(mut dep): Json<Deployment>) -> Result<(StatusCode, Json<Resource>), (StatusCode, String)> {
    dep.metadata.namespace = ns;
    s.create(Resource::Deployment(dep)).map(|r| (StatusCode::CREATED, Json(r))).map_err(|e| (StatusCode::CONFLICT, e.to_string()))
}
async fn get_deployment(State(s): State<Arc<ResourceStore>>, Path((ns, name)): Path<(String, String)>) -> Result<Json<Resource>, (StatusCode, String)> {
    s.get("Deployment", &ns, &name).map(Json).map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}
async fn delete_deployment(State(s): State<Arc<ResourceStore>>, Path((ns, name)): Path<(String, String)>) -> Result<StatusCode, (StatusCode, String)> {
    s.delete("Deployment", &ns, &name).map(|_| StatusCode::OK).map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}
