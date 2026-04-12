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
    routing::{delete, get, post, put},
    Json, Router,
};
use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router(state: Arc<DeployState>) -> Router {
    Router::new()
        // Applications — CRUD
        .route("/api/v1/deploy/applications", get(list_applications))
        .route("/api/v1/deploy/applications", post(create_application))
        .route("/api/v1/deploy/applications/{id}", get(get_application))
        .route("/api/v1/deploy/applications/{id}", put(update_application))
        .route("/api/v1/deploy/applications/{id}", delete(delete_application))
        // Sync / Rollback
        .route("/api/v1/deploy/applications/{id}/sync", post(sync_app))
        .route(
            "/api/v1/deploy/applications/{id}/rollback",
            post(rollback_app),
        )
        // Status / History
        .route(
            "/api/v1/deploy/applications/{id}/status",
            get(get_app_status),
        )
        .route(
            "/api/v1/deploy/applications/{id}/history",
            get(get_app_history),
        )
        // Diff
        .route("/api/v1/deploy/diff/{id}", get(get_diff))
        // Rollouts
        .route("/api/v1/deploy/rollouts", post(create_rollout))
        .route("/api/v1/deploy/rollouts/{id}", get(get_rollout))
        .route("/api/v1/deploy/rollouts/{id}/promote", post(promote_rollout))
        .route("/api/v1/deploy/rollouts/{id}/abort", post(abort_rollout))
        // Module health
        .route("/api/v1/deploy/health", get(health))
        .with_state(state)
}

// ─── Applications ─────────────────────────────────────────────────────────────

async fn list_applications(
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
    }))
}
