//! HTTP routes for cave-compliance.

use crate::models::*;
use crate::ComplianceState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

// ── Request types ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct RunCheckRequest {
    pub cluster_config: Option<serde_json::Value>,
    pub namespace_list: Option<Vec<String>>,
    pub pod_specs: Option<Vec<serde_json::Value>>,
    pub network_policies: Option<Vec<serde_json::Value>>,
}

#[derive(Deserialize)]
pub struct ScanRequest {
    pub framework_id: Uuid,
    pub context: Option<RunCheckRequest>,
}

#[derive(Deserialize)]
pub struct CreateFindingRequest {
    pub control_id: Uuid,
    pub control_ref: String,
    pub status: FindingStatus,
    pub target: String,
    pub details: String,
    pub remediation: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateEvidenceRequest {
    pub control_id: Uuid,
    pub finding_id: Option<Uuid>,
    pub evidence_type: EvidenceType,
    pub description: String,
    pub data: serde_json::Value,
    pub collected_by: String,
}

#[derive(Deserialize)]
pub struct CreateExceptionRequest {
    pub control_id: Uuid,
    pub control_ref: String,
    pub reason: String,
    pub approved_by: String,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
pub struct CreatePolicyMappingRequest {
    pub control_id: Uuid,
    pub control_ref: String,
    pub policy_engine: PolicyEngine,
    pub policy_name: String,
    pub policy_namespace: Option<String>,
    pub description: String,
}

#[derive(Deserialize)]
pub struct GenerateReportRequest {
    pub framework_id: Uuid,
}

#[derive(Deserialize, Default)]
pub struct FindingsQuery {
    pub control_ref: Option<String>,
    pub status: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct AuditQuery {
    pub resource_type: Option<String>,
    pub actor: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Deserialize)]
pub struct CreateCustomFrameworkRequest {
    pub name: String,
    pub version: String,
    pub description: String,
}

// ── Router ───────────────────────────────────────────────────────────────────

pub fn create_router(state: Arc<ComplianceState>) -> Router {
    Router::new()
        // Frameworks
        .route("/api/compliance/frameworks", get(list_frameworks).post(create_framework))
        .route("/api/compliance/frameworks/{id}", get(get_framework))
        .route("/api/compliance/frameworks/{id}/controls", get(list_controls))
        // Checks & Scans
        .route("/api/compliance/controls/{id}/check", post(run_control_check))
        .route("/api/compliance/scan", post(scan_framework))
        // Findings
        .route("/api/compliance/findings", get(list_findings).post(create_finding))
        .route("/api/compliance/findings/{id}", get(get_finding))
        // Evidence
        .route("/api/compliance/evidence", get(list_evidence).post(add_evidence))
        // Audit
        .route("/api/compliance/audit", get(get_audit_trail))
        // Exceptions
        .route("/api/compliance/exceptions", get(list_exceptions).post(create_exception))
        .route("/api/compliance/exceptions/{id}", delete(delete_exception))
        // Reports
        .route("/api/compliance/reports", get(list_reports))
        .route("/api/compliance/reports/generate", post(generate_report))
        .route("/api/compliance/reports/{id}", get(get_report))
        // Policy mappings
        .route("/api/compliance/policy-mappings", get(list_policy_mappings).post(create_policy_mapping))
        .route("/api/compliance/policy-mappings/suggest/{control_ref}", get(suggest_policy_mappings))
        // Health
        .route("/api/compliance/health", get(health))
        .with_state(state)
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn build_check_context(req: Option<RunCheckRequest>) -> crate::checks::CheckContext {
    match req {
        None => crate::checks::CheckContext::default(),
        Some(r) => crate::checks::CheckContext {
            cluster_config: r.cluster_config.unwrap_or_else(|| serde_json::json!({})),
            namespace_list: r.namespace_list.unwrap_or_else(|| vec!["default".to_string(), "kube-system".to_string()]),
            pod_specs: r.pod_specs.unwrap_or_default(),
            network_policies: r.network_policies.unwrap_or_default(),
        },
    }
}

// ── Handlers: Frameworks ─────────────────────────────────────────────────────

async fn list_frameworks(
    State(state): State<Arc<ComplianceState>>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    let frameworks: Vec<&ComplianceFramework> = store.frameworks.values().collect();
    Json(serde_json::json!({ "frameworks": frameworks, "total": frameworks.len() }))
}

async fn get_framework(
    State(state): State<Arc<ComplianceState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    match store.frameworks.get(&id) {
        Some(fw) => Json(serde_json::to_value(fw).unwrap()).into_response(),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "framework not found" }))).into_response(),
    }
}

