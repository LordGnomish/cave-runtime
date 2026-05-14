// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP routes for cave-secrets.

use crate::SecretsState;
use axum::{extract::State, routing::{get, post}, Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub fn create_router(state: Arc<SecretsState>) -> Router {
    Router::new()
        .route("/api/secrets/scan", post(scan_content))
        .route("/api/secrets/detectors", get(list_detectors))
        .route("/api/secrets/health", get(health))
        .with_state(state)
}

#[derive(Deserialize)]
pub struct ScanRequest {
    pub content: String,
    pub filename: String,
}

#[derive(Serialize)]
pub struct ScanResponse {
    pub findings: Vec<FindingDto>,
    pub scanned_lines: usize,
}

#[derive(Serialize)]
pub struct FindingDto {
    pub detector: String,
    pub file: String,
    pub line: usize,
    pub severity: String,
}

async fn scan_content(
    State(state): State<Arc<SecretsState>>,
    Json(req): Json<ScanRequest>,
) -> Json<ScanResponse> {
    let findings = crate::detector::scan(&req.content, &req.filename, &state.detectors);
    let lines = req.content.lines().count();
    Json(ScanResponse {
        findings: findings.iter().map(|f| FindingDto {
            detector: f.detector.clone(),
            file: f.file.clone(),
            line: f.line,
            severity: format!("{:?}", f.severity),
        }).collect(),
        scanned_lines: lines,
    })
}

async fn list_detectors(State(state): State<Arc<SecretsState>>) -> Json<Vec<String>> {
    Json(state.detectors.iter().map(|d| d.name.to_string()).collect())
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-secrets",
        "status": "ok",
        "upstream": "trufflehog + gitleaks"
    }))
}
