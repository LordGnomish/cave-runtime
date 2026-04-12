//! /pulp/api/v3/content-guards/ routes.

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
        .route(
            "/pulp/api/v3/content-guards/",
            get(list_guards).post(create_guard),
        )
        .route(
            "/pulp/api/v3/content-guards/{type}/{id}/",
            get(get_guard),
        )
        .route("/pulp/api/v3/signing-services/", get(list_signing_services))
        .route(
            "/pulp/api/v3/exporters/core/pulp/",
            get(list_exporters).post(create_exporter),
        )
        .with_state(state)
}

async fn list_guards(
    State(state): State<Arc<ArtifactsState>>,
) -> Json<PulpPage<ContentGuard>> {
    Json(PulpPage::of(state.list_content_guards().await))
}

async fn create_guard(
    State(state): State<Arc<ArtifactsState>>,
    Json(req): Json<serde_json::Value>,
) -> Result<(StatusCode, Json<ContentGuard>), (StatusCode, Json<serde_json::Value>)> {
    let name = req["name"]
        .as_str()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "detail": "name required" }))))?
        .to_string();
    let guard_type_str = req["guard_type"].as_str().unwrap_or("rbac");
    let guard_type = match guard_type_str {
        "content_redirect" => ContentGuardType::ContentRedirect,
        "header" => ContentGuardType::Header,
        _ => ContentGuardType::Rbac,
    };
    let mut guard = ContentGuard::new(name, guard_type);
    guard.description = req["description"].as_str().map(|s| s.to_string());
    guard.header_name = req["header_name"].as_str().map(|s| s.to_string());
    guard.header_value = req["header_value"].as_str().map(|s| s.to_string());
    let stored = state.create_content_guard(guard).await;
    Ok((StatusCode::CREATED, Json(stored)))
}

async fn get_guard(
    State(state): State<Arc<ArtifactsState>>,
    Path((guard_type, id)): Path<(String, String)>,
) -> Result<Json<ContentGuard>, (StatusCode, Json<serde_json::Value>)> {
    let href = format!("/pulp/api/v3/content-guards/{guard_type}/{id}/");
    state
        .get_content_guard(&href)
        .await
        .map(Json)
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(serde_json::json!({ "detail": "not found" }))))
}

async fn list_signing_services(
    State(state): State<Arc<ArtifactsState>>,
) -> Json<PulpPage<SigningService>> {
    Json(PulpPage::of(state.list_signing_services().await))
}

async fn list_exporters(
    State(state): State<Arc<ArtifactsState>>,
) -> Json<PulpPage<Exporter>> {
    Json(PulpPage::of(state.list_exporters().await))
}

async fn create_exporter(
    State(state): State<Arc<ArtifactsState>>,
    Json(req): Json<serde_json::Value>,
) -> Result<(StatusCode, Json<Exporter>), (StatusCode, Json<serde_json::Value>)> {
    let name = req["name"]
        .as_str()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "detail": "name required" }))))?
        .to_string();
    let path = req["path"]
        .as_str()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "detail": "path required" }))))?
        .to_string();
    let mut exporter = Exporter::new(name, path);
    if let Some(repos) = req["repositories"].as_array() {
        exporter.repositories = repos
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| s.to_string())
            .collect();
    }
    let stored = state.create_exporter(exporter).await;
    Ok((StatusCode::CREATED, Json(stored)))
}
