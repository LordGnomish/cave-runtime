//! /pulp/api/v3/tasks/ routes.

use crate::models::*;
use crate::store::ArtifactsState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;

pub fn router(state: Arc<ArtifactsState>) -> Router {
    Router::new()
        .route("/pulp/api/v3/tasks/", get(list_tasks))
        .route("/pulp/api/v3/tasks/{id}/", get(get_task))
        .route("/pulp/api/v3/tasks/{id}/cancel/", post(cancel_task))
        .with_state(state)
}

async fn list_tasks(State(state): State<Arc<ArtifactsState>>) -> Json<PulpPage<Task>> {
    Json(PulpPage::of(state.list_tasks().await))
}

async fn get_task(
    State(state): State<Arc<ArtifactsState>>,
    Path(id): Path<String>,
) -> Result<Json<Task>, (StatusCode, Json<serde_json::Value>)> {
    let href = format!("/pulp/api/v3/tasks/{id}/");
    state
        .get_task(&href)
        .await
        .map(Json)
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(serde_json::json!({ "detail": "task not found" }))))
}

async fn cancel_task(
    State(state): State<Arc<ArtifactsState>>,
    Path(id): Path<String>,
) -> Result<Json<Task>, (StatusCode, Json<serde_json::Value>)> {
    let href = format!("/pulp/api/v3/tasks/{id}/");
    state
        .cancel_task(&href)
        .await
        .map(Json)
        .map_err(|e| (
            StatusCode::from_u16(e.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Json(serde_json::json!({ "detail": e.to_string() })),
        ))
}
