//! HTTP routes for the flags module.

use crate::models::*;
use crate::FlagsState;
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use cave_db::StorageExt;
use std::sync::Arc;
use uuid::Uuid;

const COLLECTION: &str = "flags";

pub fn create_router(state: Arc<FlagsState>) -> Router {
    Router::new()
        .route("/api/flags", get(list_flags).post(create_flag))
        .route("/api/flags/evaluate", post(evaluate))
        .route("/api/flags/health", get(health))
        .with_state(state)
}

/// GET /api/flags — list all flags
async fn list_flags(
    State(state): State<Arc<FlagsState>>,
) -> Result<Json<Vec<FeatureFlag>>, StatusCode> {
    let flags = state
        .storage
        .list::<FeatureFlag>(COLLECTION)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(flags))
}

/// POST /api/flags — create a new flag
async fn create_flag(
    State(state): State<Arc<FlagsState>>,
    Json(req): Json<CreateFlagRequest>,
) -> Result<Json<FeatureFlag>, StatusCode> {
    let now = chrono::Utc::now();
    let flag = FeatureFlag {
        id: Uuid::new_v4(),
        name: req.name,
        description: req.description,
        enabled: true,
        flag_type: req.flag_type,
        strategy: req.strategy,
        environments: req.environments,
        tenant_id: req.tenant_id,
        kill_switch: false,
        created_at: now,
        updated_at: now,
        created_by: Uuid::new_v4(), // TODO: extract from CaveIdentity
    };

    state
        .storage
        .put::<FeatureFlag>(COLLECTION, &flag.id.to_string(), &flag)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(flag))
}

/// POST /api/flags/evaluate — evaluate all flags for a context
async fn evaluate(
    State(state): State<Arc<FlagsState>>,
    Json(req): Json<EvaluateRequest>,
) -> Result<Json<EvaluateResponse>, StatusCode> {
    let flags = state
        .storage
        .list::<FeatureFlag>(COLLECTION)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let evaluations = crate::engine::evaluate_flags(&flags, &req.context);
    Ok(Json(EvaluateResponse { flags: evaluations }))
}

/// GET /api/flags/health — module health check
async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-flags",
        "status": "ok",
        "upstream": "unleash",
        "upstream_tracked_version": "6.x"
    }))
}

// TODO: SSE endpoint for real-time flag updates
// GET /api/flags/stream — Server-Sent Events
