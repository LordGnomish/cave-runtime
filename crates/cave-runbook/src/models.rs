//! Data models for cave-runbook.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ─── Step Types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepType {
    Command,
    Script,
    HttpRequest,
    ApprovalGate,
    ConditionalBranch,
}

// ─── Failure Action ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "action")]
pub enum FailureAction {
    Continue,
    Abort,
    Retry { max_retries: u8 },
}

// ─── Step Configs ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandStep {
    pub command: String,
    pub args: Vec<String>,
    pub working_dir: Option<String>,
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptStep {
    pub content: String,
    pub interpreter: String,
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpStep {
    pub method: String,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
    pub expected_status: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalStep {
    pub message: String,
    pub approvers: Vec<Uuid>,
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchStep {
    pub condition: String,
    pub if_true_step: usize,
    pub if_false_step: Option<usize>,
}

// ─── Runbook Step ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunbookStep {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub step_type: StepType,
    pub step_config: serde_json::Value,
    pub on_failure: FailureAction,
    /// Empty means sequential (depends on all previous steps completing).
    pub depends_on: Vec<Uuid>,
}

// ─── Runbook Parameter ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamType {
    String,
    Integer,
    Boolean,
    Select,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunbookParameter {
    pub name: String,
    pub description: String,
    pub param_type: ParamType,
    pub required: bool,
    pub default_value: Option<String>,
    /// For Select type: the list of accepted values.
    pub allowed_values: Vec<String>,
}

// ─── Cron Schedule ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronSchedule {
    pub expression: String,
    pub timezone: String,
    pub enabled: bool,
    pub last_run: Option<DateTime<Utc>>,
}

// ─── Access Control ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunbookAccess {
    pub user_id: Option<Uuid>,
    pub role: Option<String>,
    pub can_execute: bool,
    pub can_edit: bool,
}

// ─── Notifications ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum NotificationChannel {
    Slack { webhook_url: String },
    Email { addresses: Vec<String> },
    Webhook { url: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunbookNotifications {
    pub on_start: bool,
    pub on_complete: bool,
    pub on_failure: bool,
    pub channels: Vec<NotificationChannel>,
}

impl Default for RunbookNotifications {
    fn default() -> Self {
        Self {
            on_start: false,
            on_complete: true,
            on_failure: true,
            channels: vec![],
        }
    }
}

// ─── Runbook ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Runbook {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub steps: Vec<RunbookStep>,
    pub parameters: Vec<RunbookParameter>,
    pub schedule: Option<CronSchedule>,
    pub access_control: Vec<RunbookAccess>,
    pub notifications: RunbookNotifications,
    pub timeout_seconds: Option<u64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub created_by: Uuid,
    pub enabled: bool,
}

// ─── Execution ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Pending,
    Running,
    WaitingApproval,
    Completed,
    Failed,
    Cancelled,
    TimedOut,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepExecution {
    pub step_id: Uuid,
    pub step_name: String,
    pub status: ExecutionStatus,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub error: Option<String>,
    pub retries: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Execution {
    pub id: Uuid,
    pub runbook_id: Uuid,
    pub runbook_name: String,
    pub status: ExecutionStatus,
    pub triggered_by: Uuid,
    pub parameters: HashMap<String, serde_json::Value>,
    pub step_executions: Vec<StepExecution>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

// ─── Request Types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct CreateRunbookRequest {
    pub name: String,
    pub description: String,
    pub steps: Vec<RunbookStep>,
    pub parameters: Option<Vec<RunbookParameter>>,
    pub schedule: Option<CronSchedule>,
    pub access_control: Option<Vec<RunbookAccess>>,
    pub notifications: Option<RunbookNotifications>,
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExecuteRunbookRequest {
    pub triggered_by: Uuid,
    pub parameters: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApproveStepRequest {
    pub approver_id: Uuid,
    pub step_id: Uuid,
    pub approved: bool,
    pub comment: Option<String>,
}
