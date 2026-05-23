// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! HTTP routes for cave-trivy server mode.
//!
//! Mirrors trivy's `pkg/commands/server` + `pkg/rpc/server` JSON-over-HTTP
//! API distilled to four endpoints:
//!   POST /trivy/scan           — submit a `ScanRequest`, get `ScanResponse`
//!   GET  /trivy/reports        — list cached report IDs
//!   GET  /trivy/reports/:id    — fetch a stored report
//!   GET  /trivy/healthz        — liveness

use crate::server::{handle, ScanRequest, ScanResponse};
use crate::State;
use axum::{
    extract::{Path, State as AxState},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;

pub fn create_router(state: Arc<State>) -> Router {
    Router::new()
        .route("/trivy/scan", post(scan))
        .route("/trivy/reports", get(list_reports))
        .route("/trivy/reports/{id}", get(get_report))
        .route("/trivy/healthz", get(healthz))
        .with_state(state)
}

async fn scan(
    AxState(state): AxState<Arc<State>>,
    Json(req): Json<ScanRequest>,
) -> impl IntoResponse {
    let resp: ScanResponse = handle(&req);
    let _ = state.store.insert(resp.report.clone());
    (StatusCode::OK, Json(resp))
}

async fn list_reports(AxState(state): AxState<Arc<State>>) -> impl IntoResponse {
    Json(state.store.ids())
}

async fn get_report(
    AxState(state): AxState<Arc<State>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.store.get(&id) {
        Some(r) => (StatusCode::OK, Json(r)).into_response(),
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

async fn healthz() -> impl IntoResponse {
    Json(serde_json::json!({
        "ok": true,
        "version": crate::UPSTREAM_VERSION,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn router_builds() {
        let s = Arc::new(State::default());
        let _r = create_router(s);
    }

    #[tokio::test]
    async fn healthz_returns_version() {
        let r = healthz().await.into_response();
        assert_eq!(r.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn list_reports_empty() {
        let s = Arc::new(State::default());
        let resp = list_reports(AxState(s)).await.into_response();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn get_report_missing_404() {
        let s = Arc::new(State::default());
        let resp = get_report(AxState(s), Path("nope".into())).await.into_response();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn scan_persists_to_store() {
        let s = Arc::new(State::default());
        let req = ScanRequest {
            target: crate::server::ScanTarget::Image,
            artifact_name: "x".into(),
            min_severity: None,
            only_fixed: false,
            format: crate::server::ReportFormat::Json,
            body: serde_json::Value::Null,
        };
        let resp = scan(AxState(s.clone()), Json(req)).await.into_response();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(s.store.count().unwrap(), 1);
    }
}
