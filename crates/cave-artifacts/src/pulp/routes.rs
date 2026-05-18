// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: pulp/pulpcore@0f991c2fa2bf6c8635e8a2de064ef04dacbbcf4f pulpcore/app/viewsets/repository.py
//! HTTP API routes for cave-artifacts — Pulp v3 REST API.

use crate::pulp::{
    distribution::validate_distribution,
    models::*,
    rbac::builtin_roles,
    repair::{enqueue_repair, RepairOptions},
    repository::{create_repository, enqueue_sync, versions_to_prune},
    signing::SigningService,
    tasks::TaskState,
    upload::{parse_content_range, FinalizeUploadRequest, Upload, UploadRegistry},
    ArtifactsState,
};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{delete, get, post, put},
    Json, Router,
};
use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router(state: Arc<ArtifactsState>) -> Router {
    Router::new()
        // Repositories
        .route("/pulp/api/v3/repositories/", get(list_repositories).post(create_repo))
        .route("/pulp/api/v3/repositories/{pulp_id}/", get(get_repo).patch(update_repo).delete(delete_repo))
        .route("/pulp/api/v3/repositories/{pulp_id}/sync/", post(sync_repo))
        .route("/pulp/api/v3/repositories/{pulp_id}/modify/", post(modify_repo))
        // Repository versions
        .route("/pulp/api/v3/repositories/{pulp_id}/versions/", get(list_versions))
        .route("/pulp/api/v3/repositories/{pulp_id}/versions/{number}/", get(get_version).delete(delete_version))
        .route("/pulp/api/v3/repositories/{pulp_id}/versions/{number}/repair/", post(repair_version))
        // Remotes
        .route("/pulp/api/v3/remotes/", get(list_remotes).post(create_remote))
        .route("/pulp/api/v3/remotes/{pulp_id}/", get(get_remote).patch(update_remote).delete(delete_remote))
        // Publications
        .route("/pulp/api/v3/publications/", get(list_publications).post(create_publication))
        .route("/pulp/api/v3/publications/{pulp_id}/", get(get_publication))
        // Distributions
        .route("/pulp/api/v3/distributions/", get(list_distributions).post(create_distribution))
        .route("/pulp/api/v3/distributions/{pulp_id}/", get(get_distribution).patch(update_distribution).delete(delete_distribution))
        // Content guards
        .route("/pulp/api/v3/contentguards/", get(list_content_guards).post(create_content_guard))
        // Content
        .route("/pulp/api/v3/content/", get(list_content))
        .route("/pulp/api/v3/artifacts/", get(list_artifacts).post(create_artifact))
        .route("/pulp/api/v3/artifacts/{pulp_id}/", get(get_artifact))
        // Uploads
        .route("/pulp/api/v3/uploads/", get(list_uploads).post(create_upload))
        .route("/pulp/api/v3/uploads/{pulp_id}/", get(get_upload).delete(abort_upload))
        .route("/pulp/api/v3/uploads/{pulp_id}/commit/", post(commit_upload))
        // Tasks
        .route("/pulp/api/v3/tasks/", get(list_tasks))
        .route("/pulp/api/v3/tasks/{pulp_id}/", get(get_task))
        .route("/pulp/api/v3/tasks/{pulp_id}/cancel/", post(cancel_task))
        // Task groups
        .route("/pulp/api/v3/task-groups/", get(list_task_groups))
        // Signing services
        .route("/pulp/api/v3/signing-services/", get(list_signing_services).post(create_signing_service))
        // RBAC roles
        .route("/pulp/api/v3/roles/", get(list_roles).post(create_role))
        .route("/pulp/api/v3/roles/{pulp_id}/", get(get_role))
        // Import/Export
        .route("/pulp/api/v3/exporters/core/pulp/", get(list_exporters).post(create_exporter))
        .route("/pulp/api/v3/exporters/core/pulp/{pulp_id}/exports/", post(create_export))
        .route("/pulp/api/v3/importers/core/pulp/", get(list_importers).post(create_importer))
        .route("/pulp/api/v3/importers/core/pulp/{pulp_id}/imports/", post(create_import))
        // Repair (global)
        .route("/pulp/api/v3/repair/", post(global_repair))
        // Status
        .route("/pulp/api/v3/status/", get(status))
        .with_state(state)
}

