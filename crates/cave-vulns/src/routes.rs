// SPDX-License-Identifier: AGPL-3.0-or-later
//! HTTP routes for cave-vulns — DefectDojo API v2 parity.
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738 dojo/api_v2/views.py
//!         (`FindingViewSet`, `EngagementViewSet`, `ProductViewSet`,
//!          `ImportScanView`, `ReImportScanView`, `RiskAcceptanceViewSet`).
//!
//! Routes mounted:
//!   GET  /api/vulns/health
//!   GET  /api/vulns/findings                       — list findings
//!   GET  /api/vulns/findings/:id                   — finding detail
//!   POST /api/vulns/findings                       — create finding
//!   GET  /api/vulns/products                       — list products
//!   POST /api/vulns/products                       — create product
//!   GET  /api/vulns/engagements
//!   POST /api/vulns/engagements
//!   POST /api/vulns/import-scan                    — upload native scan output
//!   GET  /api/vulns/sla                            — SLA config + rollup
//!   GET  /api/vulns/risk-acceptances
//!   POST /api/vulns/risk-acceptances
//!   GET  /api/vulns/reports/executive              — JSON exec summary
//!   GET  /api/vulns/reports/executive.html         — HTML exec summary
//!   GET  /api/vulns/scan-types                     — registered parsers

use crate::dedup::{deduplicate_batch, DedupAlgorithm};
use crate::finding::Finding;
use crate::hierarchy::{Engagement, Product, ProductType};
use crate::parsers::find_parser;
use crate::reports::{executive_summary, to_html, to_json};
use crate::risk_accept::RiskAcceptance;
use crate::sla::SlaConfiguration;
use crate::State;
use axum::{
    extract::{Path, Query, State as AxState},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;

const FINDINGS_COL: &str = "vulns:findings";
const PRODUCTS_COL: &str = "vulns:products";
const PRODUCT_TYPES_COL: &str = "vulns:product_types";
const ENGAGEMENTS_COL: &str = "vulns:engagements";
const RISK_ACC_COL: &str = "vulns:risk_acceptances";

pub fn create_router(state: Arc<State>) -> Router {
    Router::new()
        .route("/api/vulns/health", get(health))
        .route("/api/vulns/scan-types", get(scan_types))
        .route("/api/vulns/findings", get(list_findings).post(create_finding))
        .route("/api/vulns/findings/{id}", get(get_finding))
        .route("/api/vulns/products", get(list_products).post(create_product))
        .route("/api/vulns/product-types", get(list_product_types).post(create_product_type))
        .route("/api/vulns/engagements", get(list_engagements).post(create_engagement))
        .route("/api/vulns/import-scan", post(import_scan))
        .route("/api/vulns/sla", get(sla_rollup))
        .route("/api/vulns/risk-acceptances", get(list_risk_acceptances).post(create_risk_acceptance))
        .route("/api/vulns/reports/executive", get(report_executive))
        .route("/api/vulns/reports/executive.html", get(report_executive_html))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-vulns",
        "status": "ok",
        "upstream": "DefectDojo",
        "upstream_version": "v2.58.2",
        "upstream_sha": "6eab8738",
    }))
}

#[derive(Debug, Deserialize)]
pub struct PageQuery {
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: Option<usize>,
}

async fn list_findings(
    AxState(state): AxState<Arc<State>>,
    Query(q): Query<PageQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    use cave_db::persistence::StorageExt;
    let all: Vec<Finding> = state.storage.list(FINDINGS_COL).await.map_err(ApiError::Storage)?;
    let total = all.len();
    let offset = q.offset.unwrap_or(0);
    let limit = q.limit.unwrap_or(100).min(1000);
    let slice = all.into_iter().skip(offset).take(limit).collect::<Vec<_>>();
    Ok(Json(serde_json::json!({
        "count": total,
        "next": (offset + limit < total).then(|| format!("/api/vulns/findings?offset={}&limit={}", offset + limit, limit)),
        "previous": (offset > 0).then(|| format!("/api/vulns/findings?offset={}&limit={}", offset.saturating_sub(limit), limit)),
        "results": slice,
    })))
}

