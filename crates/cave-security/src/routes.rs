//! HTTP routes for cave-security.

use crate::{
    models::{
        Condition, Priority, SbomDocument, SbomFormat, ScanResult, SecurityAlert, SecurityEvent,
        SecurityRule, Vulnerability,
    },
    rules::evaluate_rules,
    scanner::{generate_sbom, sample_cve_db, scan_image, InstalledPackage},
    SecurityState,
};
use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router(state: Arc<SecurityState>) -> Router {
    Router::new()
        // Rules CRUD
        .route("/api/v1/rules", get(list_rules).post(create_rule))
        .route(
            "/api/v1/rules/:id",
            get(get_rule).put(update_rule).delete(delete_rule),
        )
        // Alerts: ingest events and query alerts
        .route("/api/v1/alerts", get(list_alerts).post(ingest_event))
        .route("/api/v1/alerts/:id/acknowledge", post(acknowledge_alert))
        // Scans
        .route("/api/v1/scans", get(list_scans).post(trigger_scan))
        // Vulnerabilities (aggregated from all scans)
        .route("/api/v1/vulnerabilities", get(list_vulnerabilities))
        // SBOM generation
        .route("/api/v1/sbom", post(generate_sbom_endpoint))
        // Health
        .route("/api/v1/security/health", get(health))
        .with_state(state)
}

// ── Rules ────────────────────────────────────────────────────────────────────

async fn list_rules(State(state): State<Arc<SecurityState>>) -> Json<Vec<SecurityRule>> {
    Json(state.rules.read().await.clone())
}

#[derive(Deserialize)]
pub struct CreateRuleRequest {
    pub name: String,
    pub description: String,
    pub priority: Priority,
    pub condition: Condition,
    pub tags: Option<Vec<String>>,
}

async fn create_rule(
    State(state): State<Arc<SecurityState>>,
    Json(req): Json<CreateRuleRequest>,
) -> Json<SecurityRule> {
    let mut rule = SecurityRule::new(req.name, req.description, req.priority, req.condition);
    if let Some(tags) = req.tags {
        rule.tags = tags;
    }
    let mut rules = state.rules.write().await;
    rules.push(rule.clone());
    Json(rule)
}

async fn get_rule(
    State(state): State<Arc<SecurityState>>,
    Path(id): Path<Uuid>,
) -> Json<Option<SecurityRule>> {
    Json(state.rules.read().await.iter().find(|r| r.id == id).cloned())
}

async fn update_rule(
    State(state): State<Arc<SecurityState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateRuleRequest>,
) -> Json<Option<SecurityRule>> {
    let mut rules = state.rules.write().await;
    if let Some(rule) = rules.iter_mut().find(|r| r.id == id) {
        rule.name = req.name;
        rule.description = req.description;
        rule.priority = req.priority;
        rule.condition = req.condition;
        if let Some(tags) = req.tags {
            rule.tags = tags;
        }
        rule.updated_at = Utc::now();
        Json(Some(rule.clone()))
    } else {
        Json(None)
    }
}

async fn delete_rule(
    State(state): State<Arc<SecurityState>>,
    Path(id): Path<Uuid>,
) -> Json<bool> {
    let mut rules = state.rules.write().await;
    let before = rules.len();
    rules.retain(|r| r.id != id);
    Json(rules.len() < before)
}

// ── Alerts ───────────────────────────────────────────────────────────────────

async fn list_alerts(State(state): State<Arc<SecurityState>>) -> Json<Vec<SecurityAlert>> {
    Json(state.alerts.read().await.clone())
}

async fn ingest_event(
    State(state): State<Arc<SecurityState>>,
    Json(event): Json<SecurityEvent>,
) -> Json<Vec<SecurityAlert>> {
    let new_alerts = {
        let rules = state.rules.read().await;
        evaluate_rules(&rules, &event)
    };
    state.alerts.write().await.extend(new_alerts.clone());
    Json(new_alerts)
}

async fn acknowledge_alert(
    State(state): State<Arc<SecurityState>>,
    Path(id): Path<Uuid>,
) -> Json<bool> {
    let mut alerts = state.alerts.write().await;
    if let Some(alert) = alerts.iter_mut().find(|a| a.id == id) {
        alert.acknowledged = true;
        Json(true)
    } else {
        Json(false)
    }
}

// ── Scans ────────────────────────────────────────────────────────────────────

async fn list_scans(State(state): State<Arc<SecurityState>>) -> Json<Vec<ScanResult>> {
    Json(state.scans.read().await.clone())
}

#[derive(Deserialize)]
pub struct TriggerScanRequest {
    pub image_reference: String,
    pub image_digest: Option<String>,
    pub packages: Vec<PackageDto>,
}

#[derive(Deserialize)]
pub struct PackageDto {
    pub name: String,
    pub version: String,
}

async fn trigger_scan(
    State(state): State<Arc<SecurityState>>,
    Json(req): Json<TriggerScanRequest>,
) -> Json<ScanResult> {
    let packages: Vec<InstalledPackage> = req
        .packages
        .iter()
        .map(|p| InstalledPackage {
            name: p.name.clone(),
            version: p.version.clone(),
            layer_digest: None,
        })
        .collect();
    let digest = req.image_digest.as_deref().unwrap_or("sha256:unknown");
    let db = sample_cve_db();
    let result = {
        let policy = state.policy.read().await;
        scan_image(&req.image_reference, digest, &db, &packages, &policy)
    };
    state.scans.write().await.push(result.clone());
    Json(result)
}

// ── Vulnerabilities ──────────────────────────────────────────────────────────

async fn list_vulnerabilities(State(state): State<Arc<SecurityState>>) -> Json<Vec<Vulnerability>> {
    let vulns = state
        .scans
        .read()
        .await
        .iter()
        .flat_map(|s| s.vulnerabilities.clone())
        .collect();
    Json(vulns)
}

// ── SBOM ─────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SbomRequest {
    pub image_reference: String,
    pub packages: Vec<PackageDto>,
    pub format: Option<String>,
}

async fn generate_sbom_endpoint(
    State(_state): State<Arc<SecurityState>>,
    Json(req): Json<SbomRequest>,
) -> Json<SbomDocument> {
    let packages: Vec<InstalledPackage> = req
        .packages
        .iter()
        .map(|p| InstalledPackage {
            name: p.name.clone(),
            version: p.version.clone(),
            layer_digest: None,
        })
        .collect();
    let fmt = match req.format.as_deref() {
        Some("cyclonedx") => SbomFormat::CycloneDx,
        _ => SbomFormat::Spdx,
    };
    Json(generate_sbom(&req.image_reference, &packages, fmt))
}

// ── Health ───────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct HealthResponse {
    module: &'static str,
    status: &'static str,
    upstream: &'static str,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        module: "cave-security",
        status: "ok",
        upstream: "Falco + Trivy",
    })
}
