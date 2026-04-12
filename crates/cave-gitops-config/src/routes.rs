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
};
use axum::{
    extract::{Path, State as AxumState},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        // Promises
        .route(
            "/api/v1/platform/promises",
            post(create_promise).get(list_promises),
        )
        .route(
            "/api/v1/platform/promises/:name",
            get(get_promise).put(update_promise).delete(delete_promise),
        )
        // Developer self-service
        .route(
            "/api/v1/platform/requests",
            post(create_request).get(list_requests),
        )
        .route(
            "/api/v1/platform/requests/:id/status",
            get(get_request_status),
        )
        // Catalog
        .route("/api/v1/platform/catalog", get(get_catalog))
        // Compositions
        .route(
            "/api/v1/platform/compositions",
            post(create_composition).get(list_compositions),
        )
        .route(
            "/api/v1/platform/compositions/:id",
            get(get_composition).delete(delete_composition),
        )
        // Environments
        .route(
            "/api/v1/platform/environments",
            post(create_environment).get(list_environments),
        )
        .route(
            "/api/v1/platform/environments/:name",
            get(get_environment).delete(delete_environment),
        )
        // Claims
        .route("/api/v1/platform/claims", get(list_claims))
        // Health
        .route("/api/v1/platform/health", get(health))
        .with_state(state)
}

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
}

async fn list_promises(
    AxumState(state): AxumState<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let promises = promise::list_promises(state).await;
    Json(serde_json::json!({ "data": promises, "total": promises.len() }))
}

async fn get_promise(
    AxumState(state): AxumState<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let p = promise::get_promise(state, &name)
        .await
        .map_err(AppError::from)?;
    Ok(Json(serde_json::json!({ "data": p })))
}

async fn update_promise(
    AxumState(state): AxumState<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AppError> {
    let new_status: PromiseStatus = serde_json::from_value(
        body.get("status")
            .cloned()
            .unwrap_or(serde_json::json!("active")),
    )
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
}

async fn delete_promise(
    AxumState(state): AxumState<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<StatusCode, AppError> {
    let mut promises = state.promises.lock().await;
    let before = promises.len();
    promises.retain(|p| p.name != name);
    if promises.len() == before {
        return Err(AppError::NotFound(format!("Promise '{name}' not found")));
    }
    Ok(StatusCode::NO_CONTENT)
}

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
}

async fn list_requests(
    AxumState(state): AxumState<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let requests = state.requests.lock().await.clone();
    Json(serde_json::json!({ "data": requests, "total": requests.len() }))
}

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
}

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
}

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
    };

    state
        .compositions
        .lock()
        .await
        .push(composition.clone());

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "data": composition })),
    ))
}

async fn list_compositions(
    AxumState(state): AxumState<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let compositions = state.compositions.lock().await.clone();
    Json(serde_json::json!({ "data": compositions, "total": compositions.len() }))
}

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
}

async fn delete_composition(
    AxumState(state): AxumState<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let mut compositions = state.compositions.lock().await;
    let before = compositions.len();
    compositions.retain(|c| c.id != id);
    if compositions.len() == before {
        return Err(AppError::NotFound(format!("Composition '{id}' not found")));
    }
    Ok(StatusCode::NO_CONTENT)
}

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
    };

    let mut envs = state.environments.lock().await;
    if envs.iter().any(|e| e.name == env.name) {
        return Err(AppError::Conflict(format!(
            "Environment '{}' already exists",
            env.name
        )));
    }
    envs.push(env.clone());

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "data": env })),
    ))
}

async fn list_environments(
    AxumState(state): AxumState<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let envs = state.environments.lock().await.clone();
    Json(serde_json::json!({ "data": envs, "total": envs.len() }))
}

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
}

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
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Claims
// ---------------------------------------------------------------------------

async fn list_claims(
    AxumState(state): AxumState<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let claims = state.claims.lock().await.clone();
    Json(serde_json::json!({ "data": claims, "total": claims.len() }))
}

// ---------------------------------------------------------------------------
// Health
// ---------------------------------------------------------------------------

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-gitops-config",
        "status": "ok",
        "upstream": ["Kratix", "Crossplane"]
    }))
}

// ---------------------------------------------------------------------------
// Error handling
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum AppError {
    NotFound(String),
    Conflict(String),
    BadRequest(String),
    Engine(promise::EngineError),
}

impl From<promise::EngineError> for AppError {
    fn from(e: promise::EngineError) -> Self {
        match &e {
            promise::EngineError::NotFound(_) => AppError::NotFound(e.to_string()),
            promise::EngineError::AlreadyExists(_) => AppError::Conflict(e.to_string()),
            promise::EngineError::ValidationFailed(_)
            | promise::EngineError::ComplianceViolation(_)
            | promise::EngineError::Unavailable(_) => AppError::BadRequest(e.to_string()),
            _ => AppError::Engine(e),
        }
    }
}

impl axum::response::IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match &self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, msg.clone()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AppError::Engine(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        };
        (
            status,
            Json(serde_json::json!({ "error": message })),
        )
            .into_response()
    }
}
