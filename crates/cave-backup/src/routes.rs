//! HTTP routes for cave-backup — Velero replacement API.
//!
//! Endpoints:
//!   GET  /api/v1/backups
//!   POST /api/v1/backups
//!   GET  /api/v1/backups/:id
//!   DELETE /api/v1/backups/:id
//!   GET  /api/v1/restores
//!   POST /api/v1/restores
//!   GET  /api/v1/restores/:id
//!   GET  /api/v1/schedules
//!   POST /api/v1/schedules
//!   GET  /api/v1/schedules/:id
//!   PUT  /api/v1/schedules/:id
//!   DELETE /api/v1/schedules/:id
//!   POST /api/v1/download-requests
//!   GET  /api/backup/health

use crate::{
    schedule::validate_cron_expression,
    storage::BackupStore,
    types::{Backup, BackupSchedule, BackupSpec, DownloadRequest, RestoreJob, RestoreSpec, RetentionPolicy},
    BackupState,
};
use axum::{
    routing::get,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<serde_json::Value>)>;

fn err(code: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<serde_json::Value>) {
    (code, Json(serde_json::json!({ "error": msg.into() })))
}

pub fn create_router(state: Arc<BackupState>) -> Router {
    Router::new()
        // Backups
        .route("/api/v1/backups", get(list_backups).post(create_backup))
        .route(
            "/api/v1/backups/:id",
            get(get_backup).delete(delete_backup),
        )
        // Restores
        .route("/api/v1/restores", get(list_restores).post(create_restore))
        .route("/api/v1/restores/:id", get(get_restore))
        // Schedules
        .route(
            "/api/v1/schedules",
            get(list_schedules).post(create_schedule),
        )
        .route(
            "/api/v1/schedules/:id",
            get(get_schedule).put(update_schedule).delete(delete_schedule),
        )
        // Download requests
        .route("/api/v1/download-requests", post(create_download_request))
        // Health
        .route("/api/backup/health", get(health))
        .with_state(state)
}

// ─── Backups ──────────────────────────────────────────────────────────────────

async fn list_backups(State(s): State<Arc<BackupState>>) -> Json<Vec<Backup>> {
    Json(s.store.list_backups().await)
}

#[derive(Deserialize)]
struct CreateBackupRequest {
    name: String,
    spec: BackupSpec,
}

async fn create_backup(
    State(s): State<Arc<BackupState>>,
    Json(req): Json<CreateBackupRequest>,
) -> ApiResult<Backup> {
    if req.name.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "name is required"));
    }
    let backup = Backup::new(req.name, req.spec);
    s.store.insert_backup(backup.clone()).await;
    Ok(Json(backup))
}

async fn get_backup(
    State(s): State<Arc<BackupState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<Backup> {
    s.store
        .get_backup(id)
        .await
        .map(Json)
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "backup not found"))
}

async fn delete_backup(
    State(s): State<Arc<BackupState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<serde_json::Value> {
    if s.store.delete_backup(id).await {
        Ok(Json(serde_json::json!({ "deleted": id })))
    } else {
        Err(err(StatusCode::NOT_FOUND, "backup not found"))
    }
}

// ─── Restores ─────────────────────────────────────────────────────────────────

async fn list_restores(State(s): State<Arc<BackupState>>) -> Json<Vec<RestoreJob>> {
    Json(s.store.list_restores().await)
}

#[derive(Deserialize)]
struct CreateRestoreRequest {
    name: String,
    spec: RestoreSpec,
}

async fn create_restore(
    State(s): State<Arc<BackupState>>,
    Json(req): Json<CreateRestoreRequest>,
) -> ApiResult<RestoreJob> {
    if req.name.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "name is required"));
    }
    let restore = RestoreJob::new(req.name, req.spec);
    s.store.insert_restore(restore.clone()).await;
    Ok(Json(restore))
}

async fn get_restore(
    State(s): State<Arc<BackupState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<RestoreJob> {
    s.store
        .get_restore(id)
        .await
        .map(Json)
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "restore not found"))
}

