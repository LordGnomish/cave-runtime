// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! ADR Browser page + API.
//!
//! Walks `docs/adr/*.md`, extracts (id, title, category, file path) and
//! serves a sortable, filterable index. Clicking a row pulls the markdown
//! and renders it via `pulldown_cmark`.
//!
//! Routes
//! ──────
//!   GET  /adr                    → ADR browser HTML page
//!   GET  /api/adr                → JSON list `[{id, title, category, file, …}]`
//!   GET  /api/adr/{id}           → `{ id, title, file, markdown, html }`

use std::path::PathBuf;

use axum::{
    extract::Path,
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
    Json, Router,
};
use cave_upstream::adr_links;
use pulldown_cmark::{html as pd_html, Options, Parser};
use serde::Serialize;
use serde_json::json;

use super::workspace_root;

// PAGE_HTML and templates/adr.html were removed on 2026-05-14 when
// `/adr` was redirected to `/admin/adr` (the canonical ADR Browser in
// the admin shell, persona-gated, internal/ folder excluded). The
// fancy JS-driven dark-themed page lived inside cave-runtime; the
// admin one lives inside cave-portal with sidebar/breadcrumb wiring.
// Same dedup pattern as `/upstream` (2026-05-13). The `/api/adr` +
// `/api/adr/{id}` JSON endpoints below are kept for cavectl /
// scripts / external tools that may consume them.

#[derive(Debug, Clone, Serialize)]
pub struct AdrSummary {
    pub id: String,
    pub title: String,
    pub file: String,
    pub category: String,
    pub size_bytes: u64,
    pub linked_upstreams: Vec<String>,
}

/// GET /adr — **deprecated** as of 2026-05-14. Permanently redirects
/// to `/admin/adr`, the canonical ADR Browser (admin shell, persona-
/// aware, sibling to /admin/upstream / /admin/compliance).
///
/// History: Burak flagged the duplication ("ADR Browser, Decisions —
/// 2 ayrı sayfa ama aynı konsept"). Three places rendered the same
/// docs/adr content: `/adr` (this page), `/admin/adr` (canonical),
/// and the legacy `/` SPA "Decisions" tab. /adr now redirects; the
/// SPA tab was also removed in the same change.
pub async fn page() -> Response {
    Redirect::permanent("/admin/adr").into_response()
}

fn is_platform_admin(
    claims: Option<&axum::Extension<cave_auth::jwt_middleware::JwtClaims>>,
) -> bool {
    match claims {
        Some(axum::Extension(c)) => c.roles.iter().any(|r| r == "platform_admin"),
        None => false,
    }
}

pub async fn api_list(
    claims: Option<axum::Extension<cave_auth::jwt_middleware::JwtClaims>>,
) -> Response {
    if !is_platform_admin(claims.as_ref()) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "platform_admin role required" })),
        )
            .into_response();
    }
    let mut summaries = scan_adrs(&adr_dir());
    summaries.sort_by(|a, b| a.id.cmp(&b.id));

    // Bucket counts by category for the UI summary chips.
    let mut by_cat: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for s in &summaries {
        *by_cat.entry(s.category.clone()).or_default() += 1;
    }

    Json(json!({
        "total": summaries.len(),
        "categories": by_cat,
        "adrs": summaries,
    }))
    .into_response()
}

pub async fn api_get(
    Path(id): Path<String>,
    claims: Option<axum::Extension<cave_auth::jwt_middleware::JwtClaims>>,
) -> Response {
    if !is_platform_admin(claims.as_ref()) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "platform_admin role required" })),
        )
            .into_response();
    }
    let id = id.trim().to_string();
    let dir = adr_dir();
    let resolved = match resolve_adr_file(&dir, &id) {
        Some(p) => p,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": format!("ADR {id} not found in {}", dir.display()) })),
            )
                .into_response();
        }
    };
    let raw = match std::fs::read_to_string(&resolved) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("Cannot read {}: {}", resolved.display(), e) })),
            )
                .into_response();
        }
    };
    let title = extract_title(&raw).unwrap_or_else(|| id.clone());
    let html = render_markdown(&raw);
    Json(json!({
        "id": id,
        "title": title,
        "file": resolved.file_name().and_then(|s| s.to_str()).unwrap_or(""),
        "linked_upstreams": adr_links::upstreams_for(&id),
        "markdown": raw,
        "html": html,
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub fn adr_dir() -> PathBuf {
    workspace_root().join("docs").join("adr")
}

pub fn scan_adrs(dir: &std::path::Path) -> Vec<AdrSummary> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        let file_name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if !file_name.starts_with("ADR-") {
            continue;
        }
        let id = parse_id_from_filename(&file_name);
        let raw = std::fs::read_to_string(&path).unwrap_or_default();
        let title = extract_title(&raw).unwrap_or_else(|| id.clone());
        let category = classify(&id, &title);
        let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        out.push(AdrSummary {
            id: id.clone(),
            title,
            file: file_name,
            category,
            size_bytes,
            linked_upstreams: adr_links::upstreams_for(&id)
                .into_iter()
                .map(String::from)
                .collect(),
        });
    }
    out
}

/// Filenames look like `ADR-027_Kong_API_Gateway.md` or
/// `ADR-PORTAL-PERSONAS-001.md`. The id is the prefix up to the first `_`.
pub fn parse_id_from_filename(name: &str) -> String {
    let stem = name.trim_end_matches(".md");
    match stem.find('_') {
        Some(idx) => stem[..idx].to_string(),
        None => stem.to_string(),
    }
}

/// Resolve `id` → first ADR file whose filename id matches. Accepts both
/// `ADR-027` and `ADR-PORTAL-PERSONAS-001`.
pub fn resolve_adr_file(dir: &std::path::Path, id: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !name.ends_with(".md") {
            continue;
        }
        if parse_id_from_filename(name) == id {
            return Some(path);
        }
    }
    None
}