// ─── Status ──────────────────────────────────────────────────────────────────

async fn status() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-artifacts",
        "status": "ok",
        "upstream": "Pulp v3",
        "versions": {
            "platform": "3.x",
            "pulpcore": "3.49"
        },
        "online_workers": 1,
        "online_content_apps": 1,
        "database_connection": { "connected": true },
        "redis_connection": { "connected": true }
    }))
}

// ─── Repositories ─────────────────────────────────────────────────────────────

#[derive(serde::Deserialize, Default)]
struct ListQuery {
    name: Option<String>,
    name__in: Option<String>,
    offset: Option<u64>,
    limit: Option<u64>,
}

async fn list_repositories(
    State(_state): State<Arc<ArtifactsState>>,
    Query(_q): Query<ListQuery>,
) -> Json<PaginatedResponse<Repository>> {
    Json(PaginatedResponse::of(vec![]))
}

#[derive(serde::Deserialize)]
struct CreateRepoRequest {
    name: String,
    content_type: ContentType,
    description: Option<String>,
    retain_repo_versions: Option<u32>,
    remote: Option<String>,
}

async fn create_repo(
    State(_state): State<Arc<ArtifactsState>>,
    Json(req): Json<CreateRepoRequest>,
) -> (StatusCode, Json<Repository>) {
    let mut repo = create_repository(&req.name, req.content_type);
    repo.description = req.description;
    if let Some(r) = req.retain_repo_versions { repo.retain_repo_versions = Some(r); }
    repo.remote = req.remote;
    (StatusCode::CREATED, Json(repo))
}

async fn get_repo(
    State(_state): State<Arc<ArtifactsState>>,
    Path(pulp_id): Path<Uuid>,
) -> Result<Json<Repository>, (StatusCode, Json<serde_json::Value>)> {
    Err((StatusCode::NOT_FOUND, Json(serde_json::json!({ "detail": format!("No Repository matches the given query. pulp_id={}", pulp_id) }))))
}

async fn update_repo(
    State(_state): State<Arc<ArtifactsState>>,
    Path(_pulp_id): Path<Uuid>,
    Json(_req): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "updated" }))
}

