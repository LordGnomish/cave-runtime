//! Domain models for the compliance module.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ────────────────────────────────────────────────────────────────────────────
// Framework
// ────────────────────────────────────────────────────────────────────────────

/// Compliance frameworks supported by CAVE.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Framework {
    #[serde(rename = "SOC2")]
    Soc2,
    #[serde(rename = "ISO27001")]
    Iso27001,
    #[serde(rename = "GDPR")]
    Gdpr,
    #[serde(rename = "HIPAA")]
    Hipaa,
    #[serde(rename = "PCI_DSS")]
    PciDss,
    /// User-defined framework.
    Custom(String),
}

impl std::fmt::Display for Framework {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Soc2 => write!(f, "SOC2"),
            Self::Iso27001 => write!(f, "ISO27001"),
            Self::Gdpr => write!(f, "GDPR"),
            Self::Hipaa => write!(f, "HIPAA"),
            Self::PciDss => write!(f, "PCI_DSS"),
            Self::Custom(name) => write!(f, "{name}"),
        }
    }
}

impl std::str::FromStr for Framework {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_uppercase().as_str() {
            "SOC2" => Self::Soc2,
            "ISO27001" => Self::Iso27001,
            "GDPR" => Self::Gdpr,
            "HIPAA" => Self::Hipaa,
            "PCI_DSS" | "PCIDSS" | "PCI-DSS" => Self::PciDss,
            _ => Self::Custom(s.to_string()),
        })
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Control
// ────────────────────────────────────────────────────────────────────────────

/// A specific requirement within a compliance framework (e.g. SOC2 CC6.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Control {
    pub id: Uuid,
    pub framework: Framework,
    /// Short identifier within the framework, e.g. "CC6.1", "A.12.4", "Art.32".
    pub identifier: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub required: bool,
    pub created_at: DateTime<Utc>,
}

/// Maps a CAVE module to the control it satisfies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlMapping {
    pub id: Uuid,
    pub control_id: Uuid,
    /// Name of the CAVE module (e.g. "cave-auth", "cave-vault").
    pub cave_module: String,
    pub description: String,
    /// Whether this mapping can be automatically assessed without human input.
    pub auto_assessable: bool,
}

// ────────────────────────────────────────────────────────────────────────────
// Evidence
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceType {
    Log,
    Screenshot,
    Config,
    Report,
    Attestation,
}

/// Collected proof of compliance for a control.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub id: Uuid,
    pub control_id: Uuid,
    pub evidence_type: EvidenceType,
    pub title: String,
    pub content: String,
    /// Which CAVE module produced this evidence.
    pub source_module: String,
    pub collected_at: DateTime<Utc>,
    pub collected_by: Option<Uuid>,
    pub expires_at: Option<DateTime<Utc>>,
}

// ────────────────────────────────────────────────────────────────────────────
// Assessment
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssessmentStatus {
    Compliant,
    NonCompliant,
    Partial,
    NotApplicable,
}

/// Periodic compliance check result for a single control.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Assessment {
    pub id: Uuid,
    pub control_id: Uuid,
    pub status: AssessmentStatus,
    /// 0.0 (non-compliant) → 1.0 (fully compliant). None for not_applicable.
    pub score: Option<f32>,
    pub findings: Vec<String>,
    pub evidence_ids: Vec<Uuid>,
    pub assessed_at: DateTime<Utc>,
    pub assessed_by: Option<Uuid>,
    pub next_review_at: Option<DateTime<Utc>>,
}

// ────────────────────────────────────────────────────────────────────────────
// Audit Trail
// ────────────────────────────────────────────────────────────────────────────

/// Who reviewed what, when, and with what outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditTrail {
    pub id: Uuid,
    pub reviewer: Uuid,
    pub reviewer_name: String,
    /// Type of entity reviewed: "control", "evidence", "assessment".
    pub target_type: String,
    pub target_id: Uuid,
    pub action: String,
    pub outcome: String,
    pub notes: Option<String>,
    pub reviewed_at: DateTime<Utc>,
}

// ────────────────────────────────────────────────────────────────────────────
// Risk
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RiskSeverity {
    Critical,
    High,
    Medium,
    Low,
    Informational,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RiskLikelihood {
    VeryHigh,
    High,
    Medium,
    Low,
    VeryLow,
}

/// An identified compliance risk with severity, likelihood, and mitigation plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Risk {
    pub id: Uuid,
    pub control_id: Option<Uuid>,
    pub title: String,
    pub description: String,
    pub severity: RiskSeverity,
    pub likelihood: RiskLikelihood,
    pub mitigation_plan: String,
    pub owner: Option<String>,
    pub identified_at: DateTime<Utc>,
    pub mitigated_at: Option<DateTime<Utc>>,
    pub residual_severity: Option<RiskSeverity>,
}