async fn get_finding(
    AxState(state): AxState<Arc<State>>,
    Path(id): Path<String>,
) -> Result<Json<Finding>, ApiError> {
    use cave_db::persistence::StorageExt;
    let f: Option<Finding> = state.storage.get(FINDINGS_COL, &id).await.map_err(ApiError::Storage)?;
    f.map(Json).ok_or(ApiError::NotFound)
}

async fn create_finding(
    AxState(state): AxState<Arc<State>>,
    Json(f): Json<Finding>,
) -> Result<Json<Finding>, ApiError> {
    use cave_db::persistence::StorageExt;
    state.storage.put(FINDINGS_COL, &f.id.to_string(), &f).await.map_err(ApiError::Storage)?;
    Ok(Json(f))
}

async fn list_products(AxState(state): AxState<Arc<State>>) -> Result<Json<Vec<Product>>, ApiError> {
    use cave_db::persistence::StorageExt;
    state.storage.list(PRODUCTS_COL).await.map(Json).map_err(ApiError::Storage)
}

async fn create_product(
    AxState(state): AxState<Arc<State>>,
    Json(p): Json<Product>,
) -> Result<Json<Product>, ApiError> {
    use cave_db::persistence::StorageExt;
    state.storage.put(PRODUCTS_COL, &p.id.to_string(), &p).await.map_err(ApiError::Storage)?;
    Ok(Json(p))
}

async fn list_product_types(AxState(state): AxState<Arc<State>>) -> Result<Json<Vec<ProductType>>, ApiError> {
    use cave_db::persistence::StorageExt;
    state.storage.list(PRODUCT_TYPES_COL).await.map(Json).map_err(ApiError::Storage)
}

async fn create_product_type(
    AxState(state): AxState<Arc<State>>,
    Json(p): Json<ProductType>,
) -> Result<Json<ProductType>, ApiError> {
    use cave_db::persistence::StorageExt;
    state.storage.put(PRODUCT_TYPES_COL, &p.id.to_string(), &p).await.map_err(ApiError::Storage)?;
    Ok(Json(p))
}

async fn list_engagements(AxState(state): AxState<Arc<State>>) -> Result<Json<Vec<Engagement>>, ApiError> {
    use cave_db::persistence::StorageExt;
    state.storage.list(ENGAGEMENTS_COL).await.map(Json).map_err(ApiError::Storage)
}

async fn create_engagement(
    AxState(state): AxState<Arc<State>>,
    Json(e): Json<Engagement>,
) -> Result<Json<Engagement>, ApiError> {
    use cave_db::persistence::StorageExt;
    state.storage.put(ENGAGEMENTS_COL, &e.id.to_string(), &e).await.map_err(ApiError::Storage)?;
    Ok(Json(e))
}

#[derive(Debug, Deserialize)]
pub struct ImportScanRequest {
    pub scan_type: String,
    /// Raw scanner output (JSON/XML, parser-specific).
    pub content: String,
    /// Optional engagement to attach the findings to.
    #[serde(default)]
    pub engagement_id: Option<uuid::Uuid>,
    /// Dedup algorithm override (default: hash_code).
    #[serde(default)]
    pub dedup: Option<String>,
}

async fn import_scan(
    AxState(state): AxState<Arc<State>>,
    Json(req): Json<ImportScanRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    use cave_db::persistence::StorageExt;
    let parser = find_parser(&req.scan_type)
        .ok_or_else(|| ApiError::BadRequest(format!("unknown scan_type: {}", req.scan_type)))?;
    let mut findings = parser
        .parse(req.content.as_bytes())
        .map_err(|e| ApiError::BadRequest(format!("parse error: {e}")))?;
    let algo = req.dedup.as_deref().and_then(DedupAlgorithm::parse).unwrap_or(DedupAlgorithm::HashCode);
    findings = deduplicate_batch(findings, algo, Some(&req.scan_type));
    let n = findings.len();
    for f in &findings {
        let mut to_save = f.clone();
        to_save.found_by_scanner = Some(req.scan_type.clone());
        if let Some(eid) = req.engagement_id {
            // record the engagement test linkage so reports can group.
            to_save.test_id = Some(eid);
        }
        state.storage.put(FINDINGS_COL, &to_save.id.to_string(), &to_save)
            .await.map_err(ApiError::Storage)?;
    }
    Ok(Json(serde_json::json!({
        "scan_type": req.scan_type,
        "imported": n,
        "dedup_algorithm": algo_str(algo),
    })))
}

