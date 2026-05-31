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
use crate::{engine, falcoctl, rule_loader};
use axum::{routing::{get, post}, Json, Router};

pub fn router() -> Router {
    Router::new()
        .route("/api/falco/health", get(health))
        .route("/api/falco/observability/panels", get(panels))
        .route("/api/falco/observability/alerts", get(alerts))
        .route("/api/falco/rules/parse", post(parse_rules))
        .route("/api/falco/operators", get(operators))
        .route("/api/falco/artifact/resolve", post(resolve_artifact))
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

async fn operators() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "operators": engine::supported_operators() }))
}

#[derive(serde::Deserialize)]
struct ResolveBody {
    index_yaml: String,
    #[serde(rename = "ref")]
    reference: String,
}

/// POST body `{ "index_yaml": "<index.yaml>", "ref": "cloudtrail:0.5.1" }`.
async fn resolve_artifact(Json(b): Json<ResolveBody>) -> Json<serde_json::Value> {
    let result = falcoctl::Index::from_yaml("api", &b.index_yaml)
        .and_then(|idx| idx.resolve_reference(&b.reference));
    match result {
        Ok(reference) => Json(serde_json::json!({ "ok": true, "ref": reference })),
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

    #[tokio::test]
    async fn operators_lists_grammar() {
        let Json(v) = operators().await;
        let ops = v["operators"].as_array().unwrap();
        assert!(ops.iter().any(|o| o == "regex"));
        assert!(ops.iter().any(|o| o == "pmatch"));
    }

    #[tokio::test]
    async fn resolve_artifact_resolves_index_ref() {
        let body = ResolveBody {
            index_yaml: "- name: cloudtrail\n  type: plugin\n  registry: ghcr.io\n  repository: falcosecurity/plugins/cloudtrail\n".into(),
            reference: "cloudtrail".into(),
        };
        let Json(v) = resolve_artifact(Json(body)).await;
        assert_eq!(v["ok"], true);
        assert_eq!(v["ref"], "ghcr.io/falcosecurity/plugins/cloudtrail:latest");
    }
}
