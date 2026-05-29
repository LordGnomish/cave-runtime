// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP routes for cave-gitops-config.

use crate::engine::PipelineEngine;
use crate::models::{
    ClusterDestination, ClusterStatus, CreatePromiseRequest, CreateResourceRequestRequest, Promise,
    PromiseStatus, RegisterClusterRequest, ResourceRequest, ResourceRequestStatus, StateStoreEntry,
    compare_state,
};
use crate::store::GitOpsStore;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

pub struct GitOpsAppState {
    pub store: GitOpsStore,
}

impl Default for GitOpsAppState {
    fn default() -> Self {
        Self {
            store: GitOpsStore::new(),
        }
    }
}

pub fn create_router(state: Arc<GitOpsAppState>) -> Router {
    Router::new()
        .route("/api/gitops/health", get(health))
        .route(
            "/api/gitops/promises",
            get(list_promises).post(create_promise),
        )
        .route(
            "/api/gitops/promises/{name}",
            get(get_promise).put(update_promise).delete(delete_promise),
        )
        .route(
            "/api/gitops/requests",
            get(list_requests).post(create_request),
        )
        .route(
            "/api/gitops/requests/{id}",
            get(get_request).delete(delete_request),
        )
        .route("/api/gitops/state", get(list_state))
        .route("/api/gitops/state/{*path}", get(get_state_entry))
        .route(
            "/api/gitops/clusters",
            get(list_clusters).post(register_cluster),
        )
        .route("/api/gitops/pipelines/{request_id}", get(get_pipeline))
        .with_state(state)
}

// ─── Health ───────────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-gitops-config",
        "status": "ok",
        "upstream": "kratix"
    }))
}

// ─── Promises ─────────────────────────────────────────────────────────────────

async fn list_promises(State(state): State<Arc<GitOpsAppState>>) -> Json<Vec<Promise>> {
    Json(state.store.list_promises())
}