async fn create_framework(
    State(state): State<Arc<ComplianceState>>,
    Json(req): Json<CreateCustomFrameworkRequest>,
) -> impl IntoResponse {
    let fw = ComplianceFramework {
        id: Uuid::new_v4(),
        name: req.name,
        kind: FrameworkKind::Custom,
        version: req.version,
        description: req.description,
        controls: vec![],
        created_at: Utc::now(),
    };
    let mut store = state.store.write().await;
    store.frameworks.insert(fw.id, fw.clone());
    (StatusCode::CREATED, Json(serde_json::to_value(&fw).unwrap()))
}

async fn list_controls(
    State(state): State<Arc<ComplianceState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    match store.frameworks.get(&id) {
        Some(fw) => {
            let controls = &fw.controls;
            Json(serde_json::json!({ "controls": controls, "total": controls.len() })).into_response()
        }
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "framework not found" }))).into_response(),
    }
}

// ── Handlers: Checks & Scans ─────────────────────────────────────────────────

async fn run_control_check(
    State(state): State<Arc<ComplianceState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<RunCheckRequest>,
) -> impl IntoResponse {
    let ctx = build_check_context(Some(req));
    let store = state.store.read().await;
    match store.controls.get(&id) {
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "control not found" }))).into_response(),
        Some(ctrl) => {
            match crate::checks::run_check(ctrl, &ctx) {
                None => Json(serde_json::json!({ "message": "control is not automated" })).into_response(),
                Some((finding, evidence)) => {
                    drop(store);
                    let mut w = state.store.write().await;
                    w.findings.insert(finding.id, finding.clone());
                    if let Some(ev) = &evidence {
                        w.evidence.insert(ev.id, ev.clone());
                    }
                    Json(serde_json::json!({ "finding": finding, "evidence": evidence })).into_response()
                }
            }
        }
    }
}

async fn scan_framework(
    State(state): State<Arc<ComplianceState>>,
    Json(req): Json<ScanRequest>,
) -> impl IntoResponse {
    let framework_id = req.framework_id;
    let ctx = build_check_context(req.context);

    // 1. Get framework controls
    let controls: Vec<Control> = {
        let store = state.store.read().await;
        match store.frameworks.get(&framework_id) {
            None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "framework not found" }))).into_response(),
            Some(fw) => fw.controls.clone(),
        }
    };

    // 2. Run all automated checks
    let results = crate::checks::run_all_checks(&controls, &ctx);

    // 3. Store findings and evidence
    let findings: Vec<Finding> = {
        let mut store = state.store.write().await;
        let mut stored_findings = Vec::new();
        for (finding, evidence) in results {
            store.findings.insert(finding.id, finding.clone());
            if let Some(ev) = evidence {
                store.evidence.insert(ev.id, ev);
            }
            stored_findings.push(finding);
        }
        // 4. Record audit event
        let event = crate::audit::record_event(
            "system",
            "compliance_scan",
            "framework",
            &framework_id.to_string(),
            serde_json::json!({}),
        );
        store.audit_events.push(event);
        stored_findings
    };

    // 5. Return findings
    Json(serde_json::json!({
        "framework_id": framework_id,
        "findings": findings,
        "total": findings.len(),
    })).into_response()
}

// ── Handlers: Findings ───────────────────────────────────────────────────────

async fn list_findings(
    State(state): State<Arc<ComplianceState>>,
    Query(q): Query<FindingsQuery>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    let findings: Vec<&Finding> = store.findings.values()
        .filter(|f| {
            q.control_ref.as_deref().map_or(true, |cr| f.control_ref == cr) &&
            q.status.as_deref().map_or(true, |s| format!("{:?}", f.status).to_lowercase() == s.to_lowercase())
        })
        .collect();
    Json(serde_json::json!({ "findings": findings, "total": findings.len() }))
}

async fn get_finding(
    State(state): State<Arc<ComplianceState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    match store.findings.get(&id) {
        Some(f) => Json(serde_json::to_value(f).unwrap()).into_response(),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "finding not found" }))).into_response(),
    }
}

