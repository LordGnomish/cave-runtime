//! HTTP routes for cave-compliance.

use crate::models::*;
use crate::monitor::ComplianceMonitor;
use crate::store::ComplianceStore;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post, put},
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router(store: Arc<ComplianceStore>) -> Router {
    Router::new()
        .route("/api/compliance/health", get(health))
        .route("/api/compliance/controls", get(list_controls))
        .route("/api/compliance/controls/:id", get(get_control))
        .route("/api/compliance/assessments", get(list_assessments))
        .route("/api/compliance/assessments", post(create_assessment))
        .route("/api/compliance/evidence", get(list_evidence))
        .route("/api/compliance/evidence", post(create_evidence))
        .route("/api/compliance/policies", get(list_policies))
        .route("/api/compliance/policies", post(create_policy))
        .route("/api/compliance/policies/:id/acknowledge", post(acknowledge_policy))
        .route("/api/compliance/risks", get(list_risks))
        .route("/api/compliance/risks", post(create_risk))
        .route("/api/compliance/risks/:id", put(update_risk))
        .route("/api/compliance/vendors", get(list_vendors))
        .route("/api/compliance/vendors", post(create_vendor))
        .route("/api/compliance/summary", get(compliance_summary))
        .route("/api/compliance/gaps", get(compliance_gaps))
        .route("/api/compliance/audit-log", get(get_audit_log))
        .with_state(store)
}

// ─── Query params ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct FrameworkQuery {
    framework: Option<String>,
}

#[derive(Deserialize)]
struct ControlIdQuery {
    control_id: Option<String>,
}

#[derive(Deserialize)]
struct AuditLogQuery {
    limit: Option<usize>,
}

fn parse_framework(s: &str) -> Option<Framework> {
    match s.to_lowercase().as_str() {
        "soc2" | "soc2_type_ii" | "soc2typeii" => Some(Framework::Soc2TypeII),
        "iso27001" | "iso_27001" => Some(Framework::Iso27001),
        _ => None,
    }
}

// ─── Handlers ────────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-compliance",
        "status": "ok",
        "upstream": "Vanta, Drata, Tugboat Logic"
    }))
}

async fn list_controls(
    State(store): State<Arc<ComplianceStore>>,
    Query(q): Query<FrameworkQuery>,
) -> Json<Vec<Control>> {
    let framework = q.framework.as_deref().and_then(parse_framework);
    Json(store.list_controls(framework))
}

