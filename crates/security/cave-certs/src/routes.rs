// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP routes for cave-certs.

use axum::{Json, Router, routing::get};

pub fn create_router() -> Router {
    Router::new().route("/api/certs/health", get(health))
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-certs",
        "status": "ok",
        "upstream": "cert-manager",
        "features": "ACME/Lets Encrypt, cert issuance, auto-renewal, expiry alerting, K8s CRDs"
    }))
}
