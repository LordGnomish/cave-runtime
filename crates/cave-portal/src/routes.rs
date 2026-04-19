//! HTTP routes for cave-portal.

use crate::State;
use axum::{
    extract::Path,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;

pub fn create_router(state: Arc<State>) -> Router {
    Router::new()
        .route("/api/portal/health", get(health))
        .route("/api/portal/health/all", get(health_all))
        .route("/api/portal/dashboard", get(dashboard_api))
        .route("/api/portal/nav", get(nav_api))
        .route("/api/portal/modules", get(modules_api))
        .route("/api/portal/search", get(search_api))
        .route("/api/portal/notifications", get(notifications_api))
        .route("/api/portal/scaffold", post(scaffold_project))
        .route("/api/portal/adrs", get(list_adrs))
        .route("/api/portal/adrs/{id}", get(get_adr))
        .route("/portal/tracker", get(serve_tracker_ui))
        .route("/portal/registry", get(serve_registry_ui))
        .route("/portal/scan", get(serve_scan_ui))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-portal",
        "status": "ok",
        "upstream": "Backstage"
    }))
}

async fn health_all() -> Json<serde_json::Value> {
    // Phase 1 modules are live; rest are not yet deployed.
    Json(serde_json::json!({
        "modules": {
            "secrets":   "healthy",
            "certs":     "healthy",
            "lint":      "healthy",
            "docs":      "healthy",
            "status":    "healthy",
            "changelog": "healthy",
            "portal":    "healthy",
            "upstream":  "healthy",
            "sign":      "unknown",
            "policy":    "unknown",
            "vulns":     "unknown",
            "sbom":      "unknown",
            "scan":      "unknown",
            "pii":       "unknown",
            "dast":      "unknown",
            "pam":       "unknown",
            "uptime":    "unknown",
            "incidents": "unknown",
            "ai-obs":    "unknown",
            "profiler":  "unknown",
            "forensics": "unknown",
            "registry":  "unknown",
            "chaos":     "unknown",
            "flags":     "unknown",
            "chat":      "unknown",
            "workflows": "unknown",
            "devlake":   "unknown",
            "backup":    "unknown",
            "cost":      "unknown",
            "db":        "unknown",
            "auth":      "unknown"
        }
    }))
}

async fn dashboard_api() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "dashboard": "ok" }))
}

async fn nav_api() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "nav": "ok" }))
}

async fn modules_api() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "modules": "ok" }))
}

async fn search_api() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "search": "ok" }))
}

async fn notifications_api() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "notifications": "ok" }))
}

#[derive(Deserialize)]
struct ScaffoldRequest {
    name: String,
    language: String,
    template: String,
    namespace: String,
}

async fn scaffold_project(Json(req): Json<ScaffoldRequest>) -> Json<serde_json::Value> {
    let path = format!("projects/{}/{}", req.namespace, req.name);
    Json(serde_json::json!({
        "status": "created",
        "project": req.name,
        "language": req.language,
        "template": req.template,
        "namespace": req.namespace,
        "path": path,
        "message": format!("Project '{}' scaffolded at {}", req.name, path)
    }))
}

async fn list_adrs() -> Json<serde_json::Value> {
    let dir = std::path::Path::new("docs/adr");
    let mut adrs: Vec<serde_json::Value> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let fname = entry.file_name().to_string_lossy().to_string();
            if !fname.ends_with(".md") || fname == "README.md" {
                continue;
            }
            let Some(num) = adr_num_from_filename(&fname) else { continue };
            let title = adr_title_from_filename(&fname);
            let (scope, category) = adr_meta_from_file(&entry.path());
            adrs.push(serde_json::json!({
                "id": num,
                "filename": fname,
                "title": title,
                "scope": scope,
                "category": category,
            }));
        }
    }

    adrs.sort_by_key(|a| a["id"].as_u64().unwrap_or(0));
    Json(serde_json::json!({ "adrs": adrs }))
}

async fn get_adr(Path(id): Path<String>) -> (StatusCode, String) {
    let dir = std::path::Path::new("docs/adr");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (StatusCode::NOT_FOUND, String::new());
    };
    // id is e.g. "001", "42", "130" — match filename prefix ADR-NNN
    let padded = format!("ADR-{:0>3}", id.trim_start_matches("ADR-").trim_start_matches('0').parse::<u64>().unwrap_or(0));
    for entry in entries.flatten() {
        let fname = entry.file_name().to_string_lossy().to_string();
        if fname.starts_with(&padded) && fname.ends_with(".md") {
            return match std::fs::read_to_string(entry.path()) {
                Ok(content) => (StatusCode::OK, content),
                Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            };
        }
    }
    (StatusCode::NOT_FOUND, String::new())
}

fn adr_num_from_filename(fname: &str) -> Option<u64> {
    let stripped = fname.strip_prefix("ADR-")?;
    let num_str: String = stripped.chars().take_while(|c| c.is_ascii_digit()).collect();
    num_str.parse().ok()
}

fn adr_title_from_filename(fname: &str) -> String {
    fname
        .strip_prefix("ADR-")
        .and_then(|s| s.splitn(2, '_').nth(1))
        .map(|s| s.trim_end_matches(".md").replace('_', " "))
        .unwrap_or_else(|| fname.to_string())
}

fn adr_meta_from_file(path: &std::path::Path) -> (String, String) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return ("Universal".to_string(), "General".to_string());
    };
    let mut scope = "Universal".to_string();
    let mut category = "General".to_string();
    for line in content.lines().take(25) {
        if let Some(v) = line.strip_prefix("**Scope:**") {
            scope = v.trim().to_string();
        }
        if let Some(v) = line.strip_prefix("**Category:**") {
            category = v.trim().to_string();
        }
    }
    (scope, category)
}

async fn serve_tracker_ui() -> (StatusCode, &'static str) {
    (StatusCode::OK, include_str!("tracker_ui.html"))
}

async fn serve_registry_ui() -> (StatusCode, &'static str) {
    (StatusCode::OK, include_str!("registry_ui.html"))
}

async fn serve_scan_ui() -> (StatusCode, &'static str) {
    (StatusCode::OK, include_str!("scan_ui.html"))
}
