//! /pulp/api/v3/repositories/ routes.

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
        .route(
            "/pulp/api/v3/repositories/",
            get(list_repositories).post(create_repository),
        )
        .route(
            "/pulp/api/v3/repositories/{plugin}/{subtype}/{id}/",
            get(get_repository).delete(delete_repository),
        )
        .route(
            "/pulp/api/v3/repositories/{plugin}/{subtype}/{id}/versions/",
            get(list_versions),
        )
        .route(
            "/pulp/api/v3/repositories/{plugin}/{subtype}/{id}/sync/",
            post(sync_repository),
        )
        .route(
            "/pulp/api/v3/repositories/{plugin}/{subtype}/{id}/modify/",
            post(modify_repository),
        )
        .with_state(state)
}

async fn list_repositories(
    State(state): State<Arc<ArtifactsState>>,
) -> Json<PulpPage<Repository>> {
    Json(PulpPage::of(state.list_repositories().await))
}

async fn create_repository(
    State(state): State<Arc<ArtifactsState>>,
    Json(req): Json<CreateRepositoryRequest>,
) -> Result<(StatusCode, Json<Repository>), (StatusCode, Json<serde_json::Value>)> {
    state
        .create_repository(req)
        .await
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(|e| (status(e.status_code()), Json(err_body(e.to_string()))))
}

async fn get_repository(
    State(state): State<Arc<ArtifactsState>>,
    Path((plugin, subtype, id)): Path<(String, String, String)>,
) -> Result<Json<Repository>, (StatusCode, Json<serde_json::Value>)> {
    let href = make_repo_href(&plugin, &subtype, &id);
    state
        .get_repository(&href)
        .await
        .map(Json)
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(err_body(format!("repository {href} not found")))))
}

async fn delete_repository(
    State(state): State<Arc<ArtifactsState>>,
    Path((plugin, subtype, id)): Path<(String, String, String)>,
) -> Result<StatusCode, (StatusCode, Json<serde_json::Value>)> {
    let href = make_repo_href(&plugin, &subtype, &id);
    state
        .delete_repository(&href)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (status(e.status_code()), Json(err_body(e.to_string()))))
}

async fn list_versions(
    State(state): State<Arc<ArtifactsState>>,
    Path((plugin, subtype, id)): Path<(String, String, String)>,
) -> Json<PulpPage<RepositoryVersion>> {
    let href = make_repo_href(&plugin, &subtype, &id);
    Json(PulpPage::of(state.list_repo_versions(&href).await))
}

async fn sync_repository(
    State(state): State<Arc<ArtifactsState>>,
    Path((plugin, subtype, id)): Path<(String, String, String)>,
    Json(req): Json<SyncRequest>,
) -> Result<Json<AsyncOperationResponse>, (StatusCode, Json<serde_json::Value>)> {
    let repo_href = make_repo_href(&plugin, &subtype, &id);

    let repo = state
        .get_repository(&repo_href)
        .await
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(err_body(format!("repository {repo_href} not found"))),
            )
        })?;

    let remote_href = req.remote.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(err_body("remote href is required for sync")),
        )
    })?;

    let remote = state
        .get_remote(&remote_href)
        .await
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(err_body(format!("remote {remote_href} not found"))),
            )
        })?;

    let opts = crate::sync::SyncOptions {
        mirror: req.mirror.unwrap_or(false),
        optimize: req.optimize.unwrap_or(true),
    };

    let task = state
        .run_as_task(
            "pulp.tasking.tasks.base.general_multi_call_task",
            vec![repo_href.clone()],
            move |s| async move { crate::sync::sync_repository(s, repo, remote, opts).await },
        )
        .await;

    Ok(Json(AsyncOperationResponse { task: task.pulp_href }))
}

async fn modify_repository(
    State(state): State<Arc<ArtifactsState>>,
    Path((plugin, subtype, id)): Path<(String, String, String)>,
    Json(req): Json<AddContentRequest>,
) -> Result<Json<AsyncOperationResponse>, (StatusCode, Json<serde_json::Value>)> {
    let repo_href = make_repo_href(&plugin, &subtype, &id);

    let _ = state.get_repository(&repo_href).await.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(err_body(format!("repository {repo_href} not found"))),
        )
    })?;

    let content_units = req.content_units.clone();
    let task = state
        .run_as_task(
            "pulp.tasking.tasks.base.general_create_task",
            vec![repo_href.clone()],
            move |s| async move {
                let ver = s.create_repo_version(&repo_href, content_units).await?;
                Ok(vec![ver.pulp_href])
            },
        )
        .await;

    Ok(Json(AsyncOperationResponse { task: task.pulp_href }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_repo_href(plugin: &str, subtype: &str, id: &str) -> String {
    format!("/pulp/api/v3/repositories/{plugin}/{subtype}/{id}/")
}

fn status(code: u16) -> StatusCode {
    StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

fn err_body(msg: impl Into<String>) -> serde_json::Value {
    serde_json::json!({ "detail": msg.into() })
}