fn algo_str(a: DedupAlgorithm) -> &'static str {
    match a {
        DedupAlgorithm::Legacy => "legacy",
        DedupAlgorithm::HashCode => "hash_code",
        DedupAlgorithm::UniqueIdFromTool => "unique_id_from_tool",
        DedupAlgorithm::UniqueIdFromToolOrHashCode => "unique_id_from_tool_or_hash_code",
    }
}

async fn sla_rollup(AxState(state): AxState<Arc<State>>) -> Result<Json<serde_json::Value>, ApiError> {
    use cave_db::persistence::StorageExt;
    let findings: Vec<Finding> = state.storage.list(FINDINGS_COL).await.map_err(ApiError::Storage)?;
    let cfg = SlaConfiguration::default();
    let r = crate::sla::rollup(&cfg, &findings, chrono::Utc::now());
    Ok(Json(serde_json::json!({
        "config": cfg,
        "total": r.total,
        "breached": r.breached,
        "breaching_soon": r.breaching_soon,
        "by_severity": r.by_severity.iter().map(|(s, t, b)| {
            serde_json::json!({"severity": s.as_str(), "total": t, "breached": b})
        }).collect::<Vec<_>>(),
    })))
}

async fn list_risk_acceptances(AxState(state): AxState<Arc<State>>) -> Result<Json<Vec<RiskAcceptance>>, ApiError> {
    use cave_db::persistence::StorageExt;
    state.storage.list(RISK_ACC_COL).await.map(Json).map_err(ApiError::Storage)
}

async fn create_risk_acceptance(
    AxState(state): AxState<Arc<State>>,
    Json(ra): Json<RiskAcceptance>,
) -> Result<Json<RiskAcceptance>, ApiError> {
    use cave_db::persistence::StorageExt;
    state.storage.put(RISK_ACC_COL, &ra.id.to_string(), &ra).await.map_err(ApiError::Storage)?;
    Ok(Json(ra))
}

async fn report_executive(AxState(state): AxState<Arc<State>>) -> Result<Json<serde_json::Value>, ApiError> {
    use cave_db::persistence::StorageExt;
    let findings: Vec<Finding> = state.storage.list(FINDINGS_COL).await.map_err(ApiError::Storage)?;
    let cfg = SlaConfiguration::default();
    let s = executive_summary(None, None, &findings, &cfg);
    Ok(Json(serde_json::from_str(&to_json(&s)).expect("round-trip")))
}

async fn report_executive_html(AxState(state): AxState<Arc<State>>) -> Result<axum::response::Html<String>, ApiError> {
    use cave_db::persistence::StorageExt;
    let findings: Vec<Finding> = state.storage.list(FINDINGS_COL).await.map_err(ApiError::Storage)?;
    let cfg = SlaConfiguration::default();
    let s = executive_summary(None, None, &findings, &cfg);
    Ok(axum::response::Html(to_html(&s)))
}

async fn scan_types() -> Json<Vec<&'static str>> {
    Json(crate::parsers::registry().iter().map(|p| p.scan_type()).collect())
}