async fn get_control(
    State(store): State<Arc<ComplianceStore>>,
    Path(id): Path<String>,
) -> Result<Json<Control>, StatusCode> {
    store
        .get_control(&id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn list_assessments(
    State(store): State<Arc<ComplianceStore>>,
) -> Json<Vec<ControlAssessment>> {
    Json(store.list_assessments())
}

async fn create_assessment(
    State(store): State<Arc<ComplianceStore>>,
    Json(req): Json<CreateAssessmentRequest>,
) -> (StatusCode, Json<ControlAssessment>) {
    let assessment = ControlAssessment {
        id: Uuid::new_v4(),
        control_id: req.control_id,
        status: req.status,
        effectiveness_score: req.effectiveness_score,
        gaps: req.gaps.unwrap_or_default(),
        evidence_ids: req.evidence_ids.unwrap_or_default(),
        assessor: req.assessor,
        assessed_at: Utc::now(),
        next_review_date: req.next_review_date,
    };
    store.upsert_assessment(assessment.clone());
    (StatusCode::CREATED, Json(assessment))
}

async fn list_evidence(
    State(store): State<Arc<ComplianceStore>>,
    Query(q): Query<ControlIdQuery>,
) -> Json<Vec<Evidence>> {
    Json(store.list_evidence(q.control_id.as_deref()))
}

async fn create_evidence(
    State(store): State<Arc<ComplianceStore>>,
    Json(req): Json<CreateEvidenceRequest>,
) -> (StatusCode, Json<Evidence>) {
    let evidence = Evidence {
        id: Uuid::new_v4(),
        control_id: req.control_id,
        evidence_type: req.evidence_type,
        title: req.title,
        description: req.description,
        source_module: req.source_module,
        content: req.content,
        collected_at: Utc::now(),
        collected_by: req.collected_by,
        valid_until: req.valid_until,
        is_automated: req.is_automated,
    };
    store.add_evidence(evidence.clone());
    (StatusCode::CREATED, Json(evidence))
}

async fn list_policies(State(store): State<Arc<ComplianceStore>>) -> Json<Vec<Policy>> {
    Json(store.list_policies())
}

async fn create_policy(
    State(store): State<Arc<ComplianceStore>>,
    Json(req): Json<CreatePolicyRequest>,
) -> (StatusCode, Json<Policy>) {
    let policy = store.create_policy(req);
    (StatusCode::CREATED, Json(policy))
}

async fn acknowledge_policy(
    State(store): State<Arc<ComplianceStore>>,
    Path(id): Path<Uuid>,
    Json(req): Json<AcknowledgePolicyRequest>,
) -> Result<Json<Policy>, StatusCode> {
    store
        .acknowledge_policy(id, req.user_id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn list_risks(State(store): State<Arc<ComplianceStore>>) -> Json<Vec<RiskEntry>> {
    Json(store.list_risks())
}

async fn create_risk(
    State(store): State<Arc<ComplianceStore>>,
    Json(req): Json<CreateRiskRequest>,
) -> (StatusCode, Json<RiskEntry>) {
    let risk = store.create_risk(req);
    (StatusCode::CREATED, Json(risk))
}

async fn update_risk(
    State(store): State<Arc<ComplianceStore>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateRiskRequest>,
) -> Result<Json<RiskEntry>, StatusCode> {
    store
        .update_risk(id, req)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn list_vendors(
    State(store): State<Arc<ComplianceStore>>,
) -> Json<Vec<VendorAssessment>> {
    Json(store.list_vendors())
}

async fn create_vendor(
    State(store): State<Arc<ComplianceStore>>,
    Json(req): Json<VendorQuestionnaireRequest>,
) -> (StatusCode, Json<VendorAssessment>) {
    let va = store.create_vendor_assessment(req);
    (StatusCode::CREATED, Json(va))
}

async fn compliance_summary(
    State(store): State<Arc<ComplianceStore>>,
    Query(q): Query<FrameworkQuery>,
) -> Json<ComplianceSummary> {
    let framework = q
        .framework
        .as_deref()
        .and_then(parse_framework)
        .unwrap_or(Framework::Soc2TypeII);
    Json(store.compliance_summary(framework))
}

async fn compliance_gaps(
    State(store): State<Arc<ComplianceStore>>,
    Query(q): Query<FrameworkQuery>,
) -> Json<serde_json::Value> {
    let framework = q
        .framework
        .as_deref()
        .and_then(parse_framework)
        .unwrap_or(Framework::Soc2TypeII);
    let controls = store.list_controls(Some(framework));
    let assessments = store.list_assessments();
    let gaps = ComplianceMonitor::identify_gaps(&controls, &assessments);
    let gap_json: Vec<serde_json::Value> = gaps
        .into_iter()
        .map(|g| {
            serde_json::json!({
                "control_id": g.control_id,
                "control_title": g.control_title,
                "current_status": format!("{:?}", g.current_status),
                "effectiveness_score": g.effectiveness_score,
                "gaps": g.gaps,
                "priority": format!("{:?}", g.priority),
            })
        })
        .collect();
    Json(serde_json::json!({ "gaps": gap_json }))
}

async fn get_audit_log(
    State(store): State<Arc<ComplianceStore>>,
    Query(q): Query<AuditLogQuery>,
) -> Json<Vec<AuditEvent>> {
    let limit = q.limit.unwrap_or(100);
    Json(store.get_audit_log(limit))
}