async fn create_finding(
    State(state): State<Arc<ComplianceState>>,
    Json(req): Json<CreateFindingRequest>,
) -> impl IntoResponse {
    let finding = Finding {
        id: Uuid::new_v4(),
        control_id: req.control_id,
        control_ref: req.control_ref,
        status: req.status,
        target: req.target,
        details: req.details,
        remediation: req.remediation,
        evidence_ids: vec![],
        checked_at: Utc::now(),
        exception_id: None,
    };
    let mut store = state.store.write().await;
    let event = crate::audit::record_event("api", "create", "finding", &finding.id.to_string(), serde_json::json!({}));
    store.audit_events.push(event);
    store.findings.insert(finding.id, finding.clone());
    (StatusCode::CREATED, Json(serde_json::to_value(&finding).unwrap()))
}

// ── Handlers: Evidence ───────────────────────────────────────────────────────

async fn list_evidence(
    State(state): State<Arc<ComplianceState>>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    let evidence: Vec<&Evidence> = store.evidence.values().collect();
    Json(serde_json::json!({ "evidence": evidence, "total": evidence.len() }))
}

async fn add_evidence(
    State(state): State<Arc<ComplianceState>>,
    Json(req): Json<CreateEvidenceRequest>,
) -> impl IntoResponse {
    let ev = Evidence {
        id: Uuid::new_v4(),
        finding_id: req.finding_id,
        control_id: req.control_id,
        evidence_type: req.evidence_type,
        description: req.description,
        data: req.data,
        collected_at: Utc::now(),
        collected_by: req.collected_by,
    };
    let mut store = state.store.write().await;
    // Link evidence to finding if provided
    if let Some(fid) = ev.finding_id {
        if let Some(finding) = store.findings.get_mut(&fid) {
            finding.evidence_ids.push(ev.id);
        }
    }
    store.evidence.insert(ev.id, ev.clone());
    (StatusCode::CREATED, Json(serde_json::to_value(&ev).unwrap()))
}

// ── Handlers: Audit ──────────────────────────────────────────────────────────

async fn get_audit_trail(
    State(state): State<Arc<ComplianceState>>,
    Query(q): Query<AuditQuery>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    let mut events: Vec<&AuditEvent> = crate::audit::filter_events(
        &store.audit_events,
        q.resource_type.as_deref(),
        q.actor.as_deref(),
    );
    let limit = q.limit.unwrap_or(100);
    events.truncate(limit);
    Json(serde_json::json!({ "events": events, "total": events.len() }))
}

// ── Handlers: Exceptions ─────────────────────────────────────────────────────

async fn list_exceptions(
    State(state): State<Arc<ComplianceState>>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    let exceptions: Vec<&ControlException> = store.exceptions.values().collect();
    Json(serde_json::json!({ "exceptions": exceptions, "total": exceptions.len() }))
}

async fn create_exception(
    State(state): State<Arc<ComplianceState>>,
    Json(req): Json<CreateExceptionRequest>,
) -> impl IntoResponse {
    let exception = ControlException {
        id: Uuid::new_v4(),
        control_id: req.control_id,
        control_ref: req.control_ref,
        reason: req.reason,
        approved_by: req.approved_by,
        expires_at: req.expires_at,
        created_at: Utc::now(),
    };
    let mut store = state.store.write().await;
    let event = crate::audit::record_event("api", "create", "exception", &exception.id.to_string(), serde_json::json!({}));
    store.audit_events.push(event);
    store.exceptions.insert(exception.id, exception.clone());
    (StatusCode::CREATED, Json(serde_json::to_value(&exception).unwrap()))
}

async fn delete_exception(
    State(state): State<Arc<ComplianceState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    match store.exceptions.remove(&id) {
        Some(_) => {
            let event = crate::audit::record_event("api", "delete", "exception", &id.to_string(), serde_json::json!({}));
            store.audit_events.push(event);
            StatusCode::NO_CONTENT.into_response()
        }
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "exception not found" }))).into_response(),
    }
}

// ── Handlers: Reports ────────────────────────────────────────────────────────

async fn list_reports(
    State(state): State<Arc<ComplianceState>>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    let reports: Vec<&ComplianceReport> = store.reports.values().collect();
    Json(serde_json::json!({ "reports": reports, "total": reports.len() }))
}

