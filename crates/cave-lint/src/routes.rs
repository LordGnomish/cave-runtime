// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP routes for cave-lint.

use crate::LintState;
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use serde::Deserialize;
use std::sync::Arc;

pub fn create_router(state: Arc<LintState>) -> Router {
    Router::new()
        .route("/api/lint/check", post(check))
        .route("/api/lint/rules", get(list_rules))
        .route("/api/lint/health", get(health))
        .with_state(state)
}

#[derive(Deserialize)]
pub struct CheckRequest {
    pub content: String,
    pub filename: String,
}

async fn check(
    State(state): State<Arc<LintState>>,
    Json(req): Json<CheckRequest>,
) -> Json<Vec<crate::rules::Violation>> {
    Json(crate::rules::lint(&req.content, &state.rules))
}

async fn list_rules(State(state): State<Arc<LintState>>) -> Json<Vec<String>> {
    Json(
        state
            .rules
            .iter()
            .map(|r| format!("{}: {}", r.id, r.description))
            .collect(),
    )
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-lint",
        "status": "ok",
        "upstream": "hadolint + checkov + pluto"
    }))
}
