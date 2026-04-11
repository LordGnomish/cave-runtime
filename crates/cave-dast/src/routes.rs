//! HTTP routes for cave-dast.

use crate::State;
use axum::{
    extract::State as AxumState,
    routing::get,
    Json, Router,
};
use std::sync::Arc;

pub fn create_router(state: Arc<State>) -> Router {
    Router::new()
        .route("/api/dast/health", get(health))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-dast",
        "status": "ok",
        "upstream": "OWASP ZAP"
    }))
}