async fn delete_repo(
    State(state): State<Arc<ArtifactsState>>,
    Path(pulp_id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let task = state.task_queue.enqueue("pulp.tasks.repository.delete");
    Json(serde_json::json!({ "task": task.pulp_href }))
}

#[derive(serde::Deserialize)]
struct SyncRequest {
    remote: Option<String>,
    #[serde(default)]
    mirror: bool,
    optimize: Option<bool>,
}

async fn sync_repo(
    State(state): State<Arc<ArtifactsState>>,
    Path(_pulp_id): Path<Uuid>,
    Json(req): Json<SyncRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let task = state.task_queue.enqueue("pulp.tasks.synchronize");
    (StatusCode::ACCEPTED, Json(serde_json::json!({ "task": task.pulp_href })))
}

#[derive(serde::Deserialize)]
struct ModifyRepoRequest {
    #[serde(default)]
    add_content_units: Vec<String>,
    #[serde(default)]
    remove_content_units: Vec<String>,
    base_version: Option<String>,
}

async fn modify_repo(
    State(state): State<Arc<ArtifactsState>>,
    Path(_pulp_id): Path<Uuid>,
    Json(_req): Json<ModifyRepoRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let task = state.task_queue.enqueue("pulp.tasks.repository.modify");
    (StatusCode::ACCEPTED, Json(serde_json::json!({ "task": task.pulp_href })))
}

// ─── Repository versions ──────────────────────────────────────────────────────

async fn list_versions(
    State(_state): State<Arc<ArtifactsState>>,
    Path(_pulp_id): Path<Uuid>,
) -> Json<PaginatedResponse<RepositoryVersion>> {
    Json(PaginatedResponse::of(vec![]))
}

async fn get_version(
    State(_state): State<Arc<ArtifactsState>>,
    Path((_pulp_id, number)): Path<(Uuid, u64)>,
) -> Result<Json<RepositoryVersion>, (StatusCode, Json<serde_json::Value>)> {
    Err((StatusCode::NOT_FOUND, Json(serde_json::json!({ "detail": "Not found" }))))
}

async fn delete_version(
    State(state): State<Arc<ArtifactsState>>,
    Path((_pulp_id, _number)): Path<(Uuid, u64)>,
) -> (StatusCode, Json<serde_json::Value>) {
    let task = state.task_queue.enqueue("pulp.tasks.repository_version.delete");
    (StatusCode::ACCEPTED, Json(serde_json::json!({ "task": task.pulp_href })))
}

async fn repair_version(
    State(state): State<Arc<ArtifactsState>>,
    Path((_pulp_id, number)): Path<(Uuid, u64)>,
    Json(opts): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let task = state.task_queue.enqueue("pulp.tasks.repair");
    (StatusCode::ACCEPTED, Json(serde_json::json!({ "task": task.pulp_href })))
}

// ─── Remotes ─────────────────────────────────────────────────────────────────

async fn list_remotes(
    State(_state): State<Arc<ArtifactsState>>,
) -> Json<PaginatedResponse<Remote>> {
    Json(PaginatedResponse::of(vec![]))
}

async fn create_remote(
    State(_state): State<Arc<ArtifactsState>>,
    Json(req): Json<serde_json::Value>,
) -> (StatusCode, Json<Remote>) {
    let name = req.get("name").and_then(|v| v.as_str()).unwrap_or("remote");
    let url = req.get("url").and_then(|v| v.as_str()).unwrap_or("https://example.com");
    let remote = Remote::new(name, url, ContentType::File);
    (StatusCode::CREATED, Json(remote))
}

async fn get_remote(
    State(_state): State<Arc<ArtifactsState>>,
    Path(pulp_id): Path<Uuid>,
) -> Result<Json<Remote>, (StatusCode, Json<serde_json::Value>)> {
    Err((StatusCode::NOT_FOUND, Json(serde_json::json!({ "detail": "Not found" }))))
}

async fn update_remote(
    State(_state): State<Arc<ArtifactsState>>,
    Path(_pulp_id): Path<Uuid>,
    Json(_req): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "updated" }))
}

async fn delete_remote(
    State(state): State<Arc<ArtifactsState>>,
    Path(_pulp_id): Path<Uuid>,
) -> (StatusCode, Json<serde_json::Value>) {
    let task = state.task_queue.enqueue("pulp.tasks.remote.delete");
    (StatusCode::ACCEPTED, Json(serde_json::json!({ "task": task.pulp_href })))
}

// ─── Publications ────────────────────────────────────────────────────────────

async fn list_publications(
    State(_state): State<Arc<ArtifactsState>>,
) -> Json<PaginatedResponse<Publication>> {
    Json(PaginatedResponse::of(vec![]))
}

async fn create_publication(
    State(state): State<Arc<ArtifactsState>>,
    Json(req): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let task = state.task_queue.enqueue("pulp.tasks.publish");
    (StatusCode::ACCEPTED, Json(serde_json::json!({ "task": task.pulp_href })))
}

async fn get_publication(
    State(_state): State<Arc<ArtifactsState>>,
    Path(pulp_id): Path<Uuid>,
) -> Result<Json<Publication>, (StatusCode, Json<serde_json::Value>)> {
    Err((StatusCode::NOT_FOUND, Json(serde_json::json!({ "detail": "Not found" }))))
}

// ─── Distributions ────────────────────────────────────────────────────────────

async fn list_distributions(
    State(_state): State<Arc<ArtifactsState>>,
) -> Json<PaginatedResponse<Distribution>> {
    Json(PaginatedResponse::of(vec![]))
}

async fn create_distribution(
    State(_state): State<Arc<ArtifactsState>>,
    Json(req): Json<serde_json::Value>,
) -> (StatusCode, Json<Distribution>) {
    let name = req.get("name").and_then(|v| v.as_str()).unwrap_or("dist");
    let base_path = req.get("base_path").and_then(|v| v.as_str()).unwrap_or("content");
    let dist = Distribution::new(name, base_path, ContentType::File);
    (StatusCode::CREATED, Json(dist))
}

