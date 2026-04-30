//! Runtime build/migration progress endpoints.
//!
//! Surfaces a JSON snapshot of platform port progress so the dev portal can
//! render its "Runtime Progress" panel without each module wiring up a custom
//! status endpoint. The values come from the workspace-level upstream tracker.

use axum::{routing::get, Json, Router};
use serde::Serialize;

#[derive(Serialize)]
struct ProgressSummary {
    tracked_projects: usize,
    schema_version: u32,
}

async fn progress_summary() -> Json<ProgressSummary> {
    Json(ProgressSummary {
        tracked_projects: cave_upstream::TRACKED_PROJECTS.len(),
        schema_version: 1,
    })
}

/// Build the `/api/portal/progress` router.
pub fn router() -> Router {
    Router::new().route("/api/portal/progress", get(progress_summary))
}