fn extract_title(markdown: &str) -> Option<String> {
    for line in markdown.lines() {
        let l = line.trim_start();
        if let Some(rest) = l.strip_prefix("# ") {
            return Some(rest.trim().to_string());
        }
    }
    None
}

fn classify(id: &str, title: &str) -> String {
    let lower = format!("{} {}", id.to_ascii_lowercase(), title.to_ascii_lowercase());
    let pick = |needles: &[&str]| -> bool { needles.iter().any(|n| lower.contains(n)) };
    if pick(&["portal", "persona", "auth", "keycloak", "okta", "sso", "rbac", "abac"]) {
        return "Identity & Portal".into();
    }
    if pick(&["mesh", "cilium", "istio", "kong", "gateway", "network", "dns", "cdn"]) {
        return "Networking".into();
    }
    if pick(&["secrets", "vault", "openbao", "policy", "compliance", "tetragon", "trivy", "scan"]) {
        return "Security".into();
    }
    if pick(&[
        "kafka", "stream", "postgres", "rdbms", "minio", "iceberg", "data", "valkey",
        "qdrant", "search",
    ]) {
        return "Data".into();
    }
    if pick(&["llm", "ollama", "litellm", "ai", "langfuse"]) {
        return "AI / LLM".into();
    }
    if pick(&["argo", "gitops", "ci", "pipeline", "deploy", "rollout"]) {
        return "GitOps & Delivery".into();
    }
    if pick(&["cluster", "k8s", "kubernetes", "scheduler", "etcd", "talos", "kamaji"]) {
        return "Kubernetes Core".into();
    }
    if pick(&["observability", "prometheus", "grafana", "loki", "tempo", "telemetry"]) {
        return "Observability".into();
    }
    if pick(&["chaos", "backup", "uptime", "incident"]) {
        return "Reliability".into();
    }
    "General".into()
}

fn render_markdown(md: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_FOOTNOTES);
    let parser = Parser::new_ext(md, opts);
    let mut buf = String::with_capacity(md.len() * 2);
    pd_html::push_html(&mut buf, parser);
    buf
}

