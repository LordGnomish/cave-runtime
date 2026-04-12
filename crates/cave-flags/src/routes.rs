//! HTTP routes for the flags module.

use crate::models::*;
use crate::FlagsState;
use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router(state: Arc<FlagsState>) -> Router {
    Router::new()
        .route("/api/flags", get(list_flags).post(create_flag))
        .route("/api/flags/evaluate", post(evaluate))
        .route("/api/flags/health", get(health))
        .with_state(state)
}

/// GET /api/flags — list all flags
async fn list_flags(State(_state): State<Arc<FlagsState>>) -> Json<Vec<FeatureFlag>> {
    // TODO: query from cave_flags schema
    Json(vec![])
}

/// POST /api/flags — create a new flag
async fn create_flag(
    State(_state): State<Arc<FlagsState>>,
    Json(req): Json<CreateFlagRequest>,
) -> Json<FeatureFlag> {
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
    // TODO: persist to cave_flags schema
    Json(flag)
}

/// POST /api/flags/evaluate — evaluate all flags for a context
async fn evaluate(
    State(_state): State<Arc<FlagsState>>,
    Json(req): Json<EvaluateRequest>,
) -> Json<EvaluateResponse> {
    // TODO: load flags from DB, evaluate
    let flags = vec![]; // placeholder
    let evaluations = crate::engine::evaluate_flags(&flags, &req.context);
    Json(EvaluateResponse { flags: evaluations })
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
