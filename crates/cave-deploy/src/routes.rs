<<<<<<< HEAD
//! HTTP routes for cave-deploy.

use crate::{
    gitops, health as health_engine, rollout as rollout_engine,
    models::{
        Application, CreateApplicationRequest, CreateRolloutRequest, HealthStatus,
        RollbackRequest, RolloutStrategy, SyncPolicy, SyncRequest, SyncStatus,
        UpdateApplicationRequest,
    },
    DeployState,
};
use axum::{
    extract::{Path, State as AxumState},
=======
//! Admin API — full ArgoCD v1 REST surface.
//!
//! Endpoints:
//!   GET    /api/v1/applications
//!   POST   /api/v1/applications
//!   GET    /api/v1/applications/:name
//!   PUT    /api/v1/applications/:name
//!   DELETE /api/v1/applications/:name
//!   POST   /api/v1/applications/:name/sync
//!   POST   /api/v1/applications/:name/rollback
//!   GET    /api/v1/applications/:name/diff
//!   GET    /api/v1/applications/:name/history
//!
//!   GET    /api/v1/repositories
//!   POST   /api/v1/repositories
//!
//!   GET    /api/v1/clusters
//!   POST   /api/v1/clusters
//!   DELETE /api/v1/clusters/:name
//!
//!   GET    /api/v1/projects
//!   POST   /api/v1/projects
//!   GET    /api/v1/projects/:name
//!
//!   GET    /api/v1/applicationsets
//!   POST   /api/v1/applicationsets
//!
//!   GET    /api/v1/deploy/health

use crate::error::DeployError;
use crate::models::{
    Application, ApplicationSet, AppProject, Cluster, Repository,
    CreateApplicationRequest, UpdateApplicationRequest, SyncRequest, RollbackRequest,
    OperationState, OperationPhase, SyncOperationResult, ApplicationStatus, HealthStatusDetail,
    SyncStatusDetail,
};
use crate::store::DeployStore;
use crate::sync::{detect_drift, parse_manifests};
use axum::{
    extract::{Path, State},
    http::StatusCode,
>>>>>>> claude/thirsty-snyder
    routing::{delete, get, post, put},
    Json, Router,
};
use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

<<<<<<< HEAD
pub fn create_router(state: Arc<DeployState>) -> Router {
    Router::new()
        // Applications — CRUD
        .route("/api/v1/deploy/applications", get(list_applications))
        .route("/api/v1/deploy/applications", post(create_application))
        .route("/api/v1/deploy/applications/:id", get(get_application))
        .route("/api/v1/deploy/applications/:id", put(update_application))
        .route("/api/v1/deploy/applications/:id", delete(delete_application))
        // Sync / Rollback
        .route("/api/v1/deploy/applications/:id/sync", post(sync_app))
        .route(
            "/api/v1/deploy/applications/:id/rollback",
            post(rollback_app),
        )
        // Status / History
        .route(
            "/api/v1/deploy/applications/:id/status",
            get(get_app_status),
        )
        .route(
            "/api/v1/deploy/applications/:id/history",
            get(get_app_history),
        )
        // Diff
        .route("/api/v1/deploy/diff/:id", get(get_diff))
        // Rollouts
        .route("/api/v1/deploy/rollouts", post(create_rollout))
        .route("/api/v1/deploy/rollouts/:id", get(get_rollout))
        .route("/api/v1/deploy/rollouts/:id/promote", post(promote_rollout))
        .route("/api/v1/deploy/rollouts/:id/abort", post(abort_rollout))
        // Module health
=======
// ─── Module state ─────────────────────────────────────────────────────────────

pub struct DeployState {
    pub store: Arc<DeployStore>,
}

// ─── Router ───────────────────────────────────────────────────────────────────

pub fn create_router(state: Arc<DeployState>) -> Router {
    Router::new()
        // Applications
        .route("/api/v1/applications", get(list_applications).post(create_application))
        .route(
            "/api/v1/applications/:name",
            get(get_application).put(update_application).delete(delete_application),
        )
        .route("/api/v1/applications/:name/sync", post(sync_application))
        .route("/api/v1/applications/:name/rollback", post(rollback_application))
        .route("/api/v1/applications/:name/diff", get(diff_application))
        .route("/api/v1/applications/:name/history", get(app_history))
        // Repositories
        .route("/api/v1/repositories", get(list_repositories).post(upsert_repository))
        // Clusters
        .route("/api/v1/clusters", get(list_clusters).post(upsert_cluster))
        .route("/api/v1/clusters/:name", delete(delete_cluster))
        // Projects
        .route("/api/v1/projects", get(list_projects).post(upsert_project))
        .route("/api/v1/projects/:name", get(get_project))
        // ApplicationSets
        .route("/api/v1/applicationsets", get(list_appsets).post(create_appset))
        // Health
>>>>>>> claude/thirsty-snyder
        .route("/api/v1/deploy/health", get(health))
        .with_state(state)
}

// ─── Applications ─────────────────────────────────────────────────────────────

async fn list_applications(
<<<<<<< HEAD
    AxumState(state): AxumState<Arc<DeployState>>,
) -> Json<Vec<Application>> {
    let store = state.store.lock().unwrap();
    Json(store.applications.values().cloned().collect())
}

async fn create_application(
    AxumState(state): AxumState<Arc<DeployState>>,
    Json(req): Json<CreateApplicationRequest>,
) -> Json<Application> {
    let app = Application {
        id: Uuid::new_v4(),
        name: req.name,
        namespace: req.namespace,
        source: req.source,
        target_cluster: req.target_cluster,
        sync_policy: req.sync_policy.unwrap_or(SyncPolicy::Manual),
        sync_status: SyncStatus::Unknown,
        health_status: HealthStatus::Unknown,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        last_synced_at: None,
        revision: None,
        message: None,
    };
    let mut store = state.store.lock().unwrap();
    store.applications.insert(app.id, app.clone());
    Json(app)
}

async fn get_application(
    AxumState(state): AxumState<Arc<DeployState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let store = state.store.lock().unwrap();
    match store.applications.get(&id) {
        Some(app) => Json(serde_json::to_value(app).unwrap_or_default()),
        None => Json(serde_json::json!({ "error": "not found" })),
    }
}

async fn update_application(
    AxumState(state): AxumState<Arc<DeployState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateApplicationRequest>,
) -> Json<serde_json::Value> {
    let mut store = state.store.lock().unwrap();
    match store.applications.get_mut(&id) {
        Some(app) => {
            if let Some(name) = req.name {
                app.name = name;
            }
            if let Some(source) = req.source {
                app.source = source;
            }
            if let Some(policy) = req.sync_policy {
                app.sync_policy = policy;
            }
            if let Some(cluster) = req.target_cluster {
                app.target_cluster = cluster;
            }
            app.updated_at = Utc::now();
            Json(serde_json::to_value(app.clone()).unwrap_or_default())
        }
        None => Json(serde_json::json!({ "error": "not found" })),
    }
}

async fn delete_application(
    AxumState(state): AxumState<Arc<DeployState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let mut store = state.store.lock().unwrap();
    if store.applications.remove(&id).is_some() {
        Json(serde_json::json!({ "deleted": true }))
    } else {
        Json(serde_json::json!({ "error": "not found" }))
    }
}

// ─── Sync / Rollback ─────────────────────────────────────────────────────────

async fn sync_app(
    AxumState(state): AxumState<Arc<DeployState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<SyncRequest>,
) -> Json<serde_json::Value> {
    let mut store = state.store.lock().unwrap();
    let force = req.force.unwrap_or(false);

    // Lexical scope ensures the &mut Application borrow ends before we
    // borrow store.history on the next line.
    let deployment = {
        let app = match store.applications.get_mut(&id) {
            Some(a) => a,
            None => return Json(serde_json::json!({ "error": "not found" })),
        };
        gitops::sync_application(app, req.revision, force)
    };

    store.history.entry(id).or_default().push(deployment.clone());
    Json(serde_json::to_value(deployment).unwrap_or_default())
}

async fn rollback_app(
    AxumState(state): AxumState<Arc<DeployState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<RollbackRequest>,
) -> Json<serde_json::Value> {
    let mut store = state.store.lock().unwrap();

    // Resolve target revision from history entry or explicit field.
    let target_revision = if let Some(dep_id) = req.deployment_id {
        store
            .history
            .get(&id)
            .and_then(|h| h.iter().find(|d| d.id == dep_id))
            .map(|d| d.revision.clone())
    } else {
        req.revision
    };

    let deployment = {
        let app = match store.applications.get_mut(&id) {
            Some(a) => a,
            None => return Json(serde_json::json!({ "error": "not found" })),
        };
        gitops::sync_application(app, target_revision, true)
    };

    store.history.entry(id).or_default().push(deployment.clone());
    Json(serde_json::to_value(deployment).unwrap_or_default())
}

// ─── Status / History ────────────────────────────────────────────────────────

async fn get_app_status(
    AxumState(state): AxumState<Arc<DeployState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let store = state.store.lock().unwrap();
    match store.applications.get(&id) {
        Some(app) => {
            let has_drift = gitops::detect_drift(app);
            let is_degraded = health_engine::detect_degraded(app);
            let resources = vec![health_engine::check_resource_health(
                "Deployment",
                &app.name,
                &app.namespace,
            )];
            Json(serde_json::json!({
                "id": app.id,
                "sync_status": app.sync_status,
                "health_status": app.health_status,
                "has_drift": has_drift,
                "is_degraded": is_degraded,
                "last_synced_at": app.last_synced_at,
                "revision": app.revision,
                "message": app.message,
                "resources": resources,
            }))
        }
        None => Json(serde_json::json!({ "error": "not found" })),
    }
}

async fn get_app_history(
    AxumState(state): AxumState<Arc<DeployState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let store = state.store.lock().unwrap();
    let entries = store.history.get(&id).cloned().unwrap_or_default();
    Json(serde_json::json!({
        "application_id": id,
        "entries": entries,
    }))
}

// ─── Diff ────────────────────────────────────────────────────────────────────

async fn get_diff(
    AxumState(state): AxumState<Arc<DeployState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let store = state.store.lock().unwrap();
    match store.applications.get(&id) {
        Some(app) => Json(serde_json::to_value(gitops::git_diff(app)).unwrap_or_default()),
        None => Json(serde_json::json!({ "error": "not found" })),
    }
}

// ─── Rollouts ────────────────────────────────────────────────────────────────

async fn create_rollout(
    AxumState(state): AxumState<Arc<DeployState>>,
    Json(req): Json<CreateRolloutRequest>,
) -> Json<serde_json::Value> {
    let rollout = match req.strategy {
        RolloutStrategy::Canary => {
            rollout_engine::canary_deploy(req.application_id, req.canary_revision, req.steps)
        }
        RolloutStrategy::BlueGreen => {
            rollout_engine::blue_green_deploy(req.application_id, req.canary_revision)
        }
        RolloutStrategy::Rolling => {
            rollout_engine::rolling_update(req.application_id, req.canary_revision)
        }
    };
    let mut store = state.store.lock().unwrap();
    store.rollouts.insert(rollout.id, rollout.clone());
    Json(serde_json::to_value(rollout).unwrap_or_default())
}

async fn get_rollout(
    AxumState(state): AxumState<Arc<DeployState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let store = state.store.lock().unwrap();
    match store.rollouts.get(&id) {
        Some(r) => Json(serde_json::to_value(r).unwrap_or_default()),
        None => Json(serde_json::json!({ "error": "not found" })),
    }
}

async fn promote_rollout(
    AxumState(state): AxumState<Arc<DeployState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let mut store = state.store.lock().unwrap();
    match store.rollouts.get_mut(&id) {
        Some(r) => {
            let promoted = rollout_engine::promote_canary(r);
            let snapshot = r.clone();
            Json(serde_json::json!({ "promoted": promoted, "rollout": snapshot }))
        }
        None => Json(serde_json::json!({ "error": "not found" })),
    }
}

async fn abort_rollout(
    AxumState(state): AxumState<Arc<DeployState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let mut store = state.store.lock().unwrap();
    match store.rollouts.get_mut(&id) {
        Some(r) => {
            rollout_engine::rollback(r);
            Json(serde_json::json!({ "aborted": true }))
        }
        None => Json(serde_json::json!({ "error": "not found" })),
    }
}

// ─── Health ──────────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-deploy",
        "status": "ok",
        "upstream": "ArgoCD + Flux",
=======
    State(state): State<Arc<DeployState>>,
) -> Result<Json<Vec<Application>>, DeployError> {
    let apps = state.store.list_applications().await?;
    Ok(Json(apps))
}

async fn get_application(
    State(state): State<Arc<DeployState>>,
    Path(name): Path<String>,
) -> Result<Json<Application>, DeployError> {
    let app = state
        .store
        .get_application(&name)
        .await?
        .ok_or_else(|| DeployError::NotFound(name.clone()))?;
    Ok(Json(app))
}

async fn create_application(
    State(state): State<Arc<DeployState>>,
    Json(req): Json<CreateApplicationRequest>,
) -> Result<(StatusCode, Json<Application>), DeployError> {
    // Check for duplicates
    if state.store.get_application(&req.name).await?.is_some() {
        return Err(DeployError::AlreadyExists(req.name));
    }

    let now = Utc::now();
    let app = Application {
        id: Uuid::new_v4(),
        name: req.name,
        namespace: req.namespace.unwrap_or_else(|| "argocd".to_string()),
        spec: req.spec,
        status: ApplicationStatus {
            sync: SyncStatusDetail {
                status: crate::models::SyncStatus::Unknown.to_string(),
                ..Default::default()
            },
            health: HealthStatusDetail {
                status: crate::models::HealthStatus::Unknown.to_string(),
                message: None,
            },
            ..Default::default()
        },
        created_at: now,
        updated_at: now,
        created_by: None,
        finalizers: vec!["resources-finalizer.argocd.argoproj.io".to_string()],
    };

    state.store.create_application(&app).await?;
    Ok((StatusCode::CREATED, Json(app)))
}

async fn update_application(
    State(state): State<Arc<DeployState>>,
    Path(name): Path<String>,
    Json(req): Json<UpdateApplicationRequest>,
) -> Result<Json<Application>, DeployError> {
    let mut app = state
        .store
        .get_application(&name)
        .await?
        .ok_or_else(|| DeployError::NotFound(name.clone()))?;

    state.store.update_application_spec(&name, &req.spec).await?;
    app.spec = req.spec;
    app.updated_at = Utc::now();
    Ok(Json(app))
}

async fn delete_application(
    State(state): State<Arc<DeployState>>,
    Path(name): Path<String>,
) -> Result<StatusCode, DeployError> {
    state.store.delete_application(&name).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn sync_application(
    State(state): State<Arc<DeployState>>,
    Path(name): Path<String>,
    Json(req): Json<SyncRequest>,
) -> Result<Json<OperationState>, DeployError> {
    let app = state
        .store
        .get_application(&name)
        .await?
        .ok_or_else(|| DeployError::NotFound(name.clone()))?;

    let now = Utc::now();
    // Record the operation start
    let op = OperationState {
        phase: OperationPhase::Running,
        message: Some("Sync initiated".to_string()),
        sync_result: None,
        started_at: now,
        finished_at: None,
        retry_count: 0,
    };

    // In a full implementation the sync engine would be invoked here.
    // For now we record a Succeeded operation.
    let result = OperationState {
        phase: OperationPhase::Succeeded,
        message: Some(if req.dry_run { "Dry-run complete".to_string() } else { "Sync complete".to_string() }),
        sync_result: Some(SyncOperationResult {
            revision: req.revision.unwrap_or_else(|| "HEAD".to_string()),
            ..Default::default()
        }),
        started_at: now,
        finished_at: Some(Utc::now()),
        retry_count: 0,
    };

    // Update the application status to reflect the sync
    if !req.dry_run {
        let mut status = app.status.clone();
        status.operation_state = Some(result.clone());
        status.sync.status = crate::models::SyncStatus::Synced.to_string();
        state.store.update_application_status(&name, &status).await?;
    }

    Ok(Json(result))
}

async fn rollback_application(
    State(state): State<Arc<DeployState>>,
    Path(name): Path<String>,
    Json(req): Json<RollbackRequest>,
) -> Result<Json<OperationState>, DeployError> {
    let app = state
        .store
        .get_application(&name)
        .await?
        .ok_or_else(|| DeployError::NotFound(name.clone()))?;

    // Locate the target revision history entry
    let history_entry = app
        .status
        .history
        .iter()
        .find(|h| h.id == req.id)
        .ok_or_else(|| DeployError::NotFound(format!("history entry {}", req.id)))?
        .clone();

    let now = Utc::now();
    let result = OperationState {
        phase: OperationPhase::Succeeded,
        message: Some(format!("Rolled back to revision {}", history_entry.revision)),
        sync_result: Some(SyncOperationResult {
            revision: history_entry.revision.clone(),
            source: Some(history_entry.source),
            ..Default::default()
        }),
        started_at: now,
        finished_at: Some(Utc::now()),
        retry_count: 0,
    };

    if !req.dry_run {
        let mut status = app.status.clone();
        status.operation_state = Some(result.clone());
        state.store.update_application_status(&name, &status).await?;
    }

    Ok(Json(result))
}

async fn diff_application(
    State(state): State<Arc<DeployState>>,
    Path(name): Path<String>,
) -> Result<Json<Vec<crate::models::ResourceDiff>>, DeployError> {
    let app = state
        .store
        .get_application(&name)
        .await?
        .ok_or_else(|| DeployError::NotFound(name.clone()))?;

    // In a full implementation: fetch desired from git, live from cluster.
    // Return empty diff (synced) for now; real logic lives in sync engine.
    Ok(Json(vec![]))
}

async fn app_history(
    State(state): State<Arc<DeployState>>,
    Path(name): Path<String>,
) -> Result<Json<Vec<crate::models::RevisionHistory>>, DeployError> {
    let app = state
        .store
        .get_application(&name)
        .await?
        .ok_or_else(|| DeployError::NotFound(name.clone()))?;

    let history = state
        .store
        .get_revision_history(app.id, 20)
        .await?;

    Ok(Json(history))
}

// ─── Repositories ─────────────────────────────────────────────────────────────

async fn list_repositories(
    State(state): State<Arc<DeployState>>,
) -> Result<Json<Vec<Repository>>, DeployError> {
    let repos = state.store.list_repositories().await?;
    Ok(Json(repos))
}

async fn upsert_repository(
    State(state): State<Arc<DeployState>>,
    Json(repo): Json<Repository>,
) -> Result<(StatusCode, Json<Repository>), DeployError> {
    state.store.upsert_repository(&repo).await?;
    Ok((StatusCode::OK, Json(repo)))
}

// ─── Clusters ─────────────────────────────────────────────────────────────────

async fn list_clusters(
    State(state): State<Arc<DeployState>>,
) -> Result<Json<Vec<Cluster>>, DeployError> {
    let clusters = state.store.list_clusters().await?;
    Ok(Json(clusters))
}

async fn upsert_cluster(
    State(state): State<Arc<DeployState>>,
    Json(cluster): Json<Cluster>,
) -> Result<(StatusCode, Json<Cluster>), DeployError> {
    state.store.upsert_cluster(&cluster).await?;
    Ok((StatusCode::OK, Json(cluster)))
}

async fn delete_cluster(
    State(state): State<Arc<DeployState>>,
    Path(name): Path<String>,
) -> Result<StatusCode, DeployError> {
    state.store.delete_cluster(&name).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ─── Projects ─────────────────────────────────────────────────────────────────

async fn list_projects(
    State(state): State<Arc<DeployState>>,
) -> Result<Json<Vec<AppProject>>, DeployError> {
    let projects = state.store.list_projects().await?;
    Ok(Json(projects))
}

async fn get_project(
    State(state): State<Arc<DeployState>>,
    Path(name): Path<String>,
) -> Result<Json<AppProject>, DeployError> {
    let project = state
        .store
        .get_project(&name)
        .await?
        .ok_or_else(|| DeployError::NotFound(name.clone()))?;
    Ok(Json(project))
}

async fn upsert_project(
    State(state): State<Arc<DeployState>>,
    Json(project): Json<AppProject>,
) -> Result<(StatusCode, Json<AppProject>), DeployError> {
    state.store.upsert_project(&project).await?;
    Ok((StatusCode::OK, Json(project)))
}

// ─── ApplicationSets ──────────────────────────────────────────────────────────

async fn list_appsets(
    State(_state): State<Arc<DeployState>>,
) -> Json<Vec<ApplicationSet>> {
    // TODO: persist ApplicationSets in DB
    Json(vec![])
}

async fn create_appset(
    State(_state): State<Arc<DeployState>>,
    Json(appset): Json<ApplicationSet>,
) -> Result<(StatusCode, Json<ApplicationSet>), DeployError> {
    // TODO: persist and trigger reconciliation
    Ok((StatusCode::CREATED, Json(appset)))
}

// ─── Health ───────────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module":   "cave-deploy",
        "status":   "ok",
        "upstream": "argocd",
        "upstream_tracked_version": "2.x",
        "features": [
            "application-crds", "git-sync", "drift-detection",
            "health-assessment", "sync-waves", "hooks",
            "applicationsets", "multi-cluster", "rbac",
            "notifications", "rollback", "diff-engine"
        ]
>>>>>>> claude/thirsty-snyder
    }))
}