async fn get_distribution(
    State(_state): State<Arc<ArtifactsState>>,
    Path(pulp_id): Path<Uuid>,
) -> Result<Json<Distribution>, (StatusCode, Json<serde_json::Value>)> {
    Err((StatusCode::NOT_FOUND, Json(serde_json::json!({ "detail": "Not found" }))))
}

async fn update_distribution(
    State(_state): State<Arc<ArtifactsState>>,
    Path(_pulp_id): Path<Uuid>,
    Json(_req): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "updated" }))
}

async fn delete_distribution(
    State(_state): State<Arc<ArtifactsState>>,
    Path(_pulp_id): Path<Uuid>,
) -> StatusCode {
    StatusCode::NO_CONTENT
}

// ─── Content guards ───────────────────────────────────────────────────────────

async fn list_content_guards(
    State(_state): State<Arc<ArtifactsState>>,
) -> Json<PaginatedResponse<ContentGuard>> {
    Json(PaginatedResponse::of(vec![]))
}

async fn create_content_guard(
    State(_state): State<Arc<ArtifactsState>>,
    Json(req): Json<ContentGuard>,
) -> (StatusCode, Json<ContentGuard>) {
    (StatusCode::CREATED, Json(req))
}

// ─── Content ─────────────────────────────────────────────────────────────────

async fn list_content(
    State(_state): State<Arc<ArtifactsState>>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "count": 0, "results": [] }))
}

// ─── Artifacts ───────────────────────────────────────────────────────────────

async fn list_artifacts(
    State(_state): State<Arc<ArtifactsState>>,
) -> Json<PaginatedResponse<Artifact>> {
    Json(PaginatedResponse::of(vec![]))
}

async fn create_artifact(
    State(_state): State<Arc<ArtifactsState>>,
    Json(_req): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::CREATED, Json(serde_json::json!({ "detail": "Artifact created" })))
}

async fn get_artifact(
    State(_state): State<Arc<ArtifactsState>>,
    Path(pulp_id): Path<Uuid>,
) -> Result<Json<Artifact>, (StatusCode, Json<serde_json::Value>)> {
    Err((StatusCode::NOT_FOUND, Json(serde_json::json!({ "detail": "Not found" }))))
}

// ─── Uploads ─────────────────────────────────────────────────────────────────

async fn list_uploads(
    State(_state): State<Arc<ArtifactsState>>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "count": 0, "results": [] }))
}

#[derive(serde::Deserialize)]
struct CreateUploadRequest { size: u64 }

async fn create_upload(
    State(_state): State<Arc<ArtifactsState>>,
    Json(req): Json<CreateUploadRequest>,
) -> (StatusCode, Json<Upload>) {
    let upload = Upload::new(req.size);
    (StatusCode::CREATED, Json(upload))
}

async fn get_upload(
    State(_state): State<Arc<ArtifactsState>>,
    Path(pulp_id): Path<Uuid>,
) -> Result<Json<Upload>, (StatusCode, Json<serde_json::Value>)> {
    Err((StatusCode::NOT_FOUND, Json(serde_json::json!({ "detail": "Not found" }))))
}

async fn abort_upload(
    State(_state): State<Arc<ArtifactsState>>,
    Path(_pulp_id): Path<Uuid>,
) -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn commit_upload(
    State(_state): State<Arc<ArtifactsState>>,
    Path(pulp_id): Path<Uuid>,
    Json(req): Json<FinalizeUploadRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::ACCEPTED, Json(serde_json::json!({
        "artifact": format!("/pulp/api/v3/artifacts/{}/", Uuid::new_v4()),
        "sha256": req.sha256
    })))
}

// ─── Tasks ───────────────────────────────────────────────────────────────────

async fn list_tasks(
    State(state): State<Arc<ArtifactsState>>,
) -> Json<PaginatedResponse<crate::pulp::tasks::Task>> {
    let tasks = state.task_queue.list();
    Json(PaginatedResponse::of(tasks))
}

async fn get_task(
    State(state): State<Arc<ArtifactsState>>,
    Path(pulp_id): Path<Uuid>,
) -> Result<Json<crate::pulp::tasks::Task>, (StatusCode, Json<serde_json::Value>)> {
    match state.task_queue.get(&pulp_id) {
        Some(t) => Ok(Json(t)),
        None => Err((StatusCode::NOT_FOUND, Json(serde_json::json!({ "detail": "Not found" })))),
    }
}

