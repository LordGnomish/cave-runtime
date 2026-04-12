//! Domain models for cave-runbook.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ── Trigger ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerKind {
    Manual,
    Incident,
    Alert,
    Schedule,
}

/// What starts a runbook execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trigger {
    pub kind: TriggerKind,
    /// For Incident: minimum severity that fires this runbook.
    pub incident_severity: Option<String>,
    /// For Alert: alert name pattern to match.
    pub alert_name: Option<String>,
    /// For Schedule: cron expression (e.g. "0 */6 * * *").
    pub cron: Option<String>,
}

// ── RunbookStep ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    ShellCommand,
    ApiCall,
    CaveModuleAction,
    HumanApproval,
    Condition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnFailure {
    /// Skip this step and continue to the next.
    Skip,
    /// Abort the entire runbook.
    Abort,
    /// Retry up to `retry_count` times with exponential back-off.
    Retry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunbookStep {
    pub id: String,
    pub name: String,
    pub action: ActionType,
    /// Arbitrary key/value params interpreted by the action type.
    pub params: HashMap<String, serde_json::Value>,
    pub timeout_secs: u64,
    pub on_failure: OnFailure,
    /// Used when `on_failure = retry`.
    pub retry_count: Option<u32>,
}

// ── Runbook ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Runbook {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub trigger: Trigger,
    pub steps: Vec<RunbookStep>,
    pub owner: String,
    pub tags: Vec<String>,
    pub last_run: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ── RunbookExecution ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Running,
    Completed,
    Failed,
    Aborted,
    PendingApproval,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    Running,
    Success,
    Failed,
    Skipped,
    PendingApproval,
}

/// Result of a single step within an execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub step_id: String,
    pub status: StepStatus,
    pub output: Option<String>,
    pub error: Option<String>,
    pub duration_ms: u64,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

/// A single run of a runbook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunbookExecution {
    pub id: Uuid,
    pub runbook_id: Uuid,
    pub runbook_name: String,
    pub status: ExecutionStatus,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    /// Who or what initiated this run ("manual", "incident:<id>", "alert:<name>").
    pub triggered_by: String,
    pub step_results: Vec<StepResult>,
    pub incident_id: Option<Uuid>,
}

// ── IncidentBinding ───────────────────────────────────────────────────────────

/// Links a runbook to an incident pattern so it fires automatically.
///
/// Example: incident_pattern="high CPU" → runbook "scale-up-runbook".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncidentBinding {
    pub id: Uuid,
    pub name: String,
    /// Substring/keyword to match in the incident title.
    pub incident_pattern: String,
    pub incident_severity: Option<String>,
    pub runbook_id: Uuid,
    /// If true, execute immediately on match; otherwise just surface the suggestion.
    pub auto_execute: bool,
    pub created_at: DateTime<Utc>,
}

// ── ApprovalRequest ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Rejected,
}

/// Human-in-the-loop gate: execution pauses until this is resolved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: Uuid,
    pub execution_id: Uuid,
    pub step_id: String,
    pub message: String,
    pub status: ApprovalStatus,
    pub requested_at: DateTime<Utc>,
    pub responded_at: Option<DateTime<Utc>>,
    pub responder: Option<String>,
}

// ── RunbookTemplate ───────────────────────────────────────────────────────────

/// Reusable runbook skeleton that operators can instantiate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunbookTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub steps: Vec<RunbookStep>,
    pub default_trigger: TriggerKind,
}
