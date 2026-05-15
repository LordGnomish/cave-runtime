// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP router skeleton. Endpoint surface to be filled out against Twenty's
//! REST + GraphQL APIs in v0.2 — see parity.manifest.toml for tracked surfaces.

use crate::store::CrmStore;
use axum::{response::IntoResponse, routing::get, Json, Router};
use serde_json::json;
use std::sync::Arc;

async fn health() -> impl IntoResponse {
    Json(json!({
        "module": "cave-crm",
        "status": "ok",
        "upstream": "twentyhq/twenty",
        "upstream_version": "v2.2.0",
        "objects": ["person", "company", "opportunity", "activity"]
    }))
}

pub fn create_router(_state: Arc<CrmStore>) -> Router {
    Router::new().route("/api/crm/health", get(health))
}
