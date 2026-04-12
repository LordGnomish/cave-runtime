//! Route assembly for cave-registry.

pub mod v2;

use crate::AppState;
use axum::{routing::get, Router};
use std::sync::Arc;

pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        // Health check (non-V2 path kept for backwards compat).
        .route("/api/registry/health", get(health))
        // Docker Registry V2 base check.
        .route("/v2/", get(v2::v2_check))
        // All other V2 paths dispatched through a single wildcard handler.
        .route("/v2/{*path}", axum::routing::any(v2::v2_dispatch))
        .with_state(state)
}

async fn health() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "module": "cave-registry",
        "status": "ok",
        "upstream": "Harbor"
    }))
}
