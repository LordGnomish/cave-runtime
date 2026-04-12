//! HTTP API routes for cave-deploy.

use crate::{
    appset::ApplicationSet,
    models::*,
    rbac::AppProject,
    sync::{RollbackRequest, SyncRequest, SyncStrategy},
    DeployState,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post, put},
    Json, Router,
};
use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router(state: Arc<DeployState>) -> Router {
    Router::new()
        // Application CRUD
        .route("/api/deploy/apps", get(list_applications).post(create_application))
        .route("/api/deploy/apps/{name}", get(get_application).put(update_application).delete(delete_application))
        // Sync operations
        .route("/api/deploy/apps/{name}/sync", post(sync_application))
        .route("/api/deploy/apps/{name}/refresh", post(refresh_application))
        .route("/api/deploy/apps/{name}/rollback", post(rollback_application))
        .route("/api/deploy/apps/{name}/diff", get(diff_application))
        .route("/api/deploy/apps/{name}/history", get(get_revision_history))
        // ApplicationSet
        .route("/api/deploy/appsets", get(list_appsets).post(create_appset))
        .route("/api/deploy/appsets/{name}", get(get_appset).delete(delete_appset))
        // Projects
        .route("/api/deploy/projects", get(list_projects).post(create_project))
        .route("/api/deploy/projects/{name}", get(get_project).put(update_project).delete(delete_project))
        // Repository credentials
        .route("/api/deploy/repos", get(list_repos).post(add_repo))
        .route("/api/deploy/repos/{id}", delete(remove_repo))
        // Notifications
        .route("/api/deploy/notifications", get(list_notifications).post(create_notification))
        // SSO
        .route("/api/deploy/sso/config", get(get_sso_config).put(update_sso_config))
        // Webhook
        .route("/api/deploy/webhook", post(handle_webhook))
        // Health
        .route("/api/deploy/health", get(health))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-deploy",
        "status": "ok",
        "upstream": ["ArgoCD", "Flux"]
    }))
}

// ─── Application CRUD ────────────────────────────────────────────────────────

async fn list_applications(
    State(_state): State<Arc<DeployState>>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "items": [], "total": 0 }))
}

async fn create_application(
    State(_state): State<Arc<DeployState>>,
    Json(spec): Json<ApplicationSpec>,
) -> (StatusCode, Json<Application>) {
    let app = Application {
        id: Uuid::new_v4(),
        name: format!("app-{}", Uuid::new_v4()),
        namespace: "argocd".to_string(),
        spec,
        status: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        labels: Default::default(),
        annotations: Default::default(),
        tracking: ResourceTracking::default(),
    };
    (StatusCode::CREATED, Json(app))
}

async fn get_application(
    State(_state): State<Arc<DeployState>>,
    Path(name): Path<String>,
) -> Result<Json<Application>, (StatusCode, Json<serde_json::Value>)> {
    Err((
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": format!("Application '{}' not found", name) })),
    ))
}

async fn update_application(
    State(_state): State<Arc<DeployState>>,
    Path(_name): Path<String>,
    Json(_spec): Json<ApplicationSpec>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "updated" }))
}

async fn delete_application(
    State(_state): State<Arc<DeployState>>,
    Path(_name): Path<String>,
) -> StatusCode {
    StatusCode::NO_CONTENT
}

// ─── Sync operations ─────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct SyncReq {
    revision: Option<String>,
    #[serde(default)]
    dry_run: bool,
    #[serde(default)]
    prune: bool,
    #[serde(default)]
    force: bool,
}

async fn sync_application(
    State(_state): State<Arc<DeployState>>,
    Path(name): Path<String>,
    Json(req): Json<SyncReq>,
) -> Json<serde_json::Value> {
    tracing::info!(app = %name, revision = ?req.revision, dry_run = req.dry_run, "Sync requested");
    Json(serde_json::json!({
        "operation_id": Uuid::new_v4(),
        "status": "running",
        "dry_run": req.dry_run
    }))
}

async fn refresh_application(
    State(_state): State<Arc<DeployState>>,
    Path(name): Path<String>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "refresh triggered", "app": name }))
}

#[derive(serde::Deserialize)]
struct RollbackReq {
    history_id: u64,
    #[serde(default)]
    prune: bool,
    #[serde(default)]
    dry_run: bool,
}

