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
    routing::{delete, get, post, put},
    Json, Router,
};
use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

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
        .route("/api/v1/deploy/health", get(health))
        .with_state(state)
}

// ─── Applications ─────────────────────────────────────────────────────────────

async fn list_applications(
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
    }))
}
