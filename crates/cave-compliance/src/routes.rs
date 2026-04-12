<<<<<<< HEAD
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
=======
//! HTTP routes for the compliance module.
//!
//! All routes are mounted under `/api/v1/compliance/`.

use crate::models::*;
use crate::{engine, ComplianceState};
use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

// ────────────────────────────────────────────────────────────────────────────
// Router
// ────────────────────────────────────────────────────────────────────────────

pub fn create_router(state: Arc<ComplianceState>) -> Router {
    Router::new()
        // ── Frameworks ────────────────────────────────────────────────────
        .route("/api/v1/compliance/frameworks", get(list_frameworks))
        // ── Controls ──────────────────────────────────────────────────────
        .route(
            "/api/v1/compliance/controls",
            get(list_controls).post(create_control),
        )
        .route(
            "/api/v1/compliance/controls/:id",
            get(get_control).delete(delete_control),
        )
        // ── Evidence ──────────────────────────────────────────────────────
        .route(
            "/api/v1/compliance/evidence",
            get(list_evidence).post(create_evidence),
        )
        .route(
            "/api/v1/compliance/evidence/:id",
            get(get_evidence).delete(delete_evidence),
        )
        // ── Assessments ───────────────────────────────────────────────────
        .route(
            "/api/v1/compliance/assessments",
            get(list_assessments).post(create_assessment),
        )
        .route("/api/v1/compliance/assessments/:id", get(get_assessment))
        // ── Remediations ──────────────────────────────────────────────────
        .route(
            "/api/v1/compliance/remediations",
            get(list_remediations).post(create_remediation),
        )
        .route(
            "/api/v1/compliance/remediations/:id",
            get(get_remediation)
                .put(update_remediation)
                .delete(delete_remediation),
        )
        // ── Analysis ──────────────────────────────────────────────────────
        .route("/api/v1/compliance/report", get(get_report))
        .route("/api/v1/compliance/gaps", get(get_gaps))
        .route("/api/v1/compliance/assess", post(run_assessment))
        .route("/api/v1/compliance/dashboard", get(get_dashboard))
        // ── Health ────────────────────────────────────────────────────────
        .route("/api/v1/compliance/health", get(health))
        .with_state(state)
}

// ────────────────────────────────────────────────────────────────────────────
// Frameworks
// ────────────────────────────────────────────────────────────────────────────

/// GET /api/v1/compliance/frameworks — list all supported frameworks
async fn list_frameworks() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "frameworks": [
            { "id": "SOC2",     "name": "SOC 2 Type II" },
            { "id": "ISO27001", "name": "ISO/IEC 27001:2022" },
            { "id": "GDPR",     "name": "General Data Protection Regulation" },
            { "id": "HIPAA",    "name": "Health Insurance Portability and Accountability Act" },
            { "id": "PCI_DSS",  "name": "Payment Card Industry Data Security Standard" },
        ]
    }))
}

// ────────────────────────────────────────────────────────────────────────────
// Controls
// ────────────────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
>>>>>>> claude/zen-poincare
struct FrameworkQuery {
    framework: Option<String>,
}

<<<<<<< HEAD
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
=======
/// GET /api/v1/compliance/controls[?framework=SOC2]
async fn list_controls(
    State(state): State<Arc<ComplianceState>>,
    Query(q): Query<FrameworkQuery>,
) -> Json<Vec<Control>> {
    let store = state.store.lock().unwrap();
    let controls: Vec<Control> = match q.framework {
        Some(fw_str) => {
            let fw: Framework = fw_str.parse().unwrap();
            store
                .controls
                .iter()
                .filter(|c| c.framework == fw)
                .cloned()
                .collect()
        }
        None => store.controls.clone(),
    };
    Json(controls)
}

/// POST /api/v1/compliance/controls
async fn create_control(
    State(state): State<Arc<ComplianceState>>,
    Json(req): Json<CreateControlRequest>,
) -> Json<Control> {
    let control = Control {
        id: Uuid::new_v4(),
        framework: req.framework,
        identifier: req.identifier,
        name: req.name,
        description: req.description,
        category: req.category,
        required: req.required,
        created_at: Utc::now(),
    };
    state.store.lock().unwrap().controls.push(control.clone());
    Json(control)
}