// ─── Schedules ────────────────────────────────────────────────────────────────

async fn list_schedules(State(s): State<Arc<BackupState>>) -> Json<Vec<BackupSchedule>> {
    Json(s.store.list_schedules().await)
}

#[derive(Deserialize)]
struct CreateScheduleRequest {
    name: String,
    cron_expression: String,
    backup_spec: BackupSpec,
    retention: RetentionPolicy,
}

async fn create_schedule(
    State(s): State<Arc<BackupState>>,
    Json(req): Json<CreateScheduleRequest>,
) -> ApiResult<BackupSchedule> {
    if !validate_cron_expression(&req.cron_expression) {
        return Err(err(
            StatusCode::BAD_REQUEST,
            format!("invalid cron expression: {}", req.cron_expression),
        ));
    }
    let schedule = BackupSchedule::new(req.name, req.cron_expression, req.backup_spec, req.retention);
    s.store.insert_schedule(schedule.clone()).await;
    Ok(Json(schedule))
}

async fn get_schedule(
    State(s): State<Arc<BackupState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<BackupSchedule> {
    s.store
        .get_schedule(id)
        .await
        .map(Json)
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "schedule not found"))
}

#[derive(Deserialize)]
struct UpdateScheduleRequest {
    paused: Option<bool>,
    cron_expression: Option<String>,
    retention: Option<RetentionPolicy>,
}

async fn update_schedule(
    State(s): State<Arc<BackupState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateScheduleRequest>,
) -> ApiResult<BackupSchedule> {
    let mut schedule = s
        .store
        .get_schedule(id)
        .await
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "schedule not found"))?;

    if let Some(paused) = req.paused {
        schedule.paused = paused;
    }
    if let Some(expr) = req.cron_expression {
        if !validate_cron_expression(&expr) {
            return Err(err(StatusCode::BAD_REQUEST, "invalid cron expression"));
        }
        schedule.cron_expression = expr;
    }
    if let Some(retention) = req.retention {
        schedule.retention = retention;
    }

    s.store.update_schedule(schedule.clone()).await;
    Ok(Json(schedule))
}

async fn delete_schedule(
    State(s): State<Arc<BackupState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<serde_json::Value> {
    if s.store.delete_schedule(id).await {
        Ok(Json(serde_json::json!({ "deleted": id })))
    } else {
        Err(err(StatusCode::NOT_FOUND, "schedule not found"))
    }
}

// ─── Download Requests ────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CreateDownloadRequest {
    backup_id: Uuid,
    #[serde(default = "default_ttl")]
    ttl_seconds: u64,
}

fn default_ttl() -> u64 {
    3600
}

#[derive(Serialize)]
struct DownloadResponse {
    request: DownloadRequest,
    description: String,
}

async fn create_download_request(
    State(s): State<Arc<BackupState>>,
    Json(req): Json<CreateDownloadRequest>,
) -> ApiResult<DownloadResponse> {
    // Verify the backup exists.
    if s.store.get_backup(req.backup_id).await.is_none() {
        return Err(err(StatusCode::NOT_FOUND, "backup not found"));
    }
    let dl = DownloadRequest::new(req.backup_id, req.ttl_seconds);
    s.store.insert_download(dl.clone()).await;
    Ok(Json(DownloadResponse {
        description: format!(
            "Download link will be available at /api/v1/download-requests/{}",
            dl.id
        ),
        request: dl,
    }))
}

// ─── Health ───────────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-backup",
        "status": "ok",
        "upstream": "Velero",
        "features": [
            "full-cluster-backup",
            "namespace-scoped-backup",
            "label-selector-backup",
            "resource-filter-backup",
            "s3-compatible-storage",
            "azure-blob-storage",
            "gcs-storage",
            "local-storage",
            "cron-schedule",
            "retention-policies",
            "csi-volume-snapshots",
            "restic-kopia-file-backup",
            "pre-post-hooks",
            "cross-cluster-restore",
            "aes256-encryption",
            "namespace-remapping",
            "storage-class-remapping"
        ]
    }))
}