// ────────────────────────────────────────────────────────────────────────────
// Policy Document
// ────────────────────────────────────────────────────────────────────────────

/// A stored compliance policy or procedure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyDocument {
    pub id: Uuid,
    pub framework: Framework,
    pub title: String,
    pub content: String,
    pub version: String,
    pub owner: String,
    pub effective_date: DateTime<Utc>,
    pub review_date: DateTime<Utc>,
    pub approved_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ────────────────────────────────────────────────────────────────────────────
// Compliance Report
// ────────────────────────────────────────────────────────────────────────────

/// Per-control result within a generated report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlResult {
    pub control: Control,
    pub status: AssessmentStatus,
    pub evidence_count: usize,
    pub gaps: Vec<String>,
}

/// Full compliance report per framework with pass/fail per control.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceReport {
    pub id: Uuid,
    pub framework: Framework,
    pub title: String,
    pub generated_at: DateTime<Utc>,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub total_controls: usize,
    pub compliant: usize,
    pub non_compliant: usize,
    pub partial: usize,
    pub not_applicable: usize,
    pub overall_score: f32,
    pub control_results: Vec<ControlResult>,
    pub summary: String,
}

// ────────────────────────────────────────────────────────────────────────────
// Remediation
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RemediationStatus {
    Open,
    InProgress,
    Resolved,
    Accepted,
    Deferred,
}

/// An action item for a non-compliant control with a deadline and owner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Remediation {
    pub id: Uuid,
    pub control_id: Uuid,
    pub assessment_id: Option<Uuid>,
    pub title: String,
    pub description: String,
    pub owner: String,
    pub status: RemediationStatus,
    pub priority: RiskSeverity,
    pub deadline: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub resolution_notes: Option<String>,
}

// ────────────────────────────────────────────────────────────────────────────
// Request / Response DTOs
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateControlRequest {
    pub framework: Framework,
    pub identifier: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub required: bool,
}

#[derive(Debug, Deserialize)]
pub struct CreateEvidenceRequest {
    pub control_id: Uuid,
    pub evidence_type: EvidenceType,
    pub title: String,
    pub content: String,
    pub source_module: String,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateAssessmentRequest {
    pub control_id: Uuid,
    pub status: AssessmentStatus,
    pub score: Option<f32>,
    pub findings: Vec<String>,
    pub evidence_ids: Vec<Uuid>,
    pub next_review_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateRemediationRequest {
    pub control_id: Uuid,
    pub assessment_id: Option<Uuid>,
    pub title: String,
    pub description: String,
    pub owner: String,
    pub priority: RiskSeverity,
    pub deadline: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRemediationRequest {
    pub status: Option<RemediationStatus>,
    pub owner: Option<String>,
    pub resolution_notes: Option<String>,
}

/// Body for POST /assess.
#[derive(Debug, Deserialize)]
pub struct AssessRequest {
    /// Limit assessment to a specific framework. None = all frameworks.
    pub framework: Option<Framework>,
    /// Limit assessment to specific control IDs. None = all controls.
    pub control_ids: Option<Vec<Uuid>>,
}

/// Overall compliance posture for the dashboard.
#[derive(Debug, Serialize)]
pub struct DashboardResponse {
    pub overall_score: f32,
    pub frameworks: Vec<FrameworkStatus>,
    pub total_controls: usize,
    pub compliant: usize,
    pub non_compliant: usize,
    pub partial: usize,
    pub open_remediations: usize,
    pub critical_risks: usize,
    pub last_assessed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct FrameworkStatus {
    pub framework: Framework,
    pub score: f32,
    pub total_controls: usize,
    pub compliant: usize,
}

/// Response for GET /gaps.
#[derive(Debug, Serialize)]
pub struct GapsResponse {
    pub gaps: Vec<ControlGap>,
    pub total: usize,
}

#[derive(Debug, Serialize)]
pub struct ControlGap {
    pub control: Control,
    pub gap_reason: String,
    pub evidence_count: usize,
    pub last_assessed: Option<DateTime<Utc>>,
}
