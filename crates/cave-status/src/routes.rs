//! HTTP routes for cave-status.

use axum::{routing::get, Json, Router};

pub fn create_router() -> Router {
    Router::new()
        .route("/api/status/health", get(health))
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-status",
        "status": "ok",
        "upstream": "custom",
        "features": "Public/internal status page, auto-generation from probes, incident integration"
    }))
}