async fn get_promise(
    State(state): State<Arc<GitOpsAppState>>,
    Path(name): Path<String>,
) -> Result<Json<Promise>, StatusCode> {
    state
        .store
        .get_promise(&name)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn create_promise(
    State(state): State<Arc<GitOpsAppState>>,
    Json(req): Json<CreatePromiseRequest>,
) -> (StatusCode, Json<Promise>) {
    let now = Utc::now();
    let promise = Promise {
        id: Uuid::new_v4(),
        name: req.name,
        version: req.version,
        description: req.description,
        api_schema: req.api_schema,
        pipeline: req.pipeline,
        dependencies: req.dependencies.unwrap_or_default(),
        destination_selectors: req.destination_selectors.unwrap_or_default(),
        status: PromiseStatus::Active,
        created_at: now,
        updated_at: now,
    };
    let created = state.store.create_promise(promise);
    (StatusCode::CREATED, Json(created))
}

async fn update_promise(
    State(state): State<Arc<GitOpsAppState>>,
    Path(name): Path<String>,
    Json(req): Json<CreatePromiseRequest>,
) -> Result<Json<Promise>, StatusCode> {
    let existing = state
        .store
        .get_promise(&name)
        .ok_or(StatusCode::NOT_FOUND)?;
    let updated = Promise {
        name: req.name,
        version: req.version,
        description: req.description,
        api_schema: req.api_schema,
        pipeline: req.pipeline,
        dependencies: req.dependencies.unwrap_or_default(),
        destination_selectors: req.destination_selectors.unwrap_or_default(),
        updated_at: Utc::now(),
        ..existing
    };
    state
        .store
        .update_promise(&name, updated)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn delete_promise(
    State(state): State<Arc<GitOpsAppState>>,
    Path(name): Path<String>,
) -> StatusCode {
    if state.store.delete_promise(&name) {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

// ─── Resource Requests ────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct RequestQuery {
    promise_name: Option<String>,
}

async fn list_requests(
    State(state): State<Arc<GitOpsAppState>>,
    Query(query): Query<RequestQuery>,
) -> Json<Vec<ResourceRequest>> {
    Json(
        state
            .store
            .list_resource_requests(query.promise_name.as_deref()),
    )
}

async fn get_request(
    State(state): State<Arc<GitOpsAppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ResourceRequest>, StatusCode> {
    state
        .store
        .get_resource_request(id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn create_request(
    State(state): State<Arc<GitOpsAppState>>,
    Json(req): Json<CreateResourceRequestRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    // Look up the promise
    let promise = state.store.get_promise(&req.promise_name).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "promise not found"})),
        )
    })?;

    // Validate spec against schema
    if let Err(errors) = PipelineEngine::validate_spec(&promise, &req.spec) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"errors": errors})),
        ));
    }

    let now = Utc::now();
    let resource_request = ResourceRequest {
        id: Uuid::new_v4(),
        promise_name: req.promise_name.clone(),
        promise_version: req.promise_version,
        namespace: req.namespace,
        name: req.name,
        spec: req.spec,
        requester: req.requester,
        status: ResourceRequestStatus::InPipeline,
        pipeline_run: None,
        destinations: vec![],
        created_at: now,
        updated_at: now,
    };

    let stored = state
        .store
        .create_resource_request(resource_request);

    // Run the pipeline
    let clusters = state.store.list_clusters();
    let destinations = PipelineEngine::select_destinations(&promise, &clusters);
    let pipeline_run = PipelineEngine::run_pipeline(&promise, &stored);
    let run_status = pipeline_run.status.clone();
    let run_id = pipeline_run.id;
    state.store.add_pipeline_run(pipeline_run.clone());

    let new_status = match run_status {
        crate::models::PipelineRunStatus::Completed => ResourceRequestStatus::Ready,
        crate::models::PipelineRunStatus::Failed => ResourceRequestStatus::Failed,
        crate::models::PipelineRunStatus::Running => ResourceRequestStatus::InPipeline,
    };

    // Write state store entries for each destination
    for cluster in &destinations {
        let path = PipelineEngine::state_store_path(
            cluster,
            &stored.promise_name,
            &stored.namespace,
            &stored.name,
        );
        let desired = format!(
            "apiVersion: cave.dev/v1\nkind: {}\nmetadata:\n  name: {}\n  namespace: {}",
            stored.promise_name, stored.name, stored.namespace
        );
        // Reconcile against any existing live entry at this path. A freshly
        // written desired manifest with no live counterpart, or one that drifts
        // from the live state, is OutOfSync (ArgoCD CompareAppState semantics).
        let live = state
            .store
            .get_state_entry(&path)
            .map(|e| e.content);
        let sync_status = compare_state(&desired, live.as_deref());
        state.store.upsert_state_entry(StateStoreEntry {
            id: Uuid::new_v4(),
            path,
            cluster: cluster.clone(),
            content: desired,
            checksum: format!("{:x}", stored.id.as_u128()),
            promise_name: stored.promise_name.clone(),
            resource_request_id: stored.id,
            last_synced: Some(Utc::now()),
            sync_status,
        });
    }

    state.store.update_resource_request_status(
        stored.id,
        new_status,
        Some(pipeline_run),
        Some(destinations),
    );

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "resource_request_id": stored.id,
            "pipeline_run_id": run_id,
        })),
    ))
}

async fn delete_request(
    State(state): State<Arc<GitOpsAppState>>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    if state.store.delete_resource_request(id) {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

// ─── State Store ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct StateQuery {
    cluster: Option<String>,
}

async fn list_state(
    State(state): State<Arc<GitOpsAppState>>,
    Query(query): Query<StateQuery>,
) -> Json<Vec<StateStoreEntry>> {
    Json(state.store.list_state_entries(query.cluster.as_deref()))
}

async fn get_state_entry(
    State(state): State<Arc<GitOpsAppState>>,
    Path(path): Path<String>,
) -> Result<Json<StateStoreEntry>, StatusCode> {
    state
        .store
        .get_state_entry(&path)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

// ─── Clusters ─────────────────────────────────────────────────────────────────

async fn list_clusters(State(state): State<Arc<GitOpsAppState>>) -> Json<Vec<ClusterDestination>> {
    Json(state.store.list_clusters())
}

async fn register_cluster(
    State(state): State<Arc<GitOpsAppState>>,
    Json(req): Json<RegisterClusterRequest>,
) -> (StatusCode, Json<ClusterDestination>) {
    let cluster = ClusterDestination {
        name: req.name,
        api_server: req.api_server,
        labels: req.labels.unwrap_or_default(),
        status: ClusterStatus::Unknown,
        registered_at: Utc::now(),
    };
    let registered = state.store.register_cluster(cluster);
    (StatusCode::CREATED, Json(registered))
}

// ─── Pipeline Runs ────────────────────────────────────────────────────────────

async fn get_pipeline(
    State(state): State<Arc<GitOpsAppState>>,
    Path(request_id): Path<Uuid>,
) -> Result<Json<crate::models::PipelineRun>, StatusCode> {
    state
        .store
        .get_pipeline_run(request_id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}
