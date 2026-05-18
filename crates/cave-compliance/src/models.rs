// SPDX-License-Identifier: AGPL-3.0-or-later
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FrameworkKind { CisKubernetes, Soc2, PciDss, Hipaa, Nist800_53, Custom }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceFramework {
    pub id: Uuid,
    pub name: String,
    pub kind: FrameworkKind,
    pub version: String,
    pub description: String,
    pub controls: Vec<Control>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Control {
    pub id: Uuid,
    pub framework_id: Uuid,
    pub control_id: String,    // e.g., "CIS-1.1.1", "SOC2-CC6.1"
    pub title: String,
    pub description: String,
    pub category: String,
    pub severity: ControlSeverity,
    pub automated: bool,       // can be auto-checked
    pub check_fn: Option<String>, // name of check function
    pub remediation: String,
    pub references: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ControlSeverity { Critical, High, Medium, Low, Informational }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FindingStatus { Pass, Fail, Warn, NotApplicable, Error, Manual }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub id: Uuid,
    pub control_id: Uuid,
    pub control_ref: String,   // e.g., "CIS-1.1.1"
    pub status: FindingStatus,
    pub target: String,        // resource that was checked
    pub details: String,
    pub remediation: Option<String>,
    pub evidence_ids: Vec<Uuid>,
    pub checked_at: DateTime<Utc>,
    pub exception_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub id: Uuid,
    pub finding_id: Option<Uuid>,
    pub control_id: Uuid,
    pub evidence_type: EvidenceType,
    pub description: String,
    pub data: serde_json::Value,
    pub collected_at: DateTime<Utc>,
    pub collected_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceType { Screenshot, ApiResponse, ConfigSnapshot, LogEntry, Manual }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: Uuid,
    pub actor: String,
    pub action: String,
    pub resource_type: String,
    pub resource_id: String,
    pub details: serde_json::Value,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlException {
    pub id: Uuid,
    pub control_id: Uuid,
    pub control_ref: String,
    pub reason: String,
    pub approved_by: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceReport {
    pub id: Uuid,
    pub name: String,
    pub framework_id: Uuid,
    pub framework_name: String,
    pub total_controls: usize,
    pub passed: usize,
    pub failed: usize,
    pub warned: usize,
    pub not_applicable: usize,
    pub manual: usize,
    pub compliance_score: f64,   // 0.0-100.0
    pub findings: Vec<Finding>,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyMapping {
    pub id: Uuid,
    pub control_id: Uuid,
    pub control_ref: String,
    pub policy_engine: PolicyEngine,
    pub policy_name: String,
    pub policy_namespace: Option<String>,
    pub description: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyEngine { Opa, Kyverno, Custom }

