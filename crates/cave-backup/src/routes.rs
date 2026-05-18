// SPDX-License-Identifier: AGPL-3.0-or-later
//! HTTP routes for cave-backup.

use crate::models::{
    BackupSpec, BackupStorageLocation, BslAccessMode, BslPhase, ExistingResourcePolicy,
    FsBackupMethod, PluginInfo, ServerPhase, ServerStatus, StorageProvider,
    VolumeSnapshotLocation,
};
use crate::BackupState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct CreateBackupRequest {
    pub name: String,
    pub spec: BackupSpec,
}

#[derive(Deserialize)]
pub struct CreateRestoreRequest {
    pub name: String,
    pub backup_id: Uuid,
    pub backup_name: String,
    pub included_namespaces: Option<Vec<String>>,
    pub excluded_namespaces: Option<Vec<String>>,
    pub included_resources: Option<Vec<String>>,
    pub excluded_resources: Option<Vec<String>>,
    pub namespace_mappings: Option<HashMap<String, String>>,
    pub restore_pvs: Option<bool>,
    pub existing_resource_policy: Option<ExistingResourcePolicy>,
}

#[derive(Deserialize)]
pub struct CreateScheduleRequest {
    pub name: String,
    pub cron_expression: String,
    pub template: BackupSpec,
}

#[derive(Deserialize)]
pub struct CreateBslRequest {
    pub name: String,
    pub provider: StorageProvider,
    pub bucket: String,
    pub prefix: Option<String>,
    pub region: Option<String>,
    pub endpoint: Option<String>,
    pub access_mode: Option<BslAccessMode>,
    pub credential_secret: Option<String>,
    pub insecure_skip_tls_verify: Option<bool>,
    pub is_default: Option<bool>,
}

#[derive(Deserialize)]
pub struct CreateVslRequest {
    pub name: String,
    pub provider: StorageProvider,
    pub region: Option<String>,
    pub credential_secret: Option<String>,
    pub config: Option<HashMap<String, String>>,
    pub is_default: Option<bool>,
}

#[derive(Deserialize)]
pub struct CreateFsBackupRequest {
    pub backup_id: Uuid,
    pub method: FsBackupMethod,
    pub namespace: String,
    pub pod: String,
    pub volume: String,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn create_router(state: Arc<BackupState>) -> Router {
    Router::new()
        // Backups
        .route("/api/backup/backups", post(create_backup))
        .route("/api/backup/backups", get(list_backups))
        .route("/api/backup/backups/{id}", get(get_backup))
        .route("/api/backup/backups/{id}", delete(delete_backup))
        .route("/api/backup/backups/{id}/describe", get(describe_backup))
        .route("/api/backup/backups/{id}/logs", get(backup_logs))
        .route("/api/backup/backups/{id}/gc", post(trigger_backup_gc))
        // Restores
        .route("/api/backup/restores", post(create_restore))
        .route("/api/backup/restores", get(list_restores))
        .route("/api/backup/restores/{id}", get(get_restore))
        .route("/api/backup/restores/{id}/logs", get(restore_logs))
        // Schedules
        .route("/api/backup/schedules", post(create_schedule))
        .route("/api/backup/schedules", get(list_schedules))
        .route("/api/backup/schedules/{id}", get(get_schedule))
        .route("/api/backup/schedules/{id}", delete(delete_schedule))
        .route("/api/backup/schedules/{id}/pause", post(pause_schedule))
        .route("/api/backup/schedules/{id}/unpause", post(unpause_schedule))
        // Storage locations
        .route("/api/backup/storage-locations", post(create_storage_location))
        .route("/api/backup/storage-locations", get(list_storage_locations))
        .route("/api/backup/storage-locations/{id}", get(get_storage_location))
        // Volume snapshot locations
        .route(
            "/api/backup/volume-snapshot-locations",
            post(create_volume_snapshot_location),
        )
        .route(
            "/api/backup/volume-snapshot-locations",
            get(list_volume_snapshot_locations),
        )
        .route(
            "/api/backup/volume-snapshot-locations/{id}",
            get(get_volume_snapshot_location),
        )
        // FS backup
        .route("/api/backup/fs-backup", post(create_fs_backup))
        .route("/api/backup/fs-backup", get(list_fs_backup))
        .route("/api/backup/fs-backup/{id}", get(get_fs_backup))
        // Server
        .route("/api/backup/server/status", get(server_status))
        // Health
        .route("/api/backup/health", get(health))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Backup handlers
// ---------------------------------------------------------------------------

async fn create_backup(
    State(state): State<Arc<BackupState>>,
    Json(req): Json<CreateBackupRequest>,
) -> Json<serde_json::Value> {
    let backup = crate::engine::create_backup(req.name, req.spec);
    let id = backup.id;
    tracing::info!(backup_id = %id, "created backup");
    let mut store = state.store.write().await;
    store.backups.insert(id, backup.clone());
    Json(serde_json::to_value(&backup).unwrap())
}

async fn list_backups(State(state): State<Arc<BackupState>>) -> Json<serde_json::Value> {
    let store = state.store.read().await;
    let list: Vec<_> = store.backups.values().collect();
    Json(serde_json::to_value(&list).unwrap())
}

async fn get_backup(
    State(state): State<Arc<BackupState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    match store.backups.get(&id) {
        Some(b) => (StatusCode::OK, Json(serde_json::to_value(b).unwrap())).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "backup not found"})),
        )
            .into_response(),
    }
}