#[derive(Debug)]
pub enum ApiError {
    NotFound,
    BadRequest(String),
    Storage(cave_db::persistence::StorageError),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"not_found"}))).into_response(),
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"bad_request","message":m}))).into_response(),
            ApiError::Storage(e) => {
                tracing::error!("vulns storage error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error":"storage"}))).into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::util::ServiceExt;

    fn router() -> Router {
        create_router(Arc::new(State::default()))
    }

    #[tokio::test]
    async fn health_returns_200() {
        let resp = router().oneshot(Request::builder().uri("/api/vulns/health").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn scan_types_lists_all_seven_parsers() {
        let resp = router().oneshot(Request::builder().uri("/api/vulns/scan-types").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
        let v: Vec<String> = serde_json::from_slice(&body).unwrap();
        assert!(v.contains(&"Bandit Scan".to_string()));
        assert!(v.contains(&"SARIF".to_string()));
        assert_eq!(v.len(), 7);
    }

    #[tokio::test]
    async fn import_scan_persists_findings() {
        let r = router();
        let req = Request::post("/api/vulns/import-scan")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({
                "scan_type": "Bandit Scan",
                "content": r#"{"results":[{"test_name":"x","test_id":"B1","filename":"a.py","line_number":1,"issue_severity":"HIGH","issue_text":"y"}]}"#,
            }).to_string())).unwrap();
        let resp = r.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["imported"], 1);
        // Now list:
        let resp2 = r.oneshot(Request::builder().uri("/api/vulns/findings").body(Body::empty()).unwrap()).await.unwrap();
        let body2 = axum::body::to_bytes(resp2.into_body(), 65536).await.unwrap();
        let v2: serde_json::Value = serde_json::from_slice(&body2).unwrap();
        assert_eq!(v2["count"], 1);
    }

    #[tokio::test]
    async fn import_scan_rejects_unknown_scan_type() {
        let req = Request::post("/api/vulns/import-scan")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"scan_type":"NopeScan","content":"{}"}"#)).unwrap();
        let resp = router().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 400);
    }

    #[tokio::test]
    async fn findings_list_pagination_works() {
        let r = router();
        // seed 3 findings via Bandit
        let req = Request::post("/api/vulns/import-scan")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({
                "scan_type": "Bandit Scan",
                "content": r#"{"results":[
                    {"test_name":"a","test_id":"B1","filename":"a.py","line_number":1,"issue_severity":"HIGH","issue_text":"x"},
                    {"test_name":"b","test_id":"B2","filename":"b.py","line_number":2,"issue_severity":"LOW","issue_text":"y"},
                    {"test_name":"c","test_id":"B3","filename":"c.py","line_number":3,"issue_severity":"INFO","issue_text":"z"}
                ]}"#,
            }).to_string())).unwrap();
        r.clone().oneshot(req).await.unwrap();
        let resp = r.oneshot(Request::builder().uri("/api/vulns/findings?limit=2").body(Body::empty()).unwrap()).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["count"], 3);
        assert_eq!(v["results"].as_array().unwrap().len(), 2);
        assert!(v["next"].is_string());
    }

    #[tokio::test]
    async fn product_crud_works() {
        let r = router();
        let pt = ProductType::new("Web");
        let p = Product::new(pt.id, "App1");
        let req = Request::post("/api/vulns/products")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&p).unwrap())).unwrap();
        let resp = r.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);
        let resp2 = r.oneshot(Request::builder().uri("/api/vulns/products").body(Body::empty()).unwrap()).await.unwrap();
        let body = axum::body::to_bytes(resp2.into_body(), 65536).await.unwrap();
        let v: Vec<Product> = serde_json::from_slice(&body).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].name, "App1");
    }

    #[tokio::test]
    async fn sla_rollup_endpoint() {
        let resp = router().oneshot(Request::builder().uri("/api/vulns/sla").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["total"], 0);
        assert!(v["config"].is_object());
    }

    #[tokio::test]
    async fn report_executive_endpoint() {
        let resp = router().oneshot(Request::builder().uri("/api/vulns/reports/executive").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn report_executive_html_endpoint() {
        let resp = router().oneshot(Request::builder().uri("/api/vulns/reports/executive.html").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
        let s = String::from_utf8(body.to_vec()).unwrap();
        assert!(s.contains("<h1>"));
    }

    #[tokio::test]
    async fn finding_not_found_returns_404() {
        let resp = router().oneshot(Request::builder().uri("/api/vulns/findings/00000000-0000-0000-0000-000000000000").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), 404);
    }
}