/// GET /api/v1/compliance/controls/:id
async fn get_control(
    State(state): State<Arc<ComplianceState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let store = state.store.lock().unwrap();
    match store.controls.iter().find(|c| c.id == id) {
        Some(c) => Json(serde_json::to_value(c).unwrap()),
        None => Json(serde_json::json!({ "error": "control not found" })),
    }
}

/// DELETE /api/v1/compliance/controls/:id
async fn delete_control(
    State(state): State<Arc<ComplianceState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let mut store = state.store.lock().unwrap();
    let before = store.controls.len();
    store.controls.retain(|c| c.id != id);
    let removed = before - store.controls.len();
    Json(serde_json::json!({ "removed": removed }))
}

// ────────────────────────────────────────────────────────────────────────────
// Evidence
// ────────────────────────────────────────────────────────────────────────────

/// GET /api/v1/compliance/evidence[?framework=...]
async fn list_evidence(
    State(state): State<Arc<ComplianceState>>,
    Query(q): Query<FrameworkQuery>,
) -> Json<Vec<Evidence>> {
    let store = state.store.lock().unwrap();
    let evidence: Vec<Evidence> = match q.framework {
        Some(fw_str) => {
            let fw: Framework = fw_str.parse().unwrap();
            // Filter by controls belonging to the requested framework
            let control_ids: Vec<Uuid> = store
                .controls
                .iter()
                .filter(|c| c.framework == fw)
                .map(|c| c.id)
                .collect();
            store
                .evidences
                .iter()
                .filter(|e| control_ids.contains(&e.control_id))
                .cloned()
                .collect()
        }
        None => store.evidences.clone(),
    };
    Json(evidence)
}

/// POST /api/v1/compliance/evidence
async fn create_evidence(
    State(state): State<Arc<ComplianceState>>,
    Json(req): Json<CreateEvidenceRequest>,
) -> Json<Evidence> {
    let ev = Evidence {
>>>>>>> claude/zen-poincare
        id: Uuid::new_v4(),
        control_id: req.control_id,
        evidence_type: req.evidence_type,
        title: req.title,
<<<<<<< HEAD
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
=======
        content: req.content,
        source_module: req.source_module,
        collected_at: Utc::now(),
        collected_by: None,
        expires_at: req.expires_at,
    };
    state.store.lock().unwrap().evidences.push(ev.clone());
    Json(ev)
}

/// GET /api/v1/compliance/evidence/:id
async fn get_evidence(
    State(state): State<Arc<ComplianceState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let store = state.store.lock().unwrap();
    match store.evidences.iter().find(|e| e.id == id) {
        Some(e) => Json(serde_json::to_value(e).unwrap()),
        None => Json(serde_json::json!({ "error": "evidence not found" })),
    }
}

/// DELETE /api/v1/compliance/evidence/:id
async fn delete_evidence(
    State(state): State<Arc<ComplianceState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let mut store = state.store.lock().unwrap();
    let before = store.evidences.len();
    store.evidences.retain(|e| e.id != id);
    Json(serde_json::json!({ "removed": before - store.evidences.len() }))
}

// ────────────────────────────────────────────────────────────────────────────
// Assessments
// ────────────────────────────────────────────────────────────────────────────

/// GET /api/v1/compliance/assessments[?framework=...]
async fn list_assessments(
    State(state): State<Arc<ComplianceState>>,
    Query(q): Query<FrameworkQuery>,
) -> Json<Vec<Assessment>> {
    let store = state.store.lock().unwrap();
    let assessments: Vec<Assessment> = match q.framework {
        Some(fw_str) => {
            let fw: Framework = fw_str.parse().unwrap();
            let control_ids: Vec<Uuid> = store
                .controls
                .iter()
                .filter(|c| c.framework == fw)
                .map(|c| c.id)
                .collect();
            store
                .assessments
                .iter()
                .filter(|a| control_ids.contains(&a.control_id))
                .cloned()
                .collect()
        }
        None => store.assessments.clone(),
    };
    Json(assessments)
}

/// POST /api/v1/compliance/assessments — manually record an assessment
async fn create_assessment(
    State(state): State<Arc<ComplianceState>>,
    Json(req): Json<CreateAssessmentRequest>,
) -> Json<Assessment> {
    let assessment = Assessment {
        id: Uuid::new_v4(),
        control_id: req.control_id,
        status: req.status,
        score: req.score,
        findings: req.findings,
        evidence_ids: req.evidence_ids,
        assessed_at: Utc::now(),
        assessed_by: None,
        next_review_at: req.next_review_at,
    };
    state
        .store
        .lock()
        .unwrap()
        .assessments
        .push(assessment.clone());
    Json(assessment)
}

