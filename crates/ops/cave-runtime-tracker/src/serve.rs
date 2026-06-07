// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Minimal metrics HTTP daemon.
//!
//! `serve` exposes the latest daily report as Prometheus metrics at
//! `/metrics` (plus a `/healthz` liveness probe). It is read-only: every
//! scrape loads the cached `latest.json` (+ `latest-measure.json`) from
//! the report output dir and renders it through [`render_prometheus`], so
//! the daemon never polls GitHub itself — the daily `report` agent owns
//! refresh, this one only serves.
//!
//! Default port is **9103** (9101/9102 belong to the cave autopilots).

use std::path::Path;
use std::sync::Arc;

use axum::{extract::State, http::header, response::IntoResponse, routing::get, Router};

use crate::config::TrackerConfig;
use crate::measure::Measurement;
use crate::metrics::render_prometheus;
use crate::poll::PollSummary;
use crate::report::DailyReport;

/// Load the latest cached report + measurements from `dir`. Falls back to
/// a registry-only report (every row `unknown`, no network) when no
/// `latest.json` exists yet, so `/metrics` always renders.
pub fn load_latest(cfg: &TrackerConfig, dir: &Path) -> (DailyReport, Vec<Measurement>) {
    let report = std::fs::read_to_string(dir.join("latest.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<DailyReport>(&t).ok())
        .unwrap_or_else(|| DailyReport::assemble(PollSummary::from_registry_only(cfg)));
    let measurements = std::fs::read_to_string(dir.join("latest-measure.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Vec<Measurement>>(&t).ok())
        .unwrap_or_default();
    (report, measurements)
}

#[derive(Clone)]
struct AppState {
    cfg: Arc<TrackerConfig>,
    dir: Arc<std::path::PathBuf>,
}

async fn metrics_handler(State(st): State<AppState>) -> impl IntoResponse {
    let (report, measurements) = load_latest(&st.cfg, &st.dir);
    let body = render_prometheus(&report, &measurements);
    (
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
}

async fn healthz_handler() -> &'static str {
    "ok\n"
}

/// Build the router (separated for testability via `tower`'s oneshot).
pub fn router(cfg: TrackerConfig, dir: std::path::PathBuf) -> Router {
    let state = AppState {
        cfg: Arc::new(cfg),
        dir: Arc::new(dir),
    };
    Router::new()
        .route("/metrics", get(metrics_handler))
        .route("/healthz", get(healthz_handler))
        .with_state(state)
}

/// Bind `0.0.0.0:port` and serve until the process is signalled.
pub async fn serve(cfg: TrackerConfig, port: u16) -> crate::error::TrackerResult<()> {
    let dir = std::path::PathBuf::from(cfg.expanded_output_dir());
    let app = router(cfg, dir);
    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| crate::error::TrackerError::Http(format!("bind {addr}: {e}")))?;
    tracing::info!("cave-runtime-tracker serving /metrics on {addr}");
    axum::serve(listener, app)
        .await
        .map_err(|e| crate::error::TrackerError::Http(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::DriftStatus;

    #[test]
    fn load_latest_falls_back_to_registry_only_report() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = TrackerConfig::default_config();
        let (report, measurements) = load_latest(&cfg, dir.path());
        assert_eq!(report.totals.tracked, cfg.upstreams.len());
        // No cached report → every row unknown, no measurements.
        assert_eq!(report.totals.unknown, cfg.upstreams.len());
        assert!(measurements.is_empty());
    }

    #[test]
    fn load_latest_reads_cached_report_and_measurements() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = TrackerConfig::default_config();
        // Write a cached report + a measurement file.
        let report = DailyReport::assemble(PollSummary::from_registry_only(&cfg));
        std::fs::write(
            dir.path().join("latest.json"),
            serde_json::to_string(&report).unwrap(),
        )
        .unwrap();
        let m = Measurement {
            upstream_repo: "cilium/cilium".to_string(),
            cave_module: "cave-net".to_string(),
            upstream: Some(crate::measure::LocStats { code: 1000, ..Default::default() }),
            cave: Some(crate::measure::LocStats { code: 100, ..Default::default() }),
            ratio: Some(0.1),
        };
        std::fs::write(
            dir.path().join("latest-measure.json"),
            serde_json::to_string(&vec![m]).unwrap(),
        )
        .unwrap();

        let (loaded, measurements) = load_latest(&cfg, dir.path());
        assert_eq!(loaded.totals.tracked, cfg.upstreams.len());
        assert_eq!(measurements.len(), 1);
        assert_eq!(measurements[0].cave_module, "cave-net");
    }

    #[tokio::test]
    async fn metrics_endpoint_serves_exposition() {
        use tower::ServiceExt;
        let dir = tempfile::tempdir().unwrap();
        let cfg = TrackerConfig::default_config();
        // Seed a behind row so the exposition has a non-trivial value.
        let mut summary = PollSummary::from_registry_only(&cfg);
        summary.results[0].latest = Some("v999".to_string());
        summary.results[0].upstream.pinned = Some("v1".to_string());
        summary.results[0].status = DriftStatus::Behind;
        let report = DailyReport::assemble(summary);
        std::fs::write(
            dir.path().join("latest.json"),
            serde_json::to_string(&report).unwrap(),
        )
        .unwrap();

        let app = router(cfg, dir.path().to_path_buf());
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/metrics")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(body.contains("cave_runtime_tracker_tracked"));
        assert!(body.contains("cave_runtime_tracker_drift{"));
    }

    #[tokio::test]
    async fn healthz_returns_ok() {
        use tower::ServiceExt;
        let dir = tempfile::tempdir().unwrap();
        let app = router(TrackerConfig::default_config(), dir.path().to_path_buf());
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/healthz")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
    }
}
