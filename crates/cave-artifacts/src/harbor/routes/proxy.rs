// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Pull-through proxy routes — HTTP surface for every supported ecosystem.
//!
//! Wire shape:
//! ```text
//!   /api/registry/<ecosystem>/simple/<package>/     PyPI Simple index
//!   /api/registry/<ecosystem>/blob/<url-path...>    Blob (tarball / wheel / jar / tgz / zip)
//!   /api/registry/<ecosystem>/-/<rest...>           Ecosystem-specific passthrough
//!   /api/registry/verdict/<scan_id>                 Look up a past scan verdict
//!   /api/registry/proxy/status                      Snapshot of the proxy config
//! ```
//!
//! The Docker v2 routes live in `routes::v2` and are untouched — OCI pull
//! falls through the OCI adapter rather than this module.

use crate::harbor::pipeline::{ScanPipelineOutcome, VerdictDecision};
use crate::harbor::proxy::{Ecosystem, FetchedArtifact, ProxyError, ProxyMode};
use crate::harbor::RegistryState;
use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde_json::json;
use std::sync::Arc;
use tracing::warn;

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router(state: Arc<RegistryState>) -> Router {
    Router::new()
        // Health & status
        .route("/api/registry/health", get(health))
        .route("/api/registry/proxy/status", get(proxy_status))
        // PyPI Simple index + blobs
        .route("/api/registry/pypi/simple/", get(pypi_simple_root))
        .route("/api/registry/pypi/simple/{package}/", get(pypi_simple_project))
        .route("/api/registry/pypi/blob/{*path}", get(pypi_blob))
        // npm metadata + tarballs
        .route("/api/registry/npm/{package}", get(npm_metadata))
        .route("/api/registry/npm/{package}/-/{tarball}", get(npm_tarball))
        // Generic blob endpoints (Maven / RubyGems / Cargo / Go / NuGet / Composer)
        .route("/api/registry/{ecosystem}/blob/{*path}", get(generic_blob))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Health & status
// ---------------------------------------------------------------------------

async fn health() -> Json<serde_json::Value> {
    Json(json!({
        "module": "cave-registry",
        "status": "ok",
        "upstream": "Harbor + Pulp + Artifactory (pull-through)"
    }))
}

async fn proxy_status(State(state): State<Arc<RegistryState>>) -> Json<serde_json::Value> {
    let cfg = state.proxy.config();
    let upstreams: Vec<_> = cfg
        .upstreams
        .values()
        .map(|u| {
            json!({
                "ecosystem": u.ecosystem.as_str(),
                "base_url": u.base_url,
                "disabled": u.disabled,
            })
        })
        .collect();
    Json(json!({
        "mode": cfg.mode,
        "upstream_count": upstreams.len(),
        "upstreams": upstreams,
        "blocklist_size": cfg.blocklist.len(),
        "enforce": state.pipeline.config().enforce,
    }))
}

// ---------------------------------------------------------------------------
// PyPI Simple
// ---------------------------------------------------------------------------

async fn pypi_simple_root(State(state): State<Arc<RegistryState>>) -> Response {
    // A conservative, empty Simple root so pip doesn't choke.
    match state.proxy.fetch_index(Ecosystem::PyPI, "").await {
        Ok((ct, bytes)) => {
            let rewritten = state
                .proxy
                .rewrite_urls(Ecosystem::PyPI, std::str::from_utf8(&bytes).unwrap_or(""), "")
                .into_bytes();
            (StatusCode::OK, [(header::CONTENT_TYPE, ct)], rewritten).into_response()
        }
        Err(e) => upstream_to_status(&e),
    }
}

async fn pypi_simple_project(
    Path(package): Path<String>,
    State(state): State<Arc<RegistryState>>,
    headers: HeaderMap,
) -> Response {
    let path = format!("{}/", package);
    match state.proxy.fetch_index(Ecosystem::PyPI, &path).await {
        Ok((ct, bytes)) => {
            let host = headers
                .get(header::HOST)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("cave-registry.local");
            let body = std::str::from_utf8(&bytes)
                .map(|s| s.to_string())
                .unwrap_or_default();
            let rewritten = state.proxy.rewrite_urls(Ecosystem::PyPI, &body, host);
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, ct)],
                rewritten,
            )
                .into_response()
        }
        Err(e) => upstream_to_status(&e),
    }
}