/// GET /api/v1/compliance/assessments/:id
async fn get_assessment(
    State(state): State<Arc<ComplianceState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let store = state.store.lock().unwrap();
    match store.assessments.iter().find(|a| a.id == id) {
        Some(a) => Json(serde_json::to_value(a).unwrap()),
        None => Json(serde_json::json!({ "error": "assessment not found" })),
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Remediations
// ────────────────────────────────────────────────────────────────────────────

/// GET /api/v1/compliance/remediations
async fn list_remediations(
    State(state): State<Arc<ComplianceState>>,
) -> Json<Vec<Remediation>> {
    Json(state.store.lock().unwrap().remediations.clone())
}

/// POST /api/v1/compliance/remediations
async fn create_remediation(
    State(state): State<Arc<ComplianceState>>,
    Json(req): Json<CreateRemediationRequest>,
) -> Json<Remediation> {
    let rem = Remediation {
        id: Uuid::new_v4(),
        control_id: req.control_id,
        assessment_id: req.assessment_id,
        title: req.title,
        description: req.description,
        owner: req.owner,
        status: RemediationStatus::Open,
        priority: req.priority,
        deadline: req.deadline,
        created_at: Utc::now(),
        resolved_at: None,
        resolution_notes: None,
    };
    state
        .store
        .lock()
        .unwrap()
        .remediations
        .push(rem.clone());
    Json(rem)
}

/// GET /api/v1/compliance/remediations/:id
async fn get_remediation(
    State(state): State<Arc<ComplianceState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let store = state.store.lock().unwrap();
    match store.remediations.iter().find(|r| r.id == id) {
        Some(r) => Json(serde_json::to_value(r).unwrap()),
        None => Json(serde_json::json!({ "error": "remediation not found" })),
    }
}

/// PUT /api/v1/compliance/remediations/:id
async fn update_remediation(
    State(state): State<Arc<ComplianceState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateRemediationRequest>,
) -> Json<serde_json::Value> {
    let mut store = state.store.lock().unwrap();
    match store.remediations.iter_mut().find(|r| r.id == id) {
        Some(rem) => {
            if let Some(status) = req.status {
                if status == RemediationStatus::Resolved {
                    rem.resolved_at = Some(Utc::now());
                }
                rem.status = status;
            }
            if let Some(owner) = req.owner {
                rem.owner = owner;
            }
            if let Some(notes) = req.resolution_notes {
                rem.resolution_notes = Some(notes);
            }
            Json(serde_json::to_value(&*rem).unwrap())
        }
        None => Json(serde_json::json!({ "error": "remediation not found" })),
    }
}

/// DELETE /api/v1/compliance/remediations/:id
async fn delete_remediation(
    State(state): State<Arc<ComplianceState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let mut store = state.store.lock().unwrap();
    let before = store.remediations.len();
    store.remediations.retain(|r| r.id != id);
    Json(serde_json::json!({ "removed": before - store.remediations.len() }))
}

// ────────────────────────────────────────────────────────────────────────────
// Report
// ────────────────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct ReportQuery {
    framework: String,
}

/// GET /api/v1/compliance/report?framework=SOC2
async fn get_report(
    State(state): State<Arc<ComplianceState>>,
    Query(q): Query<ReportQuery>,
) -> Json<ComplianceReport> {
    let framework: Framework = q.framework.parse().unwrap();
    let store = state.store.lock().unwrap();
    let period_end = Utc::now();
    let period_start = period_end - chrono::Duration::days(30);
    let report = engine::generate_report(&store, &framework, period_start, period_end);
    Json(report)
}

// ────────────────────────────────────────────────────────────────────────────
// Gaps
// ────────────────────────────────────────────────────────────────────────────

/// GET /api/v1/compliance/gaps
async fn get_gaps(State(state): State<Arc<ComplianceState>>) -> Json<GapsResponse> {
    let store = state.store.lock().unwrap();
    Json(engine::detect_gaps(&store))
}

// ────────────────────────────────────────────────────────────────────────────
// Assess (on-demand)
// ────────────────────────────────────────────────────────────────────────────

/// POST /api/v1/compliance/assess — run assessment for matching controls
async fn run_assessment(
    State(state): State<Arc<ComplianceState>>,
    Json(req): Json<AssessRequest>,
) -> Json<Vec<Assessment>> {
    let mut store = state.store.lock().unwrap();

    // Determine which control IDs to assess
    let control_ids: Vec<Uuid> = if let Some(ids) = req.control_ids {
        ids
    } else if let Some(fw) = &req.framework {
        store
            .controls
            .iter()
            .filter(|c| &c.framework == fw)
            .map(|c| c.id)
            .collect()
    } else {
        store.controls.iter().map(|c| c.id).collect()
    };

    // Clone targeted controls so we can borrow the store immutably in the loop
    let controls: Vec<Control> = store
        .controls
        .iter()
        .filter(|c| control_ids.contains(&c.id))
        .cloned()
        .collect();

    let mut new_assessments: Vec<Assessment> = Vec::new();
    for control in &controls {
        let assessment = engine::assess_control(&store, control);
        new_assessments.push(assessment.clone());
        store.assessments.push(assessment);
    }

    Json(new_assessments)
}

// ────────────────────────────────────────────────────────────────────────────
// Dashboard
// ────────────────────────────────────────────────────────────────────────────

/// GET /api/v1/compliance/dashboard — overall compliance posture
async fn get_dashboard(State(state): State<Arc<ComplianceState>>) -> Json<DashboardResponse> {
    let store = state.store.lock().unwrap();

    let total_controls = store.controls.len();
    let mut compliant = 0usize;
    let mut non_compliant = 0usize;
    let mut partial = 0usize;

    for control in &store.controls {
        let status = store
            .assessments
            .iter()
            .filter(|a| a.control_id == control.id)
            .max_by_key(|a| a.assessed_at)
            .map(|a| &a.status);

        match status {
            Some(AssessmentStatus::Compliant) => compliant += 1,
            Some(AssessmentStatus::Partial) => partial += 1,
            _ => non_compliant += 1,
        }
    }

    let overall_score = if total_controls > 0 {
        (compliant as f32 + partial as f32 * 0.5) / total_controls as f32
    } else {
        0.0
    };

    // Per-framework breakdown
    let all_frameworks = [
        Framework::Soc2,
        Framework::Iso27001,
        Framework::Gdpr,
        Framework::Hipaa,
        Framework::PciDss,
    ];
    let frameworks: Vec<FrameworkStatus> = all_frameworks
        .iter()
        .filter_map(|fw| {
            let fw_controls: Vec<&Control> = store
                .controls
                .iter()
                .filter(|c| &c.framework == fw)
                .collect();
            if fw_controls.is_empty() {
                return None;
            }
            let fw_total = fw_controls.len();
            let fw_compliant = fw_controls
                .iter()
                .filter(|c| {
                    store
                        .assessments
                        .iter()
                        .filter(|a| a.control_id == c.id)
                        .max_by_key(|a| a.assessed_at)
                        .map(|a| a.status == AssessmentStatus::Compliant)
                        .unwrap_or(false)
                })
                .count();
            Some(FrameworkStatus {
                framework: fw.clone(),
                score: fw_compliant as f32 / fw_total as f32,
                total_controls: fw_total,
                compliant: fw_compliant,
            })
        })
        .collect();

    let open_remediations = store
        .remediations
        .iter()
        .filter(|r| r.status == RemediationStatus::Open || r.status == RemediationStatus::InProgress)
        .count();

    let critical_risks = store
        .risks
        .iter()
        .filter(|r| r.severity == RiskSeverity::Critical)
        .count();

    let last_assessed_at = store
        .assessments
        .iter()
        .map(|a| a.assessed_at)
        .max();

    Json(DashboardResponse {
        overall_score,
        frameworks,
        total_controls,
        compliant,
        non_compliant,
        partial,
        open_remediations,
        critical_risks,
        last_assessed_at,
    })
}

// ────────────────────────────────────────────────────────────────────────────
// Health
// ────────────────────────────────────────────────────────────────────────────

async fn health(State(state): State<Arc<ComplianceState>>) -> Json<serde_json::Value> {
    let store = state.store.lock().unwrap();
    Json(serde_json::json!({
        "module": "cave-compliance",
        "status": "ok",
        "controls": store.controls.len(),
        "assessments": store.assessments.len(),
        "evidences": store.evidences.len(),
        "remediations": store.remediations.len(),
    }))
>>>>>>> claude/zen-poincare
}
