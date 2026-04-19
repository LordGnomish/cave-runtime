//! REST API routes for the container runtime.

use crate::models::*;
use crate::registry::RegistryClient;
use crate::runtime;
use crate::store::{ContainerStore, ImageStore};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

pub struct CriState {
    pub containers: ContainerStore,
    pub images: ImageStore,
    pub registry: RegistryClient,
}

pub fn create_router(state: Arc<CriState>) -> Router {
    Router::new()
        .route("/api/cri/health", get(health))
        .route("/api/cri/containers", get(list_containers).post(create_container))
        .route("/api/cri/containers/{id}", get(inspect_container).delete(delete_container))
        .route("/api/cri/containers/{id}/start", post(start_container))
        .route("/api/cri/containers/{id}/stop", post(stop_container))
        .route("/api/cri/containers/{id}/kill", post(kill_container))
        .route("/api/cri/images", get(list_images))
        .route("/api/cri/images/pull", post(pull_image))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-cri",
        "status": "ok",
        "upstream": "containerd/crun"
    }))
}

async fn list_containers(State(state): State<Arc<CriState>>) -> Json<Vec<Container>> {
    Json(runtime::list_containers(&state.containers))
}

#[derive(Deserialize)]
struct CreateContainerReq {
    spec: ContainerSpec,
}

