// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/resources/v1/{ProjectResource,ComponentResource,BomResource,VulnerabilityResource,PolicyResource}.java
//! Axum HTTP routes — Dependency-Track REST API v1 parity surface.

use crate::State;
use crate::components::{ComponentRecord, Project};
use crate::models::VulnIntel;
use crate::policy::Policy;
use crate::sbom;
use axum::{
    Json, Router,
    extract::{Path, Query, State as AxumState},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router(state: Arc<State>) -> Router {
    Router::new()
        .route("/api/sbom/health", get(health))
        // Projects.
        .route("/api/v1/project", get(list_projects).post(create_project))
        .route("/api/v1/project/{uuid}", get(get_project))
        // Components.
        .route(
            "/api/v1/component",
            get(list_components).post(create_component),
        )
        .route("/api/v1/component/{uuid}", get(get_component))
        // BOM upload.
        .route("/api/v1/bom", post(upload_bom))
        // Vulnerabilities.
        .route("/api/v1/vulnerability", get(list_vulnerabilities))
        .route("/api/v1/vulnerability/{id}", get(get_vulnerability))
        .route(
            "/api/v1/vulnerability/{id}/analysis",
            post(set_analysis_state),
        )
        // Audit workflow (AnalysisResource parity).
        .route("/api/v1/analysis", post(record_analysis))
        // Policies.
        .route("/api/v1/policy", get(list_policies).post(create_policy))
        // Metrics.
        .route("/api/v1/metrics/portfolio", get(portfolio_metrics))
        // Cross-entity search (SearchResource parity).
        .route("/api/v1/search", get(keyword_search))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-sbom",
        "status": "ok",
        "upstream": "DependencyTrack",
        "version": crate::UPSTREAM_VERSION,
        "sha": crate::UPSTREAM_SHA
    }))
}

