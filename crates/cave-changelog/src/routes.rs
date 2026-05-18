// SPDX-License-Identifier: AGPL-3.0-or-later
//! HTTP routes for cave-changelog.

use axum::{routing::get, Json, Router};

pub fn create_router() -> Router {
    Router::new()
        .route("/api/changelog/health", get(health))
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-changelog",
        "status": "ok",
        "upstream": "custom",
        "features": "Auto-generated changelogs from git commits + SBOM diffs per deployment"
    }))
}
