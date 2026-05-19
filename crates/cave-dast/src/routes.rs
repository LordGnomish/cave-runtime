// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: zaproxy/zaproxy@v2.14.0
//   zap/src/main/java/org/zaproxy/zap/extension/api/CoreAPI.java
//
//! HTTP routes for cave-dast — ZAP-style REST surface.

use crate::State;
use axum::{routing::get, Json, Router};
use std::sync::Arc;

pub fn create_router(state: Arc<State>) -> Router {
    Router::new()
        .route("/api/dast/health", get(health))
        .route("/api/dast/version", get(version))
        .route("/api/dast/rules/active", get(active_rules))
        .route("/api/dast/rules/passive", get(passive_rules))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-dast",
        "status": "ok",
        "upstream": "OWASP ZAP"
    }))
}

async fn version() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-dast",
        "upstream": "OWASP ZAP",
        "upstream_version": crate::UPSTREAM_VERSION,
        "upstream_sha": crate::UPSTREAM_SHA,
    }))
}

async fn active_rules() -> Json<serde_json::Value> {
    let reg = crate::ascan::ScanPluginRegistry::with_baseline();
    let rows: Vec<_> = reg
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id(),
                "name": r.name(),
                "risk": format!("{:?}", r.risk()),
                "cwe_id": r.cwe_id(),
                "wasc_id": r.wasc_id(),
            })
        })
        .collect();
    Json(serde_json::json!({ "rules": rows }))
}

async fn passive_rules() -> Json<serde_json::Value> {
    let reg = crate::pscan::PassiveScanRegistry::with_baseline();
    let rows: Vec<_> = reg
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id(),
                "name": r.name(),
                "risk": format!("{:?}", r.risk()),
                "cwe_id": r.cwe_id(),
            })
        })
        .collect();
    Json(serde_json::json!({ "rules": rows }))
}