async fn delete_backup(
    State(state): State<Arc<BackupState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    match store.backups.remove(&id) {
        Some(_) => (
            StatusCode::OK,
            Json(serde_json::json!({"deleted": id})),
        )
            .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "backup not found"})),
        )
            .into_response(),
    }
}

async fn describe_backup(
    State(state): State<Arc<BackupState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    match store.backups.get(&id) {
        Some(b) => {
            let description = serde_json::json!({
                "id": b.id,
                "name": b.name,
                "phase": b.phase,
                "storage_location": b.storage_location,
                "included_namespaces": b.included_namespaces,
                "excluded_namespaces": b.excluded_namespaces,
                "included_resources": b.included_resources,
                "excluded_resources": b.excluded_resources,
                "include_cluster_resources": b.include_cluster_resources,
                "ttl_hours": b.ttl_hours,
                "expires_at": b.expires_at,
                "started_at": b.started_at,
                "completed_at": b.completed_at,
                "items_backed_up": b.items_backed_up,
                "total_items": b.total_items,
                "warnings": b.warnings,
                "errors": b.errors,
                "size_bytes": b.size_bytes,
                "hooks_count": b.hooks.len(),
                "created_at": b.created_at,
            });
            (StatusCode::OK, Json(description)).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "backup not found"})),
        )
            .into_response(),
    }
}

async fn backup_logs(
    State(state): State<Arc<BackupState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    match store.backups.get(&id) {
        Some(b) => (StatusCode::OK, Json(serde_json::to_value(&b.logs).unwrap())).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "backup not found"})),
        )
            .into_response(),
    }
}

async fn trigger_backup_gc(
    State(state): State<Arc<BackupState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    match store.backups.get_mut(&id) {
        Some(b) => {
            crate::gc::mark_deleting(b);
            tracing::info!(backup_id = %id, "gc triggered for backup");
            (
                StatusCode::OK,
                Json(serde_json::json!({"gc": "triggered", "id": id})),
            )
                .into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "backup not found"})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Restore handlers
// ---------------------------------------------------------------------------

async fn create_restore(
    State(state): State<Arc<BackupState>>,
    Json(req): Json<CreateRestoreRequest>,
) -> impl IntoResponse {
    // Verify the referenced backup exists
    {
        let store = state.store.read().await;
        if !store.backups.contains_key(&req.backup_id) {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "referenced backup not found"})),
            )
                .into_response();
        }
    }

    let restore = crate::engine::create_restore(
        req.name,
        req.backup_id,
        req.backup_name,
        req.restore_pvs.unwrap_or(true),
        req.namespace_mappings.unwrap_or_default(),
        req.included_namespaces.unwrap_or_default(),
        req.excluded_namespaces.unwrap_or_default(),
        req.included_resources.unwrap_or_default(),
        req.excluded_resources.unwrap_or_default(),
        req.existing_resource_policy
            .unwrap_or(ExistingResourcePolicy::None),
    );
    let id = restore.id;
    tracing::info!(restore_id = %id, "created restore");
    let mut store = state.store.write().await;
    store.restores.insert(id, restore.clone());
    (StatusCode::CREATED, Json(serde_json::to_value(&restore).unwrap())).into_response()
}

