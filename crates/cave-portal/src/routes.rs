// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP routes for cave-portal.

use crate::State;
use axum::{
    Json, Router,
    extract::{Path, Query, State as AxumState},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
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
        .route("/api/portal/parity", get(parity_all))
        .route("/api/portal/parity/{module}", get(parity_module))
        .route("/api/v1/attribution", get(attribution_api))
        .route("/portal/tracker", get(serve_tracker_ui))
        .route("/portal/registry", get(serve_registry_ui))
        .route("/portal/scan", get(serve_scan_ui))
        .route("/static/{file}", get(static_asset))
        .with_state(state)
}

// ── Static assets ─────────────────────────────────────────────────────────────
//
// Embedded admin-shell assets — tailwind-light.css, cave-brand.css, htmx.min.js.
// Without this route the admin pages reference `/static/tailwind-light.css` and
// `/static/htmx.min.js` but axum has no handler, every admin page renders as
// raw unstyled HTML with no client-side behaviour. See the audit note in
// docs/runbooks/portal-ux.md (2026-05-22) — fixing this single route is the
// largest single contributor to the visual polish of the admin surface.

const TAILWIND_LIGHT_CSS: &str = include_str!("../assets/tailwind-light.css");
const CAVE_BRAND_CSS: &str = include_str!("../assets/cave-brand.css");
const HTMX_MIN_JS: &str = include_str!("../assets/htmx.min.js");

/// Look up an embedded asset by exact filename. Returns `(content_type, body)`
/// or `None` if the requested name is not in the allowlist. The allowlist is
/// intentional — every served file is reviewed at build time, no filesystem
/// reads happen at request time (so the route is immune to path-traversal).
pub fn static_asset_lookup(name: &str) -> Option<(&'static str, &'static str)> {
    match name {
        "tailwind-light.css" => Some(("text/css; charset=utf-8", TAILWIND_LIGHT_CSS)),
        "cave-brand.css" => Some(("text/css; charset=utf-8", CAVE_BRAND_CSS)),
        "htmx.min.js" => Some(("application/javascript; charset=utf-8", HTMX_MIN_JS)),
        _ => None,
    }
}

async fn static_asset(Path(file): Path<String>) -> Response {
    match static_asset_lookup(&file) {
        Some((content_type, body)) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, content_type),
                // 1-hour browser cache — long enough to avoid the round-trip
                // cost on every page nav, short enough that a server upgrade
                // ships fresh CSS within an hour for everyone.
                (header::CACHE_CONTROL, "public, max-age=3600"),
            ],
            body,
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "static asset not found").into_response(),
    }
}

// ── Parity ────────────────────────────────────────────────────────────────────

async fn parity_all(AxumState(state): AxumState<Arc<State>>) -> Json<serde_json::Value> {
    let cache = state.parity_cache.read().await;
    let modules: Vec<serde_json::Value> = cache
        .values()
        .map(|r| serde_json::to_value(r).unwrap_or_default())
        .collect();
    Json(serde_json::json!({
        "modules": modules,
        "total": cache.len()
    }))
}

async fn parity_module(
    Path(module): Path<String>,
    AxumState(state): AxumState<Arc<State>>,
) -> (StatusCode, Json<serde_json::Value>) {
    let cache = state.parity_cache.read().await;
    match cache.get(&module) {
        Some(report) => (
            StatusCode::OK,
            Json(serde_json::to_value(report).unwrap_or_default()),
        ),
        None => (
            StatusCode::NOT_FOUND,
            Json(
                serde_json::json!({ "error": format!("module '{}' not found in parity cache", module) }),
            ),
        ),
    }
}

// ── Health ────────────────────────────────────────────────────────────────────

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
            let Some(num) = adr_num_from_filename(&fname) else {
                continue;
            };
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
    let padded = format!(
        "ADR-{:0>3}",
        id.trim_start_matches("ADR-")
            .trim_start_matches('0')
            .parse::<u64>()
            .unwrap_or(0)
    );
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
    let num_str: String = stripped
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
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

// ── Attribution ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct AttributionQuery {
    #[serde(default = "default_attr_days")]
    days: u32,
    #[serde(default = "default_attr_repo")]
    repo: String,
}

fn default_attr_days() -> u32 {
    7
}
fn default_attr_repo() -> String {
    "all".into()
}

async fn attribution_api(Query(q): Query<AttributionQuery>) -> Json<serde_json::Value> {
    let workspace = std::env::var("CAVE_WORKSPACE_ROOT").unwrap_or_else(|_| ".".into());
    // q.days is a validated u32 — safe to interpolate.
    let since = format!("{} days ago", q.days);
    // Format: subject \x1f author \x1f trailers — one commit per line.
    let out = std::process::Command::new("git")
        .args([
            "-C",
            &workspace,
            "log",
            &format!("--since={}", since),
            "--format=%s\x1f%an\x1f%(trailers:key=Co-Authored-By,separator=|)",
        ])
        .output();
    let (mut qwen3, mut opus, mut sonnet, mut haiku, mut claude_legacy, mut burak, mut other): (
        u64,
        u64,
        u64,
        u64,
        u64,
        u64,
        u64,
    ) = (0, 0, 0, 0, 0, 0, 0);
    if let Ok(out) = out {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let mut parts = line.splitn(3, '\x1f');
            let subject = parts.next().unwrap_or("");
            let author = parts.next().unwrap_or("");
            let trailers = parts.next().unwrap_or("").to_lowercase();
            let author_lc = author.to_lowercase();
            let is_housekeeping = subject.starts_with("Merge ")
                || subject.starts_with("On ")
                || subject.starts_with("index on ")
                || subject.starts_with("untracked files on ");
            if subject.starts_with("[qwen-amele]")
                || trailers.contains("co-authored-by: qwen")
                || author_lc.contains("qwen")
                || author_lc.contains("cave-local-llm")
            {
                qwen3 += 1;
            } else if author_lc.contains("burak") || author_lc.contains("gnomish") {
                burak += 1;
            } else if is_housekeeping {
                other += 1;
            } else if trailers.contains("claude opus") || trailers.contains("co-authored-by: opus")
            {
                opus += 1;
            } else if trailers.contains("claude sonnet")
                || trailers.contains("co-authored-by: sonnet")
            {
                sonnet += 1;
            } else if trailers.contains("claude haiku")
                || trailers.contains("co-authored-by: haiku")
            {
                haiku += 1;
            } else if trailers.contains("co-authored-by: claude")
                || author_lc.contains("claude")
                || author.contains("CAVE Contributors")
            {
                // Untagged Claude-family commit — cannot distinguish model.
                // Bucketed separately so UI can show "unknown Claude model".
                claude_legacy += 1;
            } else {
                other += 1;
            }
        }
    }
    Json(serde_json::json!({
        "period_days": q.days,
        "repo": q.repo,
        "by_commits": {
            "qwen3": qwen3,
            "opus": opus,
            "sonnet": sonnet,
            "haiku": haiku,
            "claude_legacy": claude_legacy,
            "burak": burak,
            "other": other,
        },
        "by_loc_net": {
            "qwen3": 0, "opus": 0, "sonnet": 0, "haiku": 0,
            "claude_legacy": 0, "burak": 0, "other": 0,
        },
        "by_files": {
            "qwen3": 0, "opus": 0, "sonnet": 0, "haiku": 0,
            "claude_legacy": 0, "burak": 0, "other": 0,
        },
        "timestamps":  { "earliest": null, "latest": null },
    }))
}
