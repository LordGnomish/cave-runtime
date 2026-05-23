// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Axum routes — Portal API surface for cave-trufflehog. Mirrors the
//! cavectl-side `cave secret {scan,verify,detect,custom}` verbs.

use crate::State;
use crate::custom_detectors::{compile, load_spec_yaml};
use crate::engine::Engine;
use crate::models::{Chunk, Finding};
use axum::{
    Json, Router,
    extract::State as AxumState,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub fn create_router(state: Arc<State>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/api/secret/detectors", get(list_detectors))
        .route("/api/secret/scan", post(scan))
        .route("/api/secret/findings", get(findings))
        .route("/api/secret/detect", post(detect))
        .route("/api/secret/custom", post(custom))
        .route("/api/secret/verify", post(verify))
        .route("/api/secret/metrics", get(metrics))
        .route("/api/secret/alerts", get(alerts))
        .with_state(state)
}

async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

#[derive(Debug, Deserialize)]
pub struct ScanRequest {
    pub data: String,
    pub source: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ScanResponse {
    pub findings: Vec<Finding>,
    pub count: usize,
}

async fn scan(
    AxumState(state): AxumState<Arc<State>>,
    Json(req): Json<ScanRequest>,
) -> Json<ScanResponse> {
    let engine = Engine::new(state.config.clone());
    let chunk = Chunk::new(
        req.source.as_deref().unwrap_or("api"),
        "request",
        req.data.into_bytes(),
    );
    let f = engine.scan_chunk(&chunk);
    Json(ScanResponse {
        count: f.len(),
        findings: f,
    })
}

async fn findings(AxumState(state): AxumState<Arc<State>>) -> Json<Vec<Finding>> {
    Json(
        state
            .store
            .all()
            .into_iter()
            .map(|s| s.finding)
            .collect(),
    )
}

#[derive(Debug, Serialize)]
pub struct DetectorEntry {
    pub name: &'static str,
    pub description: &'static str,
    pub keywords: Vec<String>,
}

async fn list_detectors(AxumState(state): AxumState<Arc<State>>) -> Json<Vec<DetectorEntry>> {
    Json(
        state
            .registry
            .detectors
            .iter()
            .map(|d| DetectorEntry {
                name: d.detector_type().name(),
                description: d.description(),
                keywords: d.keywords().iter().map(|s| (*s).to_string()).collect(),
            })
            .collect(),
    )
}

async fn detect(
    AxumState(state): AxumState<Arc<State>>,
    Json(req): Json<ScanRequest>,
) -> Json<usize> {
    Json(state.registry.scan(req.data.as_bytes()).len())
}

#[derive(Debug, Deserialize)]
pub struct CustomRequest {
    pub yaml: String,
    pub sample: String,
}

#[derive(Debug, Serialize)]
pub struct CustomResponse {
    pub detector: String,
    pub matches: usize,
}

async fn custom(Json(req): Json<CustomRequest>) -> Result<Json<CustomResponse>, (StatusCode, String)> {
    let specs = load_spec_yaml(&req.yaml).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let spec = specs
        .into_iter()
        .next()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "no detector in YAML".into()))?;
    let cd = compile(spec).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let r = cd.scan(req.sample.as_bytes());
    Ok(Json(CustomResponse {
        detector: cd.name,
        matches: r.len(),
    }))
}

#[derive(Debug, Deserialize)]
pub struct VerifyRequest {
    pub raw: String,
    pub detector: String,
}

#[derive(Debug, Serialize)]
pub struct VerifyResponse {
    pub detector: String,
    pub raw: String,
    pub verdict: &'static str,
}

async fn verify(Json(req): Json<VerifyRequest>) -> Json<VerifyResponse> {
    // We do not actually issue a live HTTP request here; verifying upstream
    // tokens from a server-side endpoint would leak the secret further.
    // The cavectl path issues the verification request locally where the
    // operator already has the secret on disk.
    Json(VerifyResponse {
        detector: req.detector,
        raw: crate::engine::redact(&req.raw),
        verdict: "indeterminate",
    })
}

async fn metrics() -> Json<Vec<crate::metrics::PanelSpec>> {
    Json(crate::metrics::dashboard_panels())
}

async fn alerts() -> Json<Vec<crate::metrics::AlertSpec>> {
    Json(crate::metrics::alert_rules())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_function_compiles_with_routes() {
        assert_eq!(crate::engine::redact("123456789"), "1234…6789");
    }

    #[test]
    fn router_builds() {
        let s = Arc::new(State::default());
        let _r = create_router(s);
    }
}
