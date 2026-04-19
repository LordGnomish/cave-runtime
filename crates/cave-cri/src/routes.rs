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
