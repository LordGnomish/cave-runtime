//! Data models for cave-compliance.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ─── Framework ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Framework {
    Soc2TypeII,
    Iso27001,
}

// ─── SOC2 Trust Service Criteria ─────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustServiceCriteria {
    Security,
    Availability,
    ProcessingIntegrity,
    Confidentiality,
    Privacy,
}

// ─── Evidence ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceType {
    AccessLog,
    ConfigSnapshot,
    ScanResult,
    PolicyDocument,
    AuditLog,
    SecurityAssessment,
    IncidentReport,
}

// ─── Control ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlMapping {
    pub framework: Framework,
    pub control_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Control {
    pub id: String,
    pub title: String,
    pub description: String,
    pub framework: Framework,
    pub category: String,
    pub implementation_guidance: String,
    pub evidence_types: Vec<EvidenceType>,
    pub mappings: Vec<ControlMapping>,
    pub created_at: DateTime<Utc>,
}

// ─── Control Assessment ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlStatus {
    NotImplemented,
    Planned,
    InProgress,
    Implemented,
    Tested,
    Audited,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlAssessment {
    pub id: Uuid,
    pub control_id: String,
    pub status: ControlStatus,
    /// Effectiveness score: 0.0–1.0
    pub effectiveness_score: f32,
    pub gaps: Vec<String>,
    pub evidence_ids: Vec<Uuid>,
    pub assessor: Uuid,
    pub assessed_at: DateTime<Utc>,
    pub next_review_date: Option<DateTime<Utc>>,
}

// ─── Evidence ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub id: Uuid,
    pub control_id: String,
    pub evidence_type: EvidenceType,
    pub title: String,
    pub description: String,
    pub source_module: Option<String>,
    pub content: serde_json::Value,
    pub collected_at: DateTime<Utc>,
    pub collected_by: Uuid,
    pub valid_until: Option<DateTime<Utc>>,
    pub is_automated: bool,
}

// ─── Policy ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyStatus {
    Draft,
    Active,
    Deprecated,
    Archived,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyAcknowledgment {
    pub user_id: Uuid,
    pub acknowledged_at: DateTime<Utc>,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub id: Uuid,
    pub title: String,
    pub version: String,
    pub content: String,
    pub status: PolicyStatus,
    pub owner: Uuid,
    pub effective_date: Option<DateTime<Utc>>,
    pub review_date: Option<DateTime<Utc>>,
    pub acknowledgments: Vec<PolicyAcknowledgment>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ─── Risk ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskCategory {
    Technical,
    Operational,
    Compliance,
    Strategic,
    Vendor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskTreatment {
    Accept,
    Mitigate,
    Transfer,
    Avoid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskStatus {
    Open,
    InTreatment,
    Mitigated,
    Accepted,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskEntry {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub category: RiskCategory,
    /// 1–5
    pub likelihood: u8,
    /// 1–5
    pub impact: u8,
    /// likelihood × impact as f32
    pub risk_score: f32,
    pub treatment: RiskTreatment,
    pub treatment_plan: Option<String>,
    pub owner: Uuid,
    pub control_ids: Vec<String>,
    pub status: RiskStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ─── Vendor Assessment ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VendorRiskTier {
    Critical,
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VendorAssessment {
    pub id: Uuid,
    pub vendor_name: String,
    pub vendor_url: Option<String>,
    pub risk_tier: VendorRiskTier,
    pub questionnaire_responses: serde_json::Value,
    pub score: Option<f32>,
    pub reviewed_by: Option<Uuid>,
    pub last_assessed: DateTime<Utc>,
    pub next_review: Option<DateTime<Utc>>,
}

// ─── Audit Event ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: Uuid,
    pub event_type: String,
    pub description: String,
    pub actor: Uuid,
    pub resource_type: String,
    pub resource_id: String,
    pub old_value: Option<serde_json::Value>,
    pub new_value: Option<serde_json::Value>,
    pub ip_address: Option<String>,
    pub created_at: DateTime<Utc>,
}

// ─── Compliance Summary ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceSummary {
    pub framework: Framework,
    pub total_controls: usize,
    pub implemented: usize,
    pub tested: usize,
    pub gaps: usize,
    pub effectiveness_score: f32,
}

// ─── Request types ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct CreateAssessmentRequest {
    pub control_id: String,
    pub status: ControlStatus,
    pub effectiveness_score: f32,
    pub gaps: Option<Vec<String>>,
    pub evidence_ids: Option<Vec<Uuid>>,
    pub assessor: Uuid,
    pub next_review_date: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreatePolicyRequest {
    pub title: String,
    pub version: String,
    pub content: String,
    pub owner: Uuid,
    pub effective_date: Option<DateTime<Utc>>,
    pub review_date: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AcknowledgePolicyRequest {
    pub user_id: Uuid,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateRiskRequest {
    pub title: String,
    pub description: String,
    pub category: RiskCategory,
    pub likelihood: u8,
    pub impact: u8,
    pub treatment: RiskTreatment,
    pub treatment_plan: Option<String>,
    pub owner: Uuid,
    pub control_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateRiskRequest {
    pub status: Option<RiskStatus>,
    pub treatment: Option<RiskTreatment>,
    pub treatment_plan: Option<String>,
    pub likelihood: Option<u8>,
    pub impact: Option<u8>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateEvidenceRequest {
    pub control_id: String,
    pub evidence_type: EvidenceType,
    pub title: String,
    pub description: String,
    pub source_module: Option<String>,
    pub content: serde_json::Value,
    pub collected_by: Uuid,
    pub valid_until: Option<DateTime<Utc>>,
    pub is_automated: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VendorQuestionnaireRequest {
    pub vendor_name: String,
    pub vendor_url: Option<String>,
    pub risk_tier: VendorRiskTier,
    pub questionnaire_responses: serde_json::Value,
    pub reviewed_by: Option<Uuid>,
    pub next_review: Option<DateTime<Utc>>,
}
