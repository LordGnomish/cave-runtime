//! /pulp/api/v3/remotes/ routes.

use crate::models::*;
use crate::store::ArtifactsState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use std::sync::Arc;

pub fn router(state: Arc<ArtifactsState>) -> Router {
    Router::new()
        .route("/pulp/api/v3/remotes/", get(list_remotes).post(create_remote))
        .route(
            "/pulp/api/v3/remotes/{plugin}/{subtype}/{id}/",
            get(get_remote).delete(delete_remote),
        )
        .with_state(state)
}

async fn list_remotes(State(state): State<Arc<ArtifactsState>>) -> Json<PulpPage<Remote>> {
    // Strip passwords from responses.
    let remotes: Vec<Remote> = state
        .list_remotes()
        .await
        .into_iter()
        .map(redact_remote)
        .collect();
    Json(PulpPage::of(remotes))
}

async fn create_remote(
    State(state): State<Arc<ArtifactsState>>,
    Json(req): Json<CreateRemoteRequest>,
) -> Result<(StatusCode, Json<Remote>), (StatusCode, Json<serde_json::Value>)> {
    state
        .create_remote(req)
        .await
        .map(|r| (StatusCode::CREATED, Json(redact_remote(r))))
        .map_err(|e| (status(e.status_code()), Json(err_body(e.to_string()))))
}

async fn get_remote(
    State(state): State<Arc<ArtifactsState>>,
    Path((plugin, subtype, id)): Path<(String, String, String)>,
) -> Result<Json<Remote>, (StatusCode, Json<serde_json::Value>)> {
    let href = format!("/pulp/api/v3/remotes/{plugin}/{subtype}/{id}/");
    state
        .get_remote(&href)
        .await
        .map(|r| Json(redact_remote(r)))
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(err_body("remote not found"))))
}

async fn delete_remote(
    State(state): State<Arc<ArtifactsState>>,
    Path((plugin, subtype, id)): Path<(String, String, String)>,
) -> Result<StatusCode, (StatusCode, Json<serde_json::Value>)> {
    let href = format!("/pulp/api/v3/remotes/{plugin}/{subtype}/{id}/");
    state
        .delete_remote(&href)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (status(e.status_code()), Json(err_body(e.to_string()))))
}

fn redact_remote(mut r: Remote) -> Remote {
    r.password = r.password.map(|_| "**redacted**".into());
    r
}

fn status(code: u16) -> StatusCode {
    StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

fn err_body(msg: impl Into<String>) -> serde_json::Value {
    serde_json::json!({ "detail": msg.into() })
}