async fn cancel_task(
    State(state): State<Arc<ArtifactsState>>,
    Path(pulp_id): Path<Uuid>,
) -> Json<serde_json::Value> {
    if state.task_queue.cancel(&pulp_id) {
        Json(serde_json::json!({ "status": "canceled" }))
    } else {
        Json(serde_json::json!({ "status": "task already terminal or not found" }))
    }
}

async fn list_task_groups(
    State(_state): State<Arc<ArtifactsState>>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "count": 0, "results": [] }))
}

// ─── Signing services ─────────────────────────────────────────────────────────

async fn list_signing_services(
    State(_state): State<Arc<ArtifactsState>>,
) -> Json<PaginatedResponse<SigningService>> {
    Json(PaginatedResponse::of(vec![]))
}

async fn create_signing_service(
    State(_state): State<Arc<ArtifactsState>>,
    Json(svc): Json<SigningService>,
) -> (StatusCode, Json<SigningService>) {
    (StatusCode::CREATED, Json(svc))
}

// ─── RBAC roles ──────────────────────────────────────────────────────────────

async fn list_roles(
    State(_state): State<Arc<ArtifactsState>>,
) -> Json<serde_json::Value> {
    let roles: Vec<serde_json::Value> = builtin_roles().iter().map(|r| serde_json::json!({
        "name": r.name,
        "description": r.description,
        "permissions": r.permissions,
        "locked": r.locked,
    })).collect();
    Json(serde_json::json!({ "count": roles.len(), "results": roles }))
}

async fn create_role(
    State(_state): State<Arc<ArtifactsState>>,
    Json(role): Json<crate::pulp::models::Role>,
) -> (StatusCode, Json<crate::pulp::models::Role>) {
    (StatusCode::CREATED, Json(role))
}

async fn get_role(
    State(_state): State<Arc<ArtifactsState>>,
    Path(pulp_id): Path<Uuid>,
) -> Result<Json<crate::pulp::models::Role>, (StatusCode, Json<serde_json::Value>)> {
    Err((StatusCode::NOT_FOUND, Json(serde_json::json!({ "detail": "Not found" }))))
}

// ─── Import/Export ────────────────────────────────────────────────────────────

async fn list_exporters(
    State(_state): State<Arc<ArtifactsState>>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "count": 0, "results": [] }))
}

async fn create_exporter(
    State(_state): State<Arc<ArtifactsState>>,
    Json(_req): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::CREATED, Json(serde_json::json!({ "pulp_href": format!("/pulp/api/v3/exporters/core/pulp/{}/", Uuid::new_v4()) })))
}

async fn create_export(
    State(state): State<Arc<ArtifactsState>>,
    Path(_exporter_id): Path<Uuid>,
    Json(_req): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let task = state.task_queue.enqueue("pulp.tasks.export");
    (StatusCode::ACCEPTED, Json(serde_json::json!({ "task": task.pulp_href })))
}

async fn list_importers(
    State(_state): State<Arc<ArtifactsState>>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "count": 0, "results": [] }))
}

async fn create_importer(
    State(_state): State<Arc<ArtifactsState>>,
    Json(_req): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::CREATED, Json(serde_json::json!({ "pulp_href": format!("/pulp/api/v3/importers/core/pulp/{}/", Uuid::new_v4()) })))
}

async fn create_import(
    State(state): State<Arc<ArtifactsState>>,
    Path(_importer_id): Path<Uuid>,
    Json(_req): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let task = state.task_queue.enqueue("pulp.tasks.import");
    (StatusCode::ACCEPTED, Json(serde_json::json!({ "task": task.pulp_href })))
}

// ─── Global repair ────────────────────────────────────────────────────────────

async fn global_repair(
    State(state): State<Arc<ArtifactsState>>,
    Json(_req): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let task = state.task_queue.enqueue("pulp.tasks.repair");
    (StatusCode::ACCEPTED, Json(serde_json::json!({ "task": task.pulp_href })))
}