pub fn router() -> Router {
    Router::new()
        .route("/adr", get(page))
        .route("/api/adr", get(api_list))
        .route("/api/adr/{id}", get(api_get))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal::WORKSPACE_ROOT_TEST_GUARD as ENV_GUARD;
    use axum::body::to_bytes;
    use axum::http::Request;
    use cave_auth::jwt_middleware::JwtClaims;
    use http_body_util::BodyExt;
    use std::fs;
    use tower::ServiceExt;

    /// Test helper: build the ADR router with a platform_admin
    /// `JwtClaims` extension pre-injected. Mirrors what the JWT
    /// middleware does for an authed request without spinning up the
    /// full middleware stack.
    fn router_as_platform() -> Router {
        let claims = JwtClaims {
            sub: "admin@platform.cave".into(),
            email: "admin@platform.cave".into(),
            roles: vec!["platform_admin".into()],
            exp: 4102444800,
        };
        router().layer(axum::Extension(claims))
    }

    fn tmpdir(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "cave-adr-{}-{}-{}",
            name,
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn seed_adrs(dir: &std::path::Path) {
        fs::write(
            dir.join("ADR-001_Hetzner_Cloud.md"),
            "# Hetzner Cloud as Sovereign Provider\n\nBody.\n",
        )
        .unwrap();
        fs::write(
            dir.join("ADR-027_Kong.md"),
            "# Kong API Gateway\n\n* baseline.\n",
        )
        .unwrap();
        fs::write(
            dir.join("ADR-PORTAL-PERSONAS-001.md"),
            "# Portal Personas\n\nadmin@platform / admin@tenant1.\n",
        )
        .unwrap();
        fs::write(dir.join("not-adr.md"), "# Skip me\n").unwrap();
    }

    #[test]
    fn parses_numeric_and_qualified_ids() {
        assert_eq!(parse_id_from_filename("ADR-027_Kong.md"), "ADR-027");
        assert_eq!(
            parse_id_from_filename("ADR-PORTAL-PERSONAS-001.md"),
            "ADR-PORTAL-PERSONAS-001"
        );
        assert_eq!(parse_id_from_filename("ADR-MULTI-TENANT-001.md"), "ADR-MULTI-TENANT-001");
    }

    #[test]
    fn extracts_title_from_h1() {
        let md = "Some preamble\n\n# Real Title\n\nBody";
        assert_eq!(extract_title(md).as_deref(), Some("Real Title"));
    }

    #[test]
    fn classifier_buckets_known_topics() {
        assert_eq!(classify("ADR-027", "Kong API Gateway"), "Networking");
        assert_eq!(classify("ADR-006", "Keycloak Identity"), "Identity & Portal");
        assert_eq!(
            classify("ADR-021", "Strimzi Kafka Operator"),
            "Data"
        );
        assert_eq!(classify("ADR-099", "random unrelated thing"), "General");
    }

    #[test]
    fn scan_finds_adrs_and_skips_others() {
        let d = tmpdir("scan");
        seed_adrs(&d);
        let mut s = scan_adrs(&d);
        s.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(s.len(), 3);
        let ids: Vec<&str> = s.iter().map(|x| x.id.as_str()).collect();
        assert!(ids.contains(&"ADR-001"));
        assert!(ids.contains(&"ADR-027"));
        assert!(ids.contains(&"ADR-PORTAL-PERSONAS-001"));
    }

    #[test]
    fn resolve_finds_qualified_id() {
        let d = tmpdir("resolve");
        seed_adrs(&d);
        let path = resolve_adr_file(&d, "ADR-PORTAL-PERSONAS-001").unwrap();
        assert!(path.to_string_lossy().contains("ADR-PORTAL-PERSONAS-001"));
    }

    #[tokio::test]
    async fn page_handler_returns_permanent_redirect_to_admin_adr() {
        // 2026-05-14 consolidation: /adr is permanently redirected to
        // /admin/adr (canonical admin shell page). No persona gate at
        // this layer — the target page does its own RBAC; the
        // redirect must work even for anonymous callers so bookmarks
        // transparently update.
        let app = router(); // no JWT extension layer
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/adr")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::PERMANENT_REDIRECT,
            "expected 308 Permanent Redirect (axum::Redirect::permanent)"
        );
        let location = resp
            .headers()
            .get("location")
            .expect("redirect carries Location header")
            .to_str()
            .unwrap();
        assert_eq!(location, "/admin/adr");
    }

    #[tokio::test]
    async fn list_endpoint_returns_seeded_adrs() {
        let _g = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
        let d = tmpdir("list");
        seed_adrs(&d);
        // SAFETY: guarded by ENV_GUARD above — only one test at a
        // time writes CAVE_WORKSPACE_ROOT.
        unsafe { std::env::set_var("CAVE_WORKSPACE_ROOT", d.parent().unwrap()); }
        // adr_dir() uses workspace_root()/docs/adr — recreate that layout
        let adr_actual = d.parent().unwrap().join("docs").join("adr");
        fs::create_dir_all(&adr_actual).unwrap();
        for f in fs::read_dir(&d).unwrap().flatten() {
            let dest = adr_actual.join(f.file_name());
            fs::copy(f.path(), dest).unwrap();
        }
        let app = router_as_platform();
        let resp = app
            .oneshot(Request::builder().uri("/api/adr").body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(v["total"].as_u64().unwrap() >= 3);
        let _ = fs::remove_dir_all(&adr_actual);
    }

    #[tokio::test]
    async fn get_endpoint_renders_markdown() {
        let _g = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
        let d = tmpdir("get");
        let adr_actual = d.join("docs").join("adr");
        fs::create_dir_all(&adr_actual).unwrap();
        seed_adrs(&adr_actual);
        // SAFETY: guarded by ENV_GUARD.
        unsafe { std::env::set_var("CAVE_WORKSPACE_ROOT", &d); }
        let app = router_as_platform();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/adr/ADR-027")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["id"], "ADR-027");
        assert!(v["html"].as_str().unwrap().contains("<h1>Kong"));
        assert_eq!(v["title"], "Kong API Gateway");
    }

    #[tokio::test]
    async fn get_endpoint_404_for_unknown() {
        let _g = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
        let d = tmpdir("get404");
        let adr_actual = d.join("docs").join("adr");
        fs::create_dir_all(&adr_actual).unwrap();
        // SAFETY: guarded by ENV_GUARD.
        unsafe { std::env::set_var("CAVE_WORKSPACE_ROOT", &d); }
        let app = router_as_platform();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/adr/ADR-9999")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