async fn list_restores(State(state): State<Arc<BackupState>>) -> Json<serde_json::Value> {
    let store = state.store.read().await;
    let list: Vec<_> = store.restores.values().collect();
    Json(serde_json::to_value(&list).unwrap())
}

async fn get_restore(
    State(state): State<Arc<BackupState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    match store.restores.get(&id) {
        Some(r) => (StatusCode::OK, Json(serde_json::to_value(r).unwrap())).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "restore not found"})),
        )
            .into_response(),
    }
}

async fn restore_logs(
    State(state): State<Arc<BackupState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    match store.restores.get(&id) {
        Some(r) => (StatusCode::OK, Json(serde_json::to_value(&r.logs).unwrap())).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "restore not found"})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Schedule handlers
// ---------------------------------------------------------------------------

async fn create_schedule(
    State(state): State<Arc<BackupState>>,
    Json(req): Json<CreateScheduleRequest>,
) -> impl IntoResponse {
    if !crate::schedule::validate_cron(&req.cron_expression) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({
                "error": "invalid cron expression",
                "expression": req.cron_expression,
            })),
        )
            .into_response();
    }

    let now = Utc::now();
    let schedule = crate::models::Schedule {
        id: Uuid::new_v4(),
        name: req.name,
        cron_expression: req.cron_expression,
        template: req.template,
        paused: false,
        last_backup_at: None,
        last_backup_phase: None,
        created_at: now,
    };
    let id = schedule.id;
    tracing::info!(schedule_id = %id, "created schedule");
    let mut store = state.store.write().await;
    store.schedules.insert(id, schedule.clone());
    (StatusCode::CREATED, Json(serde_json::to_value(&schedule).unwrap())).into_response()
}

async fn list_schedules(State(state): State<Arc<BackupState>>) -> Json<serde_json::Value> {
    let store = state.store.read().await;
    let list: Vec<_> = store.schedules.values().collect();
    Json(serde_json::to_value(&list).unwrap())
}

async fn get_schedule(
    State(state): State<Arc<BackupState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    match store.schedules.get(&id) {
        Some(s) => (StatusCode::OK, Json(serde_json::to_value(s).unwrap())).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "schedule not found"})),
        )
            .into_response(),
    }
}

async fn delete_schedule(
    State(state): State<Arc<BackupState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    match store.schedules.remove(&id) {
        Some(_) => (StatusCode::OK, Json(serde_json::json!({"deleted": id}))).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "schedule not found"})),
        )
            .into_response(),
    }
}

async fn pause_schedule(
    State(state): State<Arc<BackupState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    match store.schedules.get_mut(&id) {
        Some(s) => {
            s.paused = true;
            (StatusCode::OK, Json(serde_json::json!({"paused": true, "id": id}))).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "schedule not found"})),
        )
            .into_response(),
    }
}

async fn unpause_schedule(
    State(state): State<Arc<BackupState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    match store.schedules.get_mut(&id) {
        Some(s) => {
            s.paused = false;
            (
                StatusCode::OK,
                Json(serde_json::json!({"paused": false, "id": id})),
            )
                .into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "schedule not found"})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Storage location handlers
// ---------------------------------------------------------------------------

async fn create_storage_location(
    State(state): State<Arc<BackupState>>,
    Json(req): Json<CreateBslRequest>,
) -> impl IntoResponse {
    let now = Utc::now();
    let bsl = BackupStorageLocation {
        id: Uuid::new_v4(),
        name: req.name,
        provider: req.provider,
        bucket: req.bucket,
        prefix: req.prefix,
        region: req.region,
        endpoint: req.endpoint,
        access_mode: req.access_mode.unwrap_or(BslAccessMode::ReadWrite),
        credential_secret: req.credential_secret,
        ca_bundle: None,
        insecure_skip_tls_verify: req.insecure_skip_tls_verify.unwrap_or(false),
        is_default: req.is_default.unwrap_or(false),
        phase: BslPhase::Available,
        last_validated_at: Some(now),
        created_at: now,
    };

    let errors = crate::storage::validate_bsl(&bsl);
    if !errors.is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"errors": errors})),
        )
            .into_response();
    }

    let id = bsl.id;
    tracing::info!(bsl_id = %id, "created storage location");
    let mut store = state.store.write().await;
    store.storage_locations.insert(id, bsl.clone());
    (StatusCode::CREATED, Json(serde_json::to_value(&bsl).unwrap())).into_response()
}

