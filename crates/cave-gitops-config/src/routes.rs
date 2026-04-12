//! HTTP routes for cave-gitops-config.

use crate::engine::PipelineEngine;
use crate::models::{
    ClusterDestination, ClusterStatus, CreatePromiseRequest, CreateResourceRequestRequest,
    Promise, PromiseStatus, RegisterClusterRequest, ResourceRequest, ResourceRequestStatus,
    StateStoreEntry, SyncStatus,
};
use crate::store::GitOpsStore;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
    Json, Router,
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
        .route("/api/gitops/promises", get(list_promises).post(create_promise))
        .route(
            "/api/gitops/promises/:name",
            get(get_promise).put(update_promise).delete(delete_promise),
        )
        .route("/api/gitops/requests", get(list_requests).post(create_request))
        .route(
            "/api/gitops/requests/:id",
            get(get_request).delete(delete_request),
        )
        .route("/api/gitops/state", get(list_state))
        .route("/api/gitops/state/*path", get(get_state_entry))
        .route("/api/gitops/clusters", get(list_clusters).post(register_cluster))
        .route("/api/gitops/pipelines/:request_id", get(get_pipeline))
        .with_state(state)
}

// ─── Health ───────────────────────────────────────────────────────────────────
//! HTTP routes for the CAVE GitOps Config / Platform API.
//!
//! Endpoint map:
//!
//! Promises (platform team)
//!   POST   /api/v1/platform/promises
//!   GET    /api/v1/platform/promises
//!   GET    /api/v1/platform/promises/:name
//!   PUT    /api/v1/platform/promises/:name      (deprecate / reactivate)
//!   DELETE /api/v1/platform/promises/:name
//!
//! Developer self-service
//!   POST   /api/v1/platform/requests            — request a capability
//!   GET    /api/v1/platform/requests            — list all requests
//!   GET    /api/v1/platform/requests/:id/status — track provisioning
//!
//!   GET    /api/v1/platform/catalog             — self-service catalog
//!
//! Compositions
//!   POST   /api/v1/platform/compositions
//!   GET    /api/v1/platform/compositions
//!   GET    /api/v1/platform/compositions/:id
//!   DELETE /api/v1/platform/compositions/:id
//!
//! Environments
//!   POST   /api/v1/platform/environments
//!   GET    /api/v1/platform/environments
//!   GET    /api/v1/platform/environments/:name
//!   DELETE /api/v1/platform/environments/:name
//!
//! Resource claims
//!   GET    /api/v1/platform/claims              — all provisioned resources
use crate::{
    models::{
        Composition, CreateCapabilityRequest, CreateCompositionRequest, CreateEnvironmentRequest,
        CreatePromiseRequest, Environment, PromiseStatus,
    },
    promise,
    AppState,
    extract::{Path, State as AxumState},
    routing::{get, post},
pub fn create_router(state: Arc<AppState>) -> Router {
        // Promises
            "/api/v1/platform/promises",
            post(create_promise).get(list_promises),
            "/api/v1/platform/promises/:name",
        // Developer self-service
            "/api/v1/platform/requests",
            post(create_request).get(list_requests),
            "/api/v1/platform/requests/:id/status",
            get(get_request_status),
        // Catalog
        .route("/api/v1/platform/catalog", get(get_catalog))
        // Compositions
            "/api/v1/platform/compositions",
            post(create_composition).get(list_compositions),
            "/api/v1/platform/compositions/:id",
            get(get_composition).delete(delete_composition),
        // Environments
            "/api/v1/platform/environments",
            post(create_environment).get(list_environments),
            "/api/v1/platform/environments/:name",
            get(get_environment).delete(delete_environment),
        // Claims
        .route("/api/v1/platform/claims", get(list_claims))
        // Health
        .route("/api/v1/platform/health", get(health))
// ---------------------------------------------------------------------------
// Promises
// ---------------------------------------------------------------------------
async fn create_promise(
    AxumState(state): AxumState<Arc<AppState>>,
    Json(body): Json<CreatePromiseRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    let promise = promise::register_promise(state, body)
        .await
        .map_err(AppError::from)?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "data": promise })),
    ))
