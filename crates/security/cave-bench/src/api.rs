// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! axum HTTP routes — POST /scan, GET /findings, GET /profiles, GET /checks.

use crate::State;
use axum::{Json, Router, extract, http::StatusCode, response::IntoResponse, routing::{get, post}};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub fn create_router(state: Arc<State>) -> Router {
    Router::new()
        .route("/api/bench/health", get(health))
        .route("/api/bench/scan", post(post_scan))
        .route("/api/bench/findings", get(list_findings))
        .route("/api/bench/findings/failures", get(list_failures))
        .route("/api/bench/profiles", get(list_profiles))
        .route("/api/bench/checks", get(list_checks))
        .route("/api/bench/schedules", get(list_schedules))
        .route("/api/bench/observability/panels", get(panels))
        .route("/api/bench/observability/alerts", get(alerts))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-bench",
        "status": "ok",
        "upstream_kube_bench": "aquasecurity/kube-bench v0.15.5",
        "upstream_kubescape": "kubescape/kubescape v4.0.8",
    }))
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScanRequest {
    pub profile_id: String,
    pub host: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScanResponse {
    pub scan_id: String,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub score: f64,
}

async fn post_scan(
    extract::State(state): extract::State<Arc<State>>,
    Json(req): Json<ScanRequest>,
) -> Result<Json<ScanResponse>, (StatusCode, Json<serde_json::Value>)> {
    let profile = crate::profile::find_profile(&req.profile_id).map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        )
    })?;
    let target = crate::models::Target::host_files("/etc/kubernetes", req.host.clone());
    let input = crate::runner::ScanInput::new(req.host.clone());
    let (findings, summary) = crate::runner::run_profile(&profile, &target, &input, crate::runner::RunMode::Sequential);
    let resp = ScanResponse {
        scan_id: summary.scan_id.clone(),
        total: summary.total,
        passed: summary.passed,
        failed: summary.failed,
        score: summary.score,
    };
    state.findings.record(summary, findings);
    Ok(Json(resp))
}

async fn list_findings(extract::State(state): extract::State<Arc<State>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "scans": state.findings.list_summaries(),
        "count": state.findings.count(),
    }))
}

async fn list_failures(extract::State(state): extract::State<Arc<State>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "failures": state.findings.list_failures(),
    }))
}

async fn list_profiles() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "profiles": crate::profile::builtin_profiles(),
    }))
}

async fn list_checks() -> Json<serde_json::Value> {
    let cis_meta: Vec<_> = crate::runner::cis_pairs().into_iter().map(|(c, _)| c).collect();
    let nsa_meta: Vec<_> = crate::kubescape_nsa::nsa_controls().into_iter().map(|c| c.check).collect();
    let mitre_meta: Vec<_> = crate::kubescape_mitre::mitre_techniques().into_iter().map(|t| t.check).collect();
    Json(serde_json::json!({
        "cis": cis_meta,
        "nsa": nsa_meta,
        "mitre": mitre_meta,
    }))
}

async fn list_schedules(extract::State(state): extract::State<Arc<State>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "schedules": state.schedules.list(),
        "count": state.schedules.count(),
    }))
}

async fn panels() -> Json<serde_json::Value> {
    Json(serde_json::json!({"panels": crate::observability::dashboard_panels()}))
}

async fn alerts() -> Json<serde_json::Value> {
    Json(serde_json::json!({"alerts": crate::observability::alert_rules()}))
}

impl IntoResponse for crate::error::BenchError {
    fn into_response(self) -> axum::response::Response {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": self.to_string()}))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_router_constructs() {
        let s = Arc::new(State::default());
        let _r = create_router(s);
    }

    #[test]
    fn test_scan_request_deserialize() {
        let j = r#"{"profile_id":"cis-1.10","host":"node-1"}"#;
        let req: ScanRequest = serde_json::from_str(j).unwrap();
        assert_eq!(req.profile_id, "cis-1.10");
        assert_eq!(req.host, "node-1");
    }

    #[test]
    fn test_scan_response_serialize() {
        let r = ScanResponse {
            scan_id: "s1".into(),
            total: 10,
            passed: 7,
            failed: 3,
            score: 0.7,
        };
        let j = serde_json::to_string(&r).unwrap();
        assert!(j.contains("\"scan_id\":\"s1\""));
    }
}
