// SPDX-License-Identifier: AGPL-3.0-or-later
//! /api/v1/targets and /api/v1/targets/metadata

use axum::{extract::State, Json};
use std::sync::Arc;
use crate::state::MetricsState;

pub async fn list_targets(State(state): State<Arc<MetricsState>>) -> Json<serde_json::Value> {
    let targets = state.scrape_manager.targets();

    let active: Vec<serde_json::Value> = targets.iter().map(|t| {
        serde_json::json!({
            "discoveredLabels": t.labels.0,
            "labels": t.labels.0,
            "scrapePool": t.config.job_name,
            "scrapeUrl": t.url,
            "globalUrl": t.url,
            "lastError": t.last_error.as_deref().unwrap_or(""),
            "lastScrape": chrono::DateTime::<chrono::Utc>::from_timestamp_millis(t.last_scrape_ms)
                .map(|d| d.to_rfc3339()).unwrap_or_default(),
            "lastScrapeDuration": t.last_duration_ms as f64 / 1000.0,
            "health": t.health(),
            "scrapeInterval": format!("{}s", t.config.scrape_interval_ms / 1000),
            "scrapeTimeout": format!("{}s", t.config.scrape_timeout_ms / 1000),
        })
    }).collect();

    Json(serde_json::json!({
        "status": "success",
        "data": {
            "activeTargets": active,
            "droppedTargets": [],
            "droppedTargetCounts": {},
        }
    }))
}

pub async fn targets_metadata(State(_state): State<Arc<MetricsState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "success", "data": [] }))
}