async fn rollback_application(
    State(_state): State<Arc<DeployState>>,
    Path(name): Path<String>,
    Json(req): Json<RollbackReq>,
) -> Json<serde_json::Value> {
    tracing::info!(app = %name, history_id = req.history_id, "Rollback requested");
    Json(serde_json::json!({
        "operation_id": Uuid::new_v4(),
        "status": "rollback initiated",
        "history_id": req.history_id
    }))
}

async fn diff_application(
    State(_state): State<Arc<DeployState>>,
    Path(name): Path<String>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "app": name,
        "in_sync": true,
        "diffs": [],
        "normalized_desired": null,
        "normalized_live": null
    }))
}

async fn get_revision_history(
    State(_state): State<Arc<DeployState>>,
    Path(name): Path<String>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "app": name, "history": [] }))
}

// ─── ApplicationSet ───────────────────────────────────────────────────────────

async fn list_appsets(
    State(_state): State<Arc<DeployState>>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "items": [], "total": 0 }))
}

async fn create_appset(
    State(_state): State<Arc<DeployState>>,
    Json(appset): Json<ApplicationSet>,
) -> (StatusCode, Json<ApplicationSet>) {
    (StatusCode::CREATED, Json(appset))
}

async fn get_appset(
    State(_state): State<Arc<DeployState>>,
    Path(name): Path<String>,
) -> Result<Json<ApplicationSet>, (StatusCode, Json<serde_json::Value>)> {
    Err((StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": format!("AppSet '{}' not found", name) }))))
}

async fn delete_appset(
    State(_state): State<Arc<DeployState>>,
    Path(_name): Path<String>,
) -> StatusCode {
    StatusCode::NO_CONTENT
}

// ─── Projects ────────────────────────────────────────────────────────────────

async fn list_projects(
    State(_state): State<Arc<DeployState>>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "items": [] }))
}

async fn create_project(
    State(_state): State<Arc<DeployState>>,
    Json(project): Json<AppProject>,
) -> (StatusCode, Json<AppProject>) {
    (StatusCode::CREATED, Json(project))
}

async fn get_project(
    State(_state): State<Arc<DeployState>>,
    Path(name): Path<String>,
) -> Result<Json<AppProject>, (StatusCode, Json<serde_json::Value>)> {
    Err((StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": format!("Project '{}' not found", name) }))))
}

async fn update_project(
    State(_state): State<Arc<DeployState>>,
    Path(_name): Path<String>,
    Json(_project): Json<AppProject>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "updated" }))
}

async fn delete_project(
    State(_state): State<Arc<DeployState>>,
    Path(_name): Path<String>,
) -> StatusCode {
    StatusCode::NO_CONTENT
}

// ─── Repository credentials ──────────────────────────────────────────────────

async fn list_repos(
    State(_state): State<Arc<DeployState>>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "repos": [] }))
}

async fn add_repo(
    State(_state): State<Arc<DeployState>>,
    Json(cred): Json<RepositoryCredential>,
) -> (StatusCode, Json<RepositoryCredential>) {
    (StatusCode::CREATED, Json(cred))
}

async fn remove_repo(
    State(_state): State<Arc<DeployState>>,
    Path(_id): Path<Uuid>,
) -> StatusCode {
    StatusCode::NO_CONTENT
}

// ─── Notifications ────────────────────────────────────────────────────────────

async fn list_notifications(
    State(_state): State<Arc<DeployState>>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "notifications": [] }))
}

async fn create_notification(
    State(_state): State<Arc<DeployState>>,
    Json(notif): Json<NotificationConfig>,
) -> (StatusCode, Json<NotificationConfig>) {
    (StatusCode::CREATED, Json(notif))
}

// ─── SSO ─────────────────────────────────────────────────────────────────────

async fn get_sso_config(
    State(_state): State<Arc<DeployState>>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "configured": false }))
}

async fn update_sso_config(
    State(_state): State<Arc<DeployState>>,
    Json(config): Json<SSOConfig>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "updated", "provider": format!("{:?}", config.provider) }))
}

// ─── Webhook ─────────────────────────────────────────────────────────────────

async fn handle_webhook(
    State(_state): State<Arc<DeployState>>,
    Json(payload): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    tracing::info!("Git webhook received");
    Json(serde_json::json!({ "status": "received", "refresh_triggered": true }))
}
