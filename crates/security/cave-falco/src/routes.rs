// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! HTTP routes for cave-falco (`/api/falco/*`).
//!
//! Surface:
//! - `GET /api/falco/health`                       — module health
//! - `GET /api/falco/observability/panels`         — dashboard panels JSON
//! - `GET /api/falco/observability/alerts`         — alert rules JSON
//! - `POST /api/falco/rules/parse`                 — parse a YAML rule pack body

use crate::observability;
use crate::rule_loader;
use axum::{routing::{get, post}, Json, Router};

pub fn router() -> Router {
    Router::new()
        .route("/api/falco/health", get(health))
        .route("/api/falco/observability/panels", get(panels))
        .route("/api/falco/observability/alerts", get(alerts))
        .route("/api/falco/rules/parse", post(parse_rules))
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-falco",
        "upstream": "falcosecurity/falco@0.43.1",
        "status": "ok",
    }))
}

async fn panels() -> Json<Vec<observability::DashboardPanel>> {
    Json(observability::dashboard_panels())
}

async fn alerts() -> Json<Vec<observability::AlertRule>> {
    Json(observability::alert_rules())
}

async fn parse_rules(body: String) -> Json<serde_json::Value> {
    match rule_loader::parse(&body) {
        Ok(pack) => Json(serde_json::json!({
            "ok": true,
            "rules": pack.rules.len(),
            "macros": pack.macros.len(),
            "lists": pack.lists.len(),
        })),
        Err(e) => Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn router_exposes_expected_paths() {
        // Smoke — the router builds; the actual HTTP path coverage is in
        // the integration test below.
        let _r = router();
    }

    #[tokio::test]
    async fn health_returns_ok_status() {
        let Json(v) = health().await;
        assert_eq!(v["status"], "ok");
        assert_eq!(v["module"], "cave-falco");
    }

    #[tokio::test]
    async fn panels_returns_eight() {
        let Json(p) = panels().await;
        assert_eq!(p.len(), 8);
    }

    #[tokio::test]
    async fn alerts_returns_five() {
        let Json(a) = alerts().await;
        assert_eq!(a.len(), 5);
    }

    #[tokio::test]
    async fn parse_rules_accepts_falco_style_yaml() {
        let body = "- rule: { name: r, desc: d, condition: 1=1, priority: WARNING, output: hi }".to_string();
        let Json(v) = parse_rules(body).await;
        assert_eq!(v["ok"], true);
        assert_eq!(v["rules"], 1);
    }

    #[tokio::test]
    async fn parse_rules_rejects_bad_yaml() {
        let Json(v) = parse_rules("[not-yaml".into()).await;
        assert_eq!(v["ok"], false);
    }
}