#[derive(Debug, Default, Deserialize)]
pub struct PaginationParams {
    pub page: Option<usize>,
    pub page_size: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Paginated<T> {
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
    pub items: Vec<T>,
}

fn paginate<T: Clone>(items: &[T], q: &PaginationParams) -> Paginated<T> {
    let page = q.page.unwrap_or(1).max(1);
    let page_size = q.page_size.unwrap_or(50).clamp(1, 1000);
    let start = (page - 1) * page_size;
    let end = (start + page_size).min(items.len());
    let slice = if start <= end && start < items.len() {
        items[start..end].to_vec()
    } else {
        Vec::new()
    };
    Paginated {
        total: items.len(),
        page,
        page_size,
        items: slice,
    }
}

// ── Project endpoints ───────────────────────────────────────────────────────

async fn list_projects(
    AxumState(state): AxumState<Arc<State>>,
    Query(q): Query<PaginationParams>,
) -> Json<Paginated<Project>> {
    let items = state.projects.read().unwrap().clone();
    Json(paginate(&items, &q))
}

#[derive(Debug, Deserialize)]
pub struct CreateProject {
    pub name: String,
    pub version: Option<String>,
}

async fn create_project(
    AxumState(state): AxumState<Arc<State>>,
    Json(body): Json<CreateProject>,
) -> (StatusCode, Json<Project>) {
    let p = Project::new(body.name, body.version);
    state.projects.write().unwrap().push(p.clone());
    (StatusCode::CREATED, Json(p))
}

async fn get_project(
    AxumState(state): AxumState<Arc<State>>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<Project>, StatusCode> {
    state
        .projects
        .read()
        .unwrap()
        .iter()
        .find(|p| p.uuid == uuid)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

// ── Component endpoints ─────────────────────────────────────────────────────

async fn list_components(
    AxumState(state): AxumState<Arc<State>>,
    Query(q): Query<PaginationParams>,
) -> Json<Paginated<ComponentRecord>> {
    let items = state.components.read().unwrap().clone();
    Json(paginate(&items, &q))
}

#[derive(Debug, Deserialize)]
pub struct CreateComponent {
    pub project_uuid: Uuid,
    pub name: String,
    pub version: String,
    pub purl: Option<String>,
    pub license: Option<String>,
}

async fn create_component(
    AxumState(state): AxumState<Arc<State>>,
    Json(body): Json<CreateComponent>,
) -> (StatusCode, Json<ComponentRecord>) {
    let mut c = ComponentRecord::new(body.project_uuid, body.name, body.version);
    c.purl = body.purl;
    c.license = body.license;
    state.components.write().unwrap().push(c.clone());
    (StatusCode::CREATED, Json(c))
}

async fn get_component(
    AxumState(state): AxumState<Arc<State>>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<ComponentRecord>, StatusCode> {
    state
        .components
        .read()
        .unwrap()
        .iter()
        .find(|c| c.uuid == uuid)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

// ── BOM upload ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct BomUpload {
    pub project_uuid: Option<Uuid>,
    /// Raw BOM content. Format is auto-detected.
    pub bom_b64: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BomUploadResult {
    pub project_uuid: Uuid,
    pub format: String,
    pub components_added: usize,
}

async fn upload_bom(
    AxumState(state): AxumState<Arc<State>>,
    Json(body): Json<BomUpload>,
) -> Result<Json<BomUploadResult>, (StatusCode, String)> {
    use base64::Engine;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(body.bom_b64.as_bytes())
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("bad base64: {}", e)))?;
    let fmt = sbom::detect_format(&raw).ok_or((
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "unknown BOM format".into(),
    ))?;
    let parsed = match fmt {
        sbom::BomFormat::CycloneDxJson => sbom::cyclonedx::parse_json(&raw)
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?,
        sbom::BomFormat::CycloneDxXml => sbom::cyclonedx::parse_xml(&raw)
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?,
        sbom::BomFormat::SpdxJson => {
            sbom::spdx::parse_json(&raw).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
        }
        sbom::BomFormat::SpdxTagValue => sbom::spdx::parse_tag_value(&raw)
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?,
    };
    let project_uuid = body.project_uuid.unwrap_or_else(|| {
        let p = Project::new(
            parsed
                .project_name
                .clone()
                .unwrap_or_else(|| "imported".into()),
            parsed.project_version.clone(),
        );
        let uuid = p.uuid;
        state.projects.write().unwrap().push(p);
        uuid
    });
    let mut added = 0;
    let mut comps = state.components.write().unwrap();
    for c in &parsed.components {
        let mut rec = ComponentRecord::new(project_uuid, c.name.clone(), c.version.clone());
        rec.purl = c.purl.clone();
        rec.license = c.license.clone();
        comps.push(rec);
        added += 1;
    }
    Ok(Json(BomUploadResult {
        project_uuid,
        format: format!("{:?}", parsed.format_detected),
        components_added: added,
    }))
}

// ── Vulnerability endpoints ─────────────────────────────────────────────────

async fn list_vulnerabilities(
    AxumState(state): AxumState<Arc<State>>,
    Query(q): Query<PaginationParams>,
) -> Json<Paginated<VulnIntel>> {
    let items = state.vulnerabilities.read().unwrap().clone();
    Json(paginate(&items, &q))
}

async fn get_vulnerability(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<String>,
) -> Result<Json<VulnIntel>, StatusCode> {
    state
        .vulnerabilities
        .read()
        .unwrap()
        .iter()
        .find(|v| v.vuln_id == id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

#[derive(Debug, Deserialize)]
pub struct AnalysisRequest {
    pub state: crate::models::AnalysisState,
}

async fn set_analysis_state(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<String>,
    Json(body): Json<AnalysisRequest>,
) -> Result<Json<VulnIntel>, StatusCode> {
    let mut vulns = state.vulnerabilities.write().unwrap();
    if let Some(v) = vulns.iter_mut().find(|v| v.vuln_id == id) {
        v.state = body.state;
        Ok(Json(v.clone()))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

/// Body for `PUT /api/v1/analysis` — mirrors `AnalysisResource.updateAnalysis`.
#[derive(Debug, Deserialize)]
pub struct AuditRequest {
    pub component: Uuid,
    pub vulnerability: String,
    #[serde(default)]
    pub analysis_state: Option<crate::models::AnalysisState>,
    #[serde(default)]
    pub analysis_justification: Option<crate::audit::AnalysisJustification>,
    #[serde(default)]
    pub analysis_response: Option<crate::audit::AnalysisResponse>,
    #[serde(default)]
    pub analysis_details: Option<String>,
    #[serde(default)]
    pub suppressed: Option<bool>,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub commenter: Option<String>,
}

async fn record_analysis(
    AxumState(state): AxumState<Arc<State>>,
    Json(body): Json<AuditRequest>,
) -> Result<Json<crate::audit::Analysis>, StatusCode> {
    if body.vulnerability.trim().is_empty() {
        // vulnerability is @NotNull upstream.
        return Err(StatusCode::BAD_REQUEST);
    }
    let mut store = state.analyses.write().unwrap();
    let analysis = store.record(crate::audit::AnalysisRequest {
        component_uuid: body.component,
        vulnerability: body.vulnerability,
        analysis_state: body.analysis_state,
        analysis_justification: body.analysis_justification,
        analysis_response: body.analysis_response,
        analysis_details: body.analysis_details,
        suppressed: body.suppressed,
        comment: body.comment,
        commenter: body.commenter,
    });
    Ok(Json(analysis))
}

// ── Policy endpoints ────────────────────────────────────────────────────────

async fn list_policies(AxumState(state): AxumState<Arc<State>>) -> Json<Vec<Policy>> {
    Json(state.policies.read().unwrap().clone())
}

async fn create_policy(
    AxumState(state): AxumState<Arc<State>>,
    Json(body): Json<Policy>,
) -> (StatusCode, Json<Policy>) {
    state.policies.write().unwrap().push(body.clone());
    (StatusCode::CREATED, Json(body))
}

// ── Portfolio metrics ───────────────────────────────────────────────────────

async fn portfolio_metrics(
    AxumState(state): AxumState<Arc<State>>,
) -> Json<crate::portfolio::PortfolioSnapshot> {
    let projects = state.projects.read().unwrap();
    let project_uuids: Vec<Uuid> = projects.iter().map(|p| p.uuid).collect();
    let comps = state.components.read().unwrap();
    let vulns = state.vulnerabilities.read().unwrap();
    Json(crate::portfolio::PortfolioSnapshot::take(
        &project_uuids,
        &comps,
        &vulns,
        chrono::Utc::now(),
    ))
}

// ── Cross-entity search ─────────────────────────────────────────────────────

#[derive(Deserialize)]
struct SearchQuery {
    q: Option<String>,
}

/// `GET /api/v1/search?q=<query>` — mirrors DependencyTrack SearchResource.
///
/// Returns a JSON array of [`crate::search::SearchResult`] spanning
/// projects, components, and vulnerabilities.
async fn keyword_search(
    AxumState(state): AxumState<Arc<State>>,
    Query(params): Query<SearchQuery>,
) -> (StatusCode, Json<Vec<crate::search::SearchResult>>) {
    let q = params.q.as_deref().unwrap_or("").trim().to_string();
    if q.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(vec![]));
    }
    let projects = state.projects.read().unwrap();
    let comps = state.components.read().unwrap();
    let vulns = state.vulnerabilities.read().unwrap();
    let results = crate::search::search_all(&q, &projects, &comps, &vulns);
    (StatusCode::OK, Json(results))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use tower::ServiceExt;

    fn router() -> Router {
        create_router(Arc::new(State::default()))
    }

    #[tokio::test]
    async fn health_returns_ok_envelope() {
        let r = router();
        let resp = r
            .oneshot(
                Request::builder()
                    .uri("/api/sbom/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let body = to_bytes(resp.into_body(), 8192).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["module"], "cave-sbom");
    }

    #[tokio::test]
    async fn create_project_then_get() {
        let r = router();
        // POST
        let body = serde_json::json!({"name":"x","version":"1.0"});
        let post = r
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/project")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(post.status(), 201);
        let post_body = to_bytes(post.into_body(), 8192).await.unwrap();
        let p: Project = serde_json::from_slice(&post_body).unwrap();
        // GET list — uses different state since we cloned; use shared state.
        // Instead exercise the same router for both.
        let state = Arc::new(State::default());
        let r2 = create_router(state.clone());
        let post2 = r2
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/project")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(post2.status(), 201);
        let list = r2
            .oneshot(
                Request::builder()
                    .uri("/api/v1/project")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let lb = to_bytes(list.into_body(), 8192).await.unwrap();
        let page: Paginated<Project> = serde_json::from_slice(&lb).unwrap();
        assert_eq!(page.total, 1);
        let _ = p;
    }

    #[tokio::test]
    async fn list_paginated_respects_page_size() {
        let state = Arc::new(State::default());
        for i in 0..150 {
            state
                .projects
                .write()
                .unwrap()
                .push(Project::new(format!("p{}", i), None));
        }
        let r = create_router(state);
        let resp = r
            .oneshot(
                Request::builder()
                    .uri("/api/v1/project?page=2&page_size=50")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(resp.into_body(), 65536).await.unwrap();
        let page: Paginated<Project> = serde_json::from_slice(&body).unwrap();
        assert_eq!(page.total, 150);
        assert_eq!(page.page, 2);
        assert_eq!(page.page_size, 50);
        assert_eq!(page.items.len(), 50);
        assert_eq!(page.items[0].name, "p50");
    }

    #[tokio::test]
    async fn bom_upload_creates_components() {
        use base64::Engine;
        let bom = br#"{"bomFormat":"CycloneDX","specVersion":"1.5",
            "components":[{"type":"library","bom-ref":"x","name":"lodash","version":"4.17.21"}]}"#;
        let b64 = base64::engine::general_purpose::STANDARD.encode(bom);
        let body = serde_json::json!({ "bom_b64": b64 });
        let state = Arc::new(State::default());
        let r = create_router(state.clone());
        let resp = r
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/bom")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let resp_body = to_bytes(resp.into_body(), 8192).await.unwrap();
        let res: BomUploadResult = serde_json::from_slice(&resp_body).unwrap();
        assert_eq!(res.components_added, 1);
        assert_eq!(state.components.read().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn get_unknown_project_is_404() {
        let r = router();
        let resp = r
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/project/{}", Uuid::new_v4()))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);
    }
}
