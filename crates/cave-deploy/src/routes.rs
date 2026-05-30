// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP API routes for cave-deploy.

use crate::{DeployState, appset::ApplicationSet, models::*, rbac::AppProject};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post},
};
use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router(state: Arc<DeployState>) -> Router {
    Router::new()
        // Application CRUD
        .route(
            "/api/deploy/apps",
            get(list_applications).post(create_application),
        )
        .route(
            "/api/deploy/apps/{name}",
            get(get_application)
                .put(update_application)
                .delete(delete_application),
        )
        // Sync operations
        .route("/api/deploy/apps/{name}/sync", post(sync_application))
        .route("/api/deploy/apps/{name}/refresh", post(refresh_application))
        .route(
            "/api/deploy/apps/{name}/rollback",
            post(rollback_application),
        )
        .route("/api/deploy/apps/{name}/diff", get(diff_application))
        .route("/api/deploy/apps/{name}/history", get(get_revision_history))
        // ApplicationSet
        .route("/api/deploy/appsets", get(list_appsets).post(create_appset))
        .route(
            "/api/deploy/appsets/{name}",
            get(get_appset).delete(delete_appset),
        )
        // Projects
        .route(
            "/api/deploy/projects",
            get(list_projects).post(create_project),
        )
        .route(
            "/api/deploy/projects/{name}",
            get(get_project).put(update_project).delete(delete_project),
        )
        // Repository credentials
        .route("/api/deploy/repos", get(list_repos).post(add_repo))
        .route("/api/deploy/repos/{id}", delete(remove_repo))
        // Notifications
        .route(
            "/api/deploy/notifications",
            get(list_notifications).post(create_notification),
        )
        // SSO
        .route(
            "/api/deploy/sso/config",
            get(get_sso_config).put(update_sso_config),
        )
        // Webhook
        .route("/api/deploy/webhook", post(handle_webhook))
        // Helm dependency resolution (umbrella Chart.lock)
        .route("/api/deploy/helm/resolve", post(resolve_helm_deps))
        // Sync-window evaluation (maintenance windows)
        .route("/api/deploy/sync-windows/evaluate", post(evaluate_sync_windows))
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

async fn list_applications(State(_state): State<Arc<DeployState>>) -> Json<serde_json::Value> {
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
#[allow(dead_code)]
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
#[allow(dead_code)]
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

async fn list_appsets(State(_state): State<Arc<DeployState>>) -> Json<serde_json::Value> {
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
    Err((
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": format!("AppSet '{}' not found", name) })),
    ))
}

async fn delete_appset(
    State(_state): State<Arc<DeployState>>,
    Path(_name): Path<String>,
) -> StatusCode {
    StatusCode::NO_CONTENT
}

// ─── Projects ────────────────────────────────────────────────────────────────

async fn list_projects(State(_state): State<Arc<DeployState>>) -> Json<serde_json::Value> {
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
    Err((
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": format!("Project '{}' not found", name) })),
    ))
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

async fn list_repos(State(_state): State<Arc<DeployState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "repos": [] }))
}

async fn add_repo(
    State(_state): State<Arc<DeployState>>,
    Json(cred): Json<RepositoryCredential>,
) -> (StatusCode, Json<RepositoryCredential>) {
    (StatusCode::CREATED, Json(cred))
}

async fn remove_repo(State(_state): State<Arc<DeployState>>, Path(_id): Path<Uuid>) -> StatusCode {
    StatusCode::NO_CONTENT
}

// ─── Notifications ────────────────────────────────────────────────────────────

async fn list_notifications(State(_state): State<Arc<DeployState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "notifications": [] }))
}

async fn create_notification(
    State(_state): State<Arc<DeployState>>,
    Json(notif): Json<NotificationConfig>,
) -> (StatusCode, Json<NotificationConfig>) {
    (StatusCode::CREATED, Json(notif))
}

// ─── SSO ─────────────────────────────────────────────────────────────────────

async fn get_sso_config(State(_state): State<Arc<DeployState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "configured": false }))
}

async fn update_sso_config(
    State(_state): State<Arc<DeployState>>,
    Json(config): Json<SSOConfig>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "updated", "provider": format!("{:?}", config.provider) }))
}

// ─── Helm dependency resolution ──────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct HelmResolveReq {
    /// Raw `Chart.yaml` text of the umbrella chart.
    chart_yaml: String,
    /// chart name → published versions, as scraped from each Helm repo index.
    #[serde(default)]
    available: std::collections::HashMap<String, Vec<String>>,
}

async fn resolve_helm_deps(
    State(_state): State<Arc<DeployState>>,
    Json(req): Json<HelmResolveReq>,
) -> Result<Json<serde_json::Value>, crate::DeployError> {
    let chart = crate::helm_deps::parse_chart_yaml(&req.chart_yaml)?;
    let lock = crate::helm_deps::generate_lock(&chart, &req.available, &Utc::now().to_rfc3339())?;
    Ok(Json(serde_json::json!({
        "chart": chart.name,
        "version": chart.version,
        "dependencies": lock.dependencies.iter().map(|d| serde_json::json!({
            "name": d.name,
            "repository": d.repository,
            "version": d.version,
        })).collect::<Vec<_>>(),
        "digest": lock.digest,
        "generated": lock.generated,
    })))
}

// ─── Sync-window evaluation ───────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct SyncWindowEvalReq {
    windows: Vec<crate::rbac::SyncWindow>,
    #[serde(default)]
    is_manual: bool,
    /// Optional app coordinates to first filter applicable windows.
    app_name: Option<String>,
    namespace: Option<String>,
    cluster: Option<String>,
}

async fn evaluate_sync_windows(
    State(_state): State<Arc<DeployState>>,
    Json(req): Json<SyncWindowEvalReq>,
) -> Json<serde_json::Value> {
    let now = Utc::now();
    let applicable: Vec<crate::rbac::SyncWindow> = match (
        req.app_name.as_deref(),
        req.namespace.as_deref(),
        req.cluster.as_deref(),
    ) {
        (None, None, None) => req.windows.clone(),
        (a, n, c) => crate::sync_windows::matching_windows(
            &req.windows,
            a.unwrap_or(""),
            n.unwrap_or(""),
            c.unwrap_or(""),
        )
        .into_iter()
        .cloned()
        .collect(),
    };
    let active: Vec<usize> = applicable
        .iter()
        .enumerate()
        .filter(|(_, w)| crate::sync_windows::window_active(w, now))
        .map(|(i, _)| i)
        .collect();
    let can = crate::sync_windows::can_sync(&applicable, now, req.is_manual);
    Json(serde_json::json!({
        "can_sync": can,
        "is_manual": req.is_manual,
        "applicable_windows": applicable.len(),
        "active_window_indexes": active,
    }))
}

// ─── Webhook ─────────────────────────────────────────────────────────────────

async fn handle_webhook(
    State(_state): State<Arc<DeployState>>,
    Json(_payload): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    tracing::info!("Git webhook received");
    Json(serde_json::json!({ "status": "received", "refresh_triggered": true }))
}