async fn pypi_blob(
    Path(path): Path<String>,
    State(state): State<Arc<RegistryState>>,
) -> Response {
    // PyPI blob URL shape (after rewrite): `<package>/<wheel-or-sdist>`.
    let (package, _) = split_first_segment(&path);
    serve_blob(Ecosystem::PyPI, &package, None, &path, state).await
}

// ---------------------------------------------------------------------------
// npm
// ---------------------------------------------------------------------------

async fn npm_metadata(
    Path(package): Path<String>,
    State(state): State<Arc<RegistryState>>,
    headers: HeaderMap,
) -> Response {
    match state.proxy.fetch_index(Ecosystem::Npm, &package).await {
        Ok((ct, bytes)) => {
            let host = headers
                .get(header::HOST)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("cave-registry.local");
            let body = std::str::from_utf8(&bytes)
                .map(|s| s.to_string())
                .unwrap_or_default();
            let rewritten = state.proxy.rewrite_urls(Ecosystem::Npm, &body, host);
            (StatusCode::OK, [(header::CONTENT_TYPE, ct)], rewritten).into_response()
        }
        Err(e) => upstream_to_status(&e),
    }
}

async fn npm_tarball(
    Path((package, tarball)): Path<(String, String)>,
    State(state): State<Arc<RegistryState>>,
) -> Response {
    let path = format!("{}/-/{}", package, tarball);
    serve_blob(Ecosystem::Npm, &package, None, &path, state).await
}

// ---------------------------------------------------------------------------
// Generic blob (Maven / RubyGems / Cargo / Go / NuGet / Composer)
// ---------------------------------------------------------------------------

async fn generic_blob(
    Path((ecosystem, path)): Path<(String, String)>,
    State(state): State<Arc<RegistryState>>,
) -> Response {
    let Some(eco) = Ecosystem::from_slug(&ecosystem) else {
        return (StatusCode::NOT_FOUND, "unknown ecosystem").into_response();
    };
    let (pkg, _) = split_first_segment(&path);
    serve_blob(eco, &pkg, None, &path, state).await
}

// ---------------------------------------------------------------------------
// Core serve flow — cache lookup → upstream fetch → scan → respond
// ---------------------------------------------------------------------------

async fn serve_blob(
    ecosystem: Ecosystem,
    package: &str,
    version: Option<&str>,
    path: &str,
    state: Arc<RegistryState>,
) -> Response {
    // Cache key: (ecosystem, package, path) — mapped onto the content-
    // addressable storage by sha256 after the first successful fetch. For
    // the first cut we treat "first fetch wins": we keep a namespaced lookup
    // via manifests index. If we have a cached copy, serve it. Otherwise
    // fetch + scan + cache.
    let cache_key = format!("{}:{}:{}", ecosystem.as_str(), package, path);
    if let Some(bytes) = state.storage.get_blob_by_alias(&cache_key).await {
        return blob_response(&cache_key, ecosystem, bytes);
    }

    // Cache miss — try upstream.
    let art = match state.proxy.fetch(ecosystem, package, version, path).await {
        Ok(a) => a,
        Err(ProxyError::ProxyOff) => {
            return (StatusCode::NOT_FOUND, "not cached and proxy is off").into_response();
        }
        Err(ProxyError::Blocked { .. }) => {
            return build_451_response(
                ecosystem,
                package,
                version,
                &ScanPipelineOutcome {
                    id: uuid::Uuid::new_v4(),
                    verdict: VerdictDecision::Fail,
                    reasons: vec!["package on static blocklist".to_string()],
                    findings: vec![],
                    ecosystem,
                    name: package.to_string(),
                    version: version.map(str::to_string),
                    sha256: String::new(),
                    scanner_ms: 0,
                    scanned_at: chrono::Utc::now(),
                    blocked: true,
                },
            );
        }
        Err(e) => {
            warn!(target: "cave_registry::routes::proxy", ?e, "upstream fetch failed");
            return upstream_to_status(&e);
        }
    };

    // Scan pipeline
    let outcome = state.pipeline.evaluate(&art).await;
    if outcome.blocked {
        return build_451_response(ecosystem, package, version, &outcome);
    }

    // Passed — cache and serve.
    state
        .storage
        .put_blob_with_alias(cache_key.clone(), art.bytes.clone())
        .await;

    blob_response(&cache_key, ecosystem, art.bytes)
}