async fn generate_report(
    State(state): State<Arc<ComplianceState>>,
    Json(req): Json<GenerateReportRequest>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    let framework = match store.frameworks.get(&req.framework_id) {
        Some(fw) => fw.clone(),
        None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "framework not found" }))).into_response(),
    };
    let findings: Vec<Finding> = store.findings.values().cloned().collect();
    let exceptions: Vec<ControlException> = store.exceptions.values().cloned().collect();
    drop(store);

    let report = crate::reports::generate_report(&framework, &findings, &exceptions);
    let mut store = state.store.write().await;
    let event = crate::audit::record_event("api", "generate_report", "framework", &req.framework_id.to_string(), serde_json::json!({}));
    store.audit_events.push(event);
    store.reports.insert(report.id, report.clone());
    (StatusCode::CREATED, Json(serde_json::to_value(&report).unwrap())).into_response()
}

async fn get_report(
    State(state): State<Arc<ComplianceState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    match store.reports.get(&id) {
        Some(r) => Json(serde_json::to_value(r).unwrap()).into_response(),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "report not found" }))).into_response(),
    }
}

// ── Handlers: Policy Mappings ────────────────────────────────────────────────

async fn list_policy_mappings(
    State(state): State<Arc<ComplianceState>>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    let mappings: Vec<&PolicyMapping> = store.policy_mappings.values().collect();
    Json(serde_json::json!({ "mappings": mappings, "total": mappings.len() }))
}

async fn create_policy_mapping(
    State(state): State<Arc<ComplianceState>>,
    Json(req): Json<CreatePolicyMappingRequest>,
) -> impl IntoResponse {
    let mapping = crate::policy::create_mapping(
        req.control_id,
        &req.control_ref,
        req.policy_engine,
        &req.policy_name,
        req.policy_namespace.as_deref(),
        &req.description,
    );
    let mut store = state.store.write().await;
    store.policy_mappings.insert(mapping.id, mapping.clone());
    (StatusCode::CREATED, Json(serde_json::to_value(&mapping).unwrap()))
}

async fn suggest_policy_mappings(
    Path(control_ref): Path<String>,
) -> impl IntoResponse {
    let suggestions: Vec<serde_json::Value> = crate::policy::suggested_mappings(&control_ref)
        .into_iter()
        .map(|(engine, policy_name, description)| serde_json::json!({
            "policy_engine": engine,
            "policy_name": policy_name,
            "description": description,
        }))
        .collect();
    Json(serde_json::json!({ "control_ref": control_ref, "suggestions": suggestions }))
}

// ── Health ───────────────────────────────────────────────────────────────────

async fn health(
    State(state): State<Arc<ComplianceState>>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    Json(serde_json::json!({
        "module": "cave-compliance",
        "status": "ok",
        "frameworks": store.frameworks.len(),
        "controls": store.controls.len(),
        "findings": store.findings.len(),
        "evidence": store.evidence.len(),
        "exceptions": store.exceptions.len(),
        "reports": store.reports.len(),
        "policy_mappings": store.policy_mappings.len(),
        "upstream": "CIS Kubernetes Benchmark 1.8, SOC2 2017, PCI DSS 4.0, HIPAA 2013",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode as SC},
    };
    use tower::util::ServiceExt;

    fn test_app() -> Router {
        let state = Arc::new(ComplianceState::default());
        create_router(state)
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let app = test_app();
        let resp = app
            .oneshot(Request::builder().uri("/api/compliance/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), SC::OK);
    }

    #[tokio::test]
    async fn test_list_frameworks() {
        let app = test_app();
        let resp = app
            .oneshot(Request::builder().uri("/api/compliance/frameworks").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), SC::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["total"].as_u64().unwrap(), 4);
    }

    #[tokio::test]
    async fn test_get_framework_not_found() {
        let app = test_app();
        let resp = app
            .oneshot(Request::builder().uri(&format!("/api/compliance/frameworks/{}", Uuid::new_v4())).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), SC::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_scan_framework() {
        let state = Arc::new(ComplianceState::default());
        let fw_id = {
            let store = state.store.read().await;
            *store.frameworks.keys().next().unwrap()
        };
        let app = create_router(state);
        let body = serde_json::json!({ "framework_id": fw_id }).to_string();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/compliance/scan")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), SC::OK);
    }

    #[tokio::test]
    async fn test_create_and_list_findings() {
        let state = Arc::new(ComplianceState::default());
        let ctrl_id = {
            let store = state.store.read().await;
            *store.controls.keys().next().unwrap()
        };
        let app = create_router(state);
        let body = serde_json::json!({
            "control_id": ctrl_id,
            "control_ref": "CIS-5.1.1",
            "status": "pass",
            "target": "cluster",
            "details": "RBAC enabled",
        })
        .to_string();
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/compliance/findings")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), SC::CREATED);
    }
}