async fn create_container(
    State(state): State<Arc<CriState>>,
    Json(req): Json<CreateContainerReq>,
) -> Result<(StatusCode, Json<Container>), (StatusCode, String)> {
    // Check if image is available locally, pull if not
    let image = match state.images.get(&req.spec.image) {
        Some(img) => img,
        None => {
            // Auto-pull image
            match state.registry.pull_image(&req.spec.image).await {
                Ok(img) => {
                    state.images.insert(img.clone());
                    img
                }
                Err(e) => return Err((StatusCode::BAD_REQUEST, format!("image pull failed: {}", e))),
            }
        }
    };

    match runtime::create_container(req.spec, &image, &state.containers).await {
        Ok(c) => Ok((StatusCode::CREATED, Json(c))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

async fn inspect_container(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Container>, (StatusCode, String)> {
    runtime::inspect_container(id, &state.containers)
        .map(Json)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

async fn start_container(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    runtime::start_container(id, &state.containers)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct StopReq {
    #[serde(default = "default_timeout")]
    timeout: u32,
}
#[allow(dead_code)]
fn default_timeout() -> u32 { 10 }

async fn stop_container(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    runtime::stop_container(id, 10, &state.containers)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct KillReq {
    #[serde(default = "default_signal")]
    signal: i32,
}
#[allow(dead_code)]
fn default_signal() -> i32 { 15 } // SIGTERM

async fn kill_container(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    runtime::kill_container(id, 15, &state.containers)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn delete_container(
    State(state): State<Arc<CriState>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    runtime::delete_container(id, &state.containers)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn list_images(State(state): State<Arc<CriState>>) -> Json<Vec<OciImage>> {
    Json(state.images.list())
}

#[derive(Deserialize)]
struct PullImageReq {
    reference: String,
}

async fn pull_image(
    State(state): State<Arc<CriState>>,
    Json(req): Json<PullImageReq>,
) -> Result<(StatusCode, Json<OciImage>), (StatusCode, String)> {
    match state.registry.pull_image(&req.reference).await {
        Ok(image) => {
            state.images.insert(image.clone());
            Ok((StatusCode::OK, Json(image)))
        }
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Container, ContainerSpec, ContainerStatus, ImageConfig, NetworkMode, OciImage, RestartPolicy};
    use crate::registry::RegistryClient;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use chrono::Utc;
    use tower::ServiceExt;

    fn test_state() -> Arc<CriState> {
        Arc::new(CriState {
            containers: crate::store::ContainerStore::new(),
            images: crate::store::ImageStore::new(),
            registry: RegistryClient::new(std::path::PathBuf::from("/tmp/cave-test-registry")),
        })
    }

    fn insert_container(state: &Arc<CriState>, status: ContainerStatus) -> Uuid {
        let id = Uuid::new_v4();
        state.containers.insert(Container {
            id,
            spec: ContainerSpec {
                name: "test".into(),
                image: "nginx:latest".into(),
                command: vec![],
                args: vec![],
                env: Default::default(),
                mounts: vec![],
                resources: Default::default(),
                labels: Default::default(),
                working_dir: None,
                user: None,
                hostname: None,
                network_mode: NetworkMode::Bridge,
                restart_policy: RestartPolicy::Never,
            },
            status,
            pid: None,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
            exit_code: None,
            rootfs_path: "/tmp/rootfs".into(),
            log_path: "/tmp/test.log".into(),
        });
        id
    }

    fn insert_image(state: &Arc<CriState>, reference: &str) {
        state.images.insert(OciImage {
            reference: reference.to_string(),
            digest: "sha256:abc".into(),
            layers: vec![],
            config: ImageConfig::default(),
            size_bytes: 0,
            pulled_at: Utc::now(),
        });
    }

    async fn get(app: axum::Router, uri: &str) -> axum::response::Response {
        app.oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    async fn post_json(app: axum::Router, uri: &str, body: serde_json::Value) -> axum::response::Response {
        app.oneshot(
            Request::post(uri)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap()
    }

    async fn delete(app: axum::Router, uri: &str) -> axum::response::Response {
        app.oneshot(Request::delete(uri).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    // --- health ---

    #[tokio::test]
    async fn test_health_returns_200() {
        let state = test_state();
        let app = create_router(state);
        let resp = get(app, "/api/cri/health").await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_health_body_contains_module() {
        let state = test_state();
        let app = create_router(state);
        let resp = get(app, "/api/cri/health").await;
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["module"], "cave-cri");
        assert_eq!(json["status"], "ok");
    }

    // --- list_containers ---

    #[tokio::test]
    async fn test_list_containers_empty() {
        let state = test_state();
        let app = create_router(state);
        let resp = get(app, "/api/cri/containers").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let list: Vec<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
        assert!(list.is_empty());
    }

    #[tokio::test]
    async fn test_list_containers_populated() {
        let state = test_state();
        insert_container(&state, ContainerStatus::Created);
        insert_container(&state, ContainerStatus::Running);
        let app = create_router(state);
        let resp = get(app, "/api/cri/containers").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let list: Vec<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(list.len(), 2);
    }

    // --- inspect_container ---

    #[tokio::test]
    async fn test_inspect_container_not_found() {
        let state = test_state();
        let app = create_router(state);
        let resp = get(app, &format!("/api/cri/containers/{}", Uuid::new_v4())).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_inspect_container_found() {
        let state = test_state();
        let id = insert_container(&state, ContainerStatus::Created);
        let app = create_router(state);
        let resp = get(app, &format!("/api/cri/containers/{}", id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_inspect_container_invalid_uuid() {
        let state = test_state();
        let app = create_router(state);
        // Invalid UUID in path → 400 Bad Request from axum path extractor
        let resp = get(app, "/api/cri/containers/not-a-uuid").await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // --- start_container ---

    #[tokio::test]
    async fn test_start_container_not_found() {
        let state = test_state();
        let app = create_router(state);
        let resp = post_json(app, &format!("/api/cri/containers/{}/start", Uuid::new_v4()), serde_json::json!({})).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_start_container_success() {
        let state = test_state();
        let id = insert_container(&state, ContainerStatus::Created);
        let app = create_router(state);
        let resp = post_json(app, &format!("/api/cri/containers/{}/start", id), serde_json::json!({})).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn test_start_already_running_returns_400() {
        let state = test_state();
        let id = insert_container(&state, ContainerStatus::Running);
        let app = create_router(state);
        let resp = post_json(app, &format!("/api/cri/containers/{}/start", id), serde_json::json!({})).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // --- stop_container ---

    #[tokio::test]
    async fn test_stop_container_not_found() {
        let state = test_state();
        let app = create_router(state);
        let resp = post_json(app, &format!("/api/cri/containers/{}/stop", Uuid::new_v4()), serde_json::json!({})).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_stop_already_stopped_returns_204() {
        let state = test_state();
        let id = insert_container(&state, ContainerStatus::Stopped);
        let app = create_router(state);
        let resp = post_json(app, &format!("/api/cri/containers/{}/stop", id), serde_json::json!({})).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    // --- kill_container ---

    #[tokio::test]
    async fn test_kill_container_not_found() {
        let state = test_state();
        let app = create_router(state);
        let resp = post_json(app, &format!("/api/cri/containers/{}/kill", Uuid::new_v4()), serde_json::json!({})).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_kill_container_no_pid_returns_204() {
        let state = test_state();
        let id = insert_container(&state, ContainerStatus::Created);
        let app = create_router(state);
        let resp = post_json(app, &format!("/api/cri/containers/{}/kill", id), serde_json::json!({})).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    // --- delete_container ---

    #[tokio::test]
    async fn test_delete_container_not_found() {
        let state = test_state();
        let app = create_router(state);
        let resp = delete(app, &format!("/api/cri/containers/{}", Uuid::new_v4())).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_delete_running_container_returns_400() {
        let state = test_state();
        let id = insert_container(&state, ContainerStatus::Running);
        let app = create_router(state);
        let resp = delete(app, &format!("/api/cri/containers/{}", id)).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_delete_stopped_container_returns_204() {
        let state = test_state();
        let id = insert_container(&state, ContainerStatus::Stopped);
        let app = create_router(state);
        let resp = delete(app, &format!("/api/cri/containers/{}", id)).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    // --- list_images ---

    #[tokio::test]
    async fn test_list_images_empty() {
        let state = test_state();
        let app = create_router(state);
        let resp = get(app, "/api/cri/images").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let list: Vec<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
        assert!(list.is_empty());
    }

    #[tokio::test]
    async fn test_list_images_populated() {
        let state = test_state();
        insert_image(&state, "nginx:latest");
        insert_image(&state, "alpine:3.18");
        let app = create_router(state);
        let resp = get(app, "/api/cri/images").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let list: Vec<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(list.len(), 2);
    }
}