async fn list_promises(
    AxumState(state): AxumState<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let promises = promise::list_promises(state).await;
    Json(serde_json::json!({ "data": promises, "total": promises.len() }))
async fn get_promise(
    AxumState(state): AxumState<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let p = promise::get_promise(state, &name)
        .await
        .map_err(AppError::from)?;
    Ok(Json(serde_json::json!({ "data": p })))
async fn update_promise(
    AxumState(state): AxumState<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AppError> {
    let new_status: PromiseStatus = serde_json::from_value(
        body.get("status")
            .cloned()
            .unwrap_or(serde_json::json!("active")),
    .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let mut promises = state.promises.lock().await;
    let p = promises
        .iter_mut()
        .find(|p| p.name == name)
        .ok_or_else(|| AppError::NotFound(format!("Promise '{name}' not found")))?;
    p.status = new_status;
    p.updated_at = Utc::now();
    let updated = p.clone();
    Ok(Json(serde_json::json!({ "data": updated })))
async fn delete_promise(
    AxumState(state): AxumState<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<StatusCode, AppError> {
    let mut promises = state.promises.lock().await;
    let before = promises.len();
    promises.retain(|p| p.name != name);
    if promises.len() == before {
        return Err(AppError::NotFound(format!("Promise '{name}' not found")));
    Ok(StatusCode::NO_CONTENT)
// ---------------------------------------------------------------------------
// Developer requests
// ---------------------------------------------------------------------------
async fn create_request(
    AxumState(state): AxumState<Arc<AppState>>,
    Json(body): Json<CreateCapabilityRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    let req = promise::fulfill_request(state, body)
        .await
        .map_err(AppError::from)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "data": req })),
    ))
async fn list_requests(
    AxumState(state): AxumState<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let requests = state.requests.lock().await.clone();
    Json(serde_json::json!({ "data": requests, "total": requests.len() }))
async fn get_request_status(
    AxumState(state): AxumState<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let requests = state.requests.lock().await;
    let req = requests
        .iter()
        .find(|r| r.id == id)
        .ok_or_else(|| AppError::NotFound(format!("Request '{id}' not found")))?;
    Ok(Json(serde_json::json!({
        "id": req.id,
        "status": req.status,
        "message": req.message,
        "claim_ids": req.claim_ids,
        "updated_at": req.updated_at,
    })))
// ---------------------------------------------------------------------------
// Self-service catalog
// ---------------------------------------------------------------------------
async fn get_catalog(
    AxumState(state): AxumState<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let promises = state.promises.lock().await;
    let catalog: Vec<serde_json::Value> = promises
        .iter()
        .filter(|p| p.status == PromiseStatus::Active)
        .map(|p| {
            serde_json::json!({
                "name": p.name,
                "description": p.description,
                "version": p.version,
                "api_group": p.api_group,
                "input_schema": p.input_schema,
            })
        })
        .collect();
    Json(serde_json::json!({
        "catalog": catalog,
        "total": catalog.len(),
    }))
// ---------------------------------------------------------------------------
// Compositions
// ---------------------------------------------------------------------------
async fn create_composition(
    AxumState(state): AxumState<Arc<AppState>>,
    Json(body): Json<CreateCompositionRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    let composition = Composition {
        id: Uuid::new_v4(),
        name: body.name,
        description: body.description,
        promise_id: body.promise_id,
        steps: body.steps,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    state
        .compositions
        .lock()
        .await
        .push(composition.clone());
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "data": composition })),
    ))
async fn list_compositions(
    AxumState(state): AxumState<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let compositions = state.compositions.lock().await.clone();
    Json(serde_json::json!({ "data": compositions, "total": compositions.len() }))
async fn get_composition(
    AxumState(state): AxumState<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let compositions = state.compositions.lock().await;
    let c = compositions
        .iter()
        .find(|c| c.id == id)
        .ok_or_else(|| AppError::NotFound(format!("Composition '{id}' not found")))?;
    Ok(Json(serde_json::json!({ "data": c })))
async fn delete_composition(
    AxumState(state): AxumState<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let mut compositions = state.compositions.lock().await;
    let before = compositions.len();
    compositions.retain(|c| c.id != id);
    if compositions.len() == before {
        return Err(AppError::NotFound(format!("Composition '{id}' not found")));
    Ok(StatusCode::NO_CONTENT)
// ---------------------------------------------------------------------------
// Environments
// ---------------------------------------------------------------------------
async fn create_environment(
    AxumState(state): AxumState<Arc<AppState>>,
    Json(body): Json<CreateEnvironmentRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    let env = Environment {
        name: body.name.clone(),
        description: body.description,
        tier: body.tier,
        constraints: body.constraints,
        defaults: body.defaults,
    let mut envs = state.environments.lock().await;
    if envs.iter().any(|e| e.name == env.name) {
        return Err(AppError::Conflict(format!(
            "Environment '{}' already exists",
            env.name
        )));
    envs.push(env.clone());
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "data": env })),
    ))
async fn list_environments(
    AxumState(state): AxumState<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let envs = state.environments.lock().await.clone();
    Json(serde_json::json!({ "data": envs, "total": envs.len() }))
async fn get_environment(
    AxumState(state): AxumState<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let envs = state.environments.lock().await;
    let e = envs
        .iter()
        .find(|e| e.name == name)
        .ok_or_else(|| AppError::NotFound(format!("Environment '{name}' not found")))?;
    Ok(Json(serde_json::json!({ "data": e })))
async fn delete_environment(
    AxumState(state): AxumState<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<StatusCode, AppError> {
    let mut envs = state.environments.lock().await;
    let before = envs.len();
    envs.retain(|e| e.name != name);
    if envs.len() == before {
        return Err(AppError::NotFound(format!(
            "Environment '{name}' not found"
        )));
    Ok(StatusCode::NO_CONTENT)
// ---------------------------------------------------------------------------
// Claims
// ---------------------------------------------------------------------------
async fn list_claims(
    AxumState(state): AxumState<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let claims = state.claims.lock().await.clone();
    Json(serde_json::json!({ "data": claims, "total": claims.len() }))
// ---------------------------------------------------------------------------
// Health
// ---------------------------------------------------------------------------

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
    let existing = state.store.get_promise(&name).ok_or(StatusCode::NOT_FOUND)?;
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
    let promise = state
        .store
        .get_promise(&req.promise_name)
        .ok_or_else(|| {
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

    let stored = state.store.create_resource_request(resource_request.clone());

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
        state.store.upsert_state_entry(StateStoreEntry {
            id: Uuid::new_v4(),
            path,
            cluster: cluster.clone(),
            content: format!(
                "apiVersion: cave.dev/v1\nkind: {}\nmetadata:\n  name: {}\n  namespace: {}",
                stored.promise_name, stored.name, stored.namespace
            ),
            checksum: format!("{:x}", stored.id.as_u128()),
            promise_name: stored.promise_name.clone(),
            resource_request_id: stored.id,
            last_synced: Some(Utc::now()),
            sync_status: SyncStatus::Synced,
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

async fn list_clusters(
    State(state): State<Arc<GitOpsAppState>>,
) -> Json<Vec<ClusterDestination>> {
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
        "upstream": ["Kratix", "Crossplane"]
// ---------------------------------------------------------------------------
// Error handling
// ---------------------------------------------------------------------------
#[derive(Debug)]
enum AppError {
    NotFound(String),
    Conflict(String),
    BadRequest(String),
    Engine(promise::EngineError),
impl From<promise::EngineError> for AppError {
    fn from(e: promise::EngineError) -> Self {
        match &e {
            promise::EngineError::NotFound(_) => AppError::NotFound(e.to_string()),
            promise::EngineError::AlreadyExists(_) => AppError::Conflict(e.to_string()),
            promise::EngineError::ValidationFailed(_)
            | promise::EngineError::ComplianceViolation(_)
            | promise::EngineError::Unavailable(_) => AppError::BadRequest(e.to_string()),
            _ => AppError::Engine(e),
impl axum::response::IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match &self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, msg.clone()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AppError::Engine(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            status,
            Json(serde_json::json!({ "error": message })),
            .into_response()
