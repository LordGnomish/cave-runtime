// SPDX-License-Identifier: AGPL-3.0-or-later
//! HTTP routes for cave-docs.

use axum::{routing::get, Json, Router};

pub fn create_router() -> Router {
    Router::new()
        .route("/api/docs/health", get(health))
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-docs",
        "status": "ok",
        "upstream": "apicurio + openapi-diff",
        "features": "OpenAPI/AsyncAPI spec storage, breaking change detection, schema versioning"
    }))
}