async fn list_storage_locations(
    State(state): State<Arc<BackupState>>,
) -> Json<serde_json::Value> {
    let store = state.store.read().await;
    let list: Vec<_> = store.storage_locations.values().collect();
    Json(serde_json::to_value(&list).unwrap())
}

async fn get_storage_location(
    State(state): State<Arc<BackupState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    match store.storage_locations.get(&id) {
        Some(bsl) => {
            (StatusCode::OK, Json(serde_json::to_value(bsl).unwrap())).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "storage location not found"})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Volume snapshot location handlers
// ---------------------------------------------------------------------------

async fn create_volume_snapshot_location(
    State(state): State<Arc<BackupState>>,
    Json(req): Json<CreateVslRequest>,
) -> impl IntoResponse {
    let now = Utc::now();
    let vsl = VolumeSnapshotLocation {
        id: Uuid::new_v4(),
        name: req.name,
        provider: req.provider,
        region: req.region,
        credential_secret: req.credential_secret,
        config: req.config.unwrap_or_default(),
        is_default: req.is_default.unwrap_or(false),
        created_at: now,
    };
    let id = vsl.id;
    tracing::info!(vsl_id = %id, "created volume snapshot location");
    let mut store = state.store.write().await;
    store.volume_snapshot_locations.insert(id, vsl.clone());
    (StatusCode::CREATED, Json(serde_json::to_value(&vsl).unwrap())).into_response()
}

async fn list_volume_snapshot_locations(
    State(state): State<Arc<BackupState>>,
) -> Json<serde_json::Value> {
    let store = state.store.read().await;
    let list: Vec<_> = store.volume_snapshot_locations.values().collect();
    Json(serde_json::to_value(&list).unwrap())
}

async fn get_volume_snapshot_location(
    State(state): State<Arc<BackupState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    match store.volume_snapshot_locations.get(&id) {
        Some(vsl) => {
            (StatusCode::OK, Json(serde_json::to_value(vsl).unwrap())).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "volume snapshot location not found"})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Filesystem backup handlers
// ---------------------------------------------------------------------------

async fn create_fs_backup(
    State(state): State<Arc<BackupState>>,
    Json(req): Json<CreateFsBackupRequest>,
) -> impl IntoResponse {
    // Verify the referenced backup exists
    {
        let store = state.store.read().await;
        if !store.backups.contains_key(&req.backup_id) {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "referenced backup not found"})),
            )
                .into_response();
        }
    }

    let job = crate::filesystem::create_fs_backup_job(
        req.backup_id,
        req.method,
        &req.namespace,
        &req.pod,
        &req.volume,
    );
    let id = job.id;
    tracing::info!(fs_job_id = %id, "created fs backup job");
    let mut store = state.store.write().await;
    store.fs_backup_jobs.insert(id, job.clone());
    (StatusCode::CREATED, Json(serde_json::to_value(&job).unwrap())).into_response()
}

async fn list_fs_backup(State(state): State<Arc<BackupState>>) -> Json<serde_json::Value> {
    let store = state.store.read().await;
    let list: Vec<_> = store.fs_backup_jobs.values().collect();
    Json(serde_json::to_value(&list).unwrap())
}

async fn get_fs_backup(
    State(state): State<Arc<BackupState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    match store.fs_backup_jobs.get(&id) {
        Some(j) => (StatusCode::OK, Json(serde_json::to_value(j).unwrap())).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "fs backup job not found"})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Server status + health
// ---------------------------------------------------------------------------

