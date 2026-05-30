// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP routes for cave-workflows (Argo Workflows parity).

use crate::cron::CronSchedule;
use crate::State;
use axum::{extract::Query, routing::get, Json, Router};
use serde::Deserialize;
use std::sync::Arc;

pub fn create_router(state: Arc<State>) -> Router {
    Router::new()
        .route("/api/workflows/health", get(health))
        .route("/api/workflows/cron/next", get(cron_next))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-workflows",
        "status": "ok",
        "upstream": "argoproj/argo-workflows v4.0.5"
    }))
}

#[derive(Deserialize)]
struct CronNextQuery {
    schedule: String,
    /// RFC3339 instant to compute the next fire after; defaults to now.
    #[serde(default)]
    after: Option<chrono::DateTime<chrono::Utc>>,
}

/// Validate a cron schedule and return the next fire time after `after`.
async fn cron_next(Query(q): Query<CronNextQuery>) -> Json<serde_json::Value> {
    match CronSchedule::parse(&q.schedule) {
        Ok(sched) => {
            let after = q.after.unwrap_or_else(chrono::Utc::now);
            Json(serde_json::json!({
                "valid": true,
                "schedule": q.schedule,
                "next": sched.next(after),
            }))
        }
        Err(e) => Json(serde_json::json!({
            "valid": false,
            "schedule": q.schedule,
            "error": e.to_string(),
        })),
    }
}