fn blob_response(cache_key: &str, ecosystem: Ecosystem, bytes: bytes::Bytes) -> Response {
    let ct = default_content_type(ecosystem);
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(ct),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    headers.insert(
        HeaderName::from_static("x-cave-cache-key"),
        HeaderValue::from_str(cache_key).unwrap_or(HeaderValue::from_static("-")),
    );
    (StatusCode::OK, headers, Body::from(bytes)).into_response()
}

fn default_content_type(ecosystem: Ecosystem) -> &'static str {
    match ecosystem {
        Ecosystem::PyPI => "application/octet-stream",
        Ecosystem::Npm => "application/octet-stream",
        Ecosystem::Maven => "application/java-archive",
        Ecosystem::RubyGems => "application/octet-stream",
        Ecosystem::Cargo => "application/gzip",
        Ecosystem::Go => "application/zip",
        Ecosystem::NuGet => "application/zip",
        Ecosystem::Composer => "application/zip",
        Ecosystem::Oci => "application/vnd.oci.image.manifest.v1+json",
    }
}

// ---------------------------------------------------------------------------
// 451 body — ADR-133 §3.4
// ---------------------------------------------------------------------------

fn build_451_response(
    ecosystem: Ecosystem,
    package: &str,
    version: Option<&str>,
    outcome: &ScanPipelineOutcome,
) -> Response {
    let reasons_payload: Vec<_> = outcome
        .findings
        .iter()
        .map(|f| {
            json!({
                "rule": f.rule,
                "scanner": f.scanner,
                "severity": f.severity,
                "cves": f.cves,
                "summary": f.summary,
            })
        })
        .collect();

    let body = json!({
        "error": "policy_violation",
        "error_code": "cave.registry.blocked",
        "ecosystem": ecosystem.as_str(),
        "package": package,
        "version": version,
        "verdict": outcome.verdict,
        "reasons": if reasons_payload.is_empty() { json!(outcome.reasons) } else { json!(reasons_payload) },
        "finding_ids": outcome.findings.iter().map(|_| outcome.id).collect::<Vec<_>>(),
        "finding_url": format!("/portal/vulns/findings/{}", outcome.id),
        "ledger_entry": format!(
            "sl://artifacts/{}/{}/{}/blocked",
            outcome.scanned_at.format("%Y-%m-%d"),
            ecosystem.as_str(),
            package
        ),
        "appeal": {
            "contact": "platform-security@caveplatform.dev",
            "override_policy": "platform-security can grant a time-boxed allowlist entry via cave-policy",
            "docs": "https://portal.caveplatform.dev/docs/registry/blocked-packages"
        }
    });

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    headers.insert(
        HeaderName::from_static("x-cave-registry-verdict"),
        HeaderValue::from_static("fail"),
    );

    (
        StatusCode::from_u16(451).unwrap_or(StatusCode::FORBIDDEN),
        headers,
        Json(body),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

fn upstream_to_status(err: &ProxyError) -> Response {
    match err {
        ProxyError::ProxyOff => (StatusCode::NOT_FOUND, "proxy off").into_response(),
        ProxyError::NoUpstream(_) => (StatusCode::NOT_FOUND, "no upstream configured").into_response(),
        ProxyError::UpstreamDisabled(_) => (StatusCode::SERVICE_UNAVAILABLE, "upstream disabled").into_response(),
        ProxyError::Blocked { .. } => (StatusCode::from_u16(451).unwrap_or(StatusCode::FORBIDDEN), "blocked").into_response(),
        ProxyError::UpstreamStatus { status, .. } => {
            (StatusCode::from_u16(*status).unwrap_or(StatusCode::BAD_GATEWAY), "upstream error").into_response()
        }
        ProxyError::UpstreamFetch { .. } | ProxyError::ClientBuild(_) => {
            (StatusCode::BAD_GATEWAY, "upstream fetch failed").into_response()
        }
    }
}

fn split_first_segment(path: &str) -> (String, String) {
    let trimmed = path.trim_start_matches('/');
    match trimmed.split_once('/') {
        Some((a, b)) => (a.to_string(), b.to_string()),
        None => (trimmed.to_string(), String::new()),
    }
}