async fn server_status(State(state): State<Arc<BackupState>>) -> Json<ServerStatus> {
    let store = state.store.read().await;
    Json(ServerStatus {
        phase: ServerPhase::Available,
        plugins: vec![
            PluginInfo {
                name: "velero-plugin-for-aws".into(),
                kind: "ObjectStore".into(),
                version: "1.10.0".into(),
            },
            PluginInfo {
                name: "velero-plugin-for-gcp".into(),
                kind: "ObjectStore".into(),
                version: "1.10.0".into(),
            },
            PluginInfo {
                name: "velero-plugin-for-csi".into(),
                kind: "VolumeSnapshotter".into(),
                version: "0.7.0".into(),
            },
        ],
        storage_location_count: store.storage_locations.len(),
        server_version: "cave-backup/0.1.0".into(),
    })
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-backup",
        "status": "ok",
        "upstream": "Velero",
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Method, Request},
    };
    use tower::util::ServiceExt;

    fn test_state() -> Arc<BackupState> {
        Arc::new(BackupState::default())
    }

    async fn body_json(body: Body) -> serde_json::Value {
        let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn health_returns_ok() {
        let app = create_router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/backup/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn server_status_returns_available() {
        let app = create_router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/backup/server/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        assert_eq!(json["phase"], "available");
        assert!(json["plugins"].as_array().unwrap().len() > 0);
    }

    #[tokio::test]
    async fn create_and_get_backup() {
        let app = create_router(test_state());
        let body = serde_json::json!({
            "name": "test-backup",
            "spec": {
                "storage_location": "default",
                "included_namespaces": ["default"],
                "excluded_namespaces": [],
                "included_resources": [],
                "excluded_resources": [],
                "label_selector": {},
                "include_cluster_resources": false,
                "ttl_hours": 24,
                "hooks": [],
                "default_volumes_to_fs_backup": false,
            }
        });

        let create_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/backup/backups")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create_resp.status(), StatusCode::OK);
        let created = body_json(create_resp.into_body()).await;
        let id = created["id"].as_str().unwrap().to_string();
        assert_eq!(created["phase"], "in_progress");

        let get_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/api/backup/backups/{id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_resp.status(), StatusCode::OK);
        let fetched = body_json(get_resp.into_body()).await;
        assert_eq!(fetched["id"], id);
    }

    #[tokio::test]
    async fn get_nonexistent_backup_returns_404() {
        let app = create_router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/api/backup/backups/{}", Uuid::new_v4()))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn list_backups_empty() {
        let app = create_router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/backup/backups")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        assert!(json.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn create_schedule_invalid_cron_returns_422() {
        let app = create_router(test_state());
        let body = serde_json::json!({
            "name": "bad-sched",
            "cron_expression": "not-a-cron",
            "template": BackupSpec::default(),
        });
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/backup/schedules")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn list_storage_locations_includes_default() {
        let app = create_router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/backup/storage-locations")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        let locations = json.as_array().unwrap();
        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0]["name"], "default");
        assert_eq!(locations[0]["is_default"], true);
    }

    #[tokio::test]
    async fn delete_backup_removes_it() {
        let state = test_state();
        let app = create_router(state.clone());

        // Create
        let body = serde_json::json!({
            "name": "del-test",
            "spec": BackupSpec::default(),
        });
        let create_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/backup/backups")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let created = body_json(create_resp.into_body()).await;
        let id = created["id"].as_str().unwrap().to_string();

        // Delete
        let del_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri(format!("/api/backup/backups/{id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(del_resp.status(), StatusCode::OK);

        // Verify gone
        let get_resp = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/api/backup/backups/{id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn pause_unpause_schedule() {
        let state = test_state();
        let app = create_router(state.clone());

        let body = serde_json::json!({
            "name": "nightly",
            "cron_expression": "0 0 * * *",
            "template": BackupSpec::default(),
        });
        let create_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/backup/schedules")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create_resp.status(), StatusCode::CREATED);
        let created = body_json(create_resp.into_body()).await;
        let id = created["id"].as_str().unwrap().to_string();

        // Pause
        let pause_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(format!("/api/backup/schedules/{id}/pause"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(pause_resp.status(), StatusCode::OK);
        let paused_json = body_json(pause_resp.into_body()).await;
        assert_eq!(paused_json["paused"], true);

        // Unpause
        let unpause_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(format!("/api/backup/schedules/{id}/unpause"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unpause_resp.status(), StatusCode::OK);
        let unpaused_json = body_json(unpause_resp.into_body()).await;
        assert_eq!(unpaused_json["paused"], false);
    }
}
