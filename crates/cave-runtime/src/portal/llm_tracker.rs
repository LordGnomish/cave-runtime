// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! LLM-tracker portal page.
//!
//! Renders the daily local-LLM tracker dashboard: the current baseline,
//! the per-candidate verdicts from the latest `daily-<date>.json`, and a
//! Phase-0 banner reminding the operator that nothing is ever swapped
//! automatically. The page reads the report the `cave-llm-tracker` binary
//! writes; the portal layer owns no bench/network logic.
//!
//! Routes
//! ──────
//!   GET /portal/llm-tracker                  → dashboard page (HTML)
//!   GET /api/portal/llm-tracker/latest       → latest report JSON (or
//!                                               `{ "available": false }`)
//!   GET /api/portal/llm-tracker/matrix       → cost × quality matrix JSON
//!
//! The report directory honours `CAVE_LLM_TRACKER_DIR`, falling back to
//! the macOS Application-Support path the LaunchAgent writes to.

use axum::{response::Html, routing::get, Json, Router};
use cave_llm_tracker::report::DailyReport;
use serde_json::json;
use std::path::PathBuf;

pub fn router() -> Router {
    Router::new()
        .route("/portal/llm-tracker", get(dashboard_page))
        .route("/api/portal/llm-tracker/latest", get(api_latest))
        .route("/api/portal/llm-tracker/matrix", get(api_matrix))
}

/// Directory the daily reports land in. `CAVE_LLM_TRACKER_DIR` overrides;
/// otherwise the Application-Support path the config defaults to.
fn report_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("CAVE_LLM_TRACKER_DIR") {
        return PathBuf::from(dir);
    }
    let cfg = cave_llm_tracker::default_config();
    PathBuf::from(cfg.expanded_output_dir())
}

/// Read `latest.json` (then the newest `daily-<date>.json`) from the
/// report dir, if any.
fn load_latest() -> Option<DailyReport> {
    let dir = report_dir();
    let latest = dir.join("latest.json");
    let path = if latest.exists() {
        latest
    } else {
        let mut dated: Vec<PathBuf> = std::fs::read_dir(&dir)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("daily-") && n.ends_with(".json"))
                    .unwrap_or(false)
            })
            .collect();
        dated.sort();
        dated.pop()?
    };
    let text = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&text).ok()
}

async fn dashboard_page() -> Html<String> {
    Html(render_dashboard(load_latest().as_ref()))
}

async fn api_latest() -> Json<serde_json::Value> {
    match load_latest() {
        Some(r) => Json(serde_json::to_value(&r).unwrap_or_else(|_| json!({ "available": false }))),
        None => Json(json!({ "available": false })),
    }
}

async fn api_matrix() -> Json<serde_json::Value> {
    match load_latest() {
        Some(r) => {
            let cfg = cave_llm_tracker::default_config();
            let m = cave_llm_tracker::build_matrix(&r, &cfg);
            Json(serde_json::to_value(&m).unwrap_or_else(|_| json!({ "available": false })))
        }
        None => Json(json!({ "available": false })),
    }
}

/// Pure HTML renderer for the dashboard. `None` → an empty-state page.
pub fn render_dashboard(_report: Option<&DailyReport>) -> String {
    // RED stub — filled in the GREEN commit.
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cave_llm_tracker::bench::synth_snapshot;
    use cave_llm_tracker::config::TrackerConfig;
    use cave_llm_tracker::poll::PollSummary;
    use cave_llm_tracker::selection::baseline_verdict;

    fn sample_report() -> DailyReport {
        let cfg = TrackerConfig::default_config();
        DailyReport::assemble(
            &cfg,
            PollSummary::from_seed_only(),
            synth_snapshot(&cfg.baseline.model),
            vec![],
            vec![baseline_verdict(&cfg.baseline.model)],
        )
    }

    #[test]
    fn empty_state_names_the_page_and_no_report() {
        let html = render_dashboard(None);
        assert!(html.contains("LLM Tracker"), "title missing");
        assert!(
            html.to_lowercase().contains("no report"),
            "empty-state hint missing"
        );
    }

    #[test]
    fn populated_page_shows_baseline_and_phase_0_banner() {
        let html = render_dashboard(Some(&sample_report()));
        assert!(html.contains("qwen3.6:35b-a3b-coding-mxfp8"), "baseline missing");
        assert!(html.contains("Phase 0"), "phase-0 banner missing");
        assert!(html.to_lowercase().contains("verdict"), "verdict table missing");
    }
}
