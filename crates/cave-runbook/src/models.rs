// SPDX-License-Identifier: AGPL-3.0-or-later
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Runbook {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub version: String,
    pub tags: Vec<String>,
    pub parameters: Vec<ParameterDef>,
    pub steps: Vec<Step>,
    pub timeout_seconds: u64,
    pub on_failure: FailureAction,
    pub is_template: bool,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterDef {
    pub name: String,
    pub description: String,
    pub param_type: ParamType,
    pub required: bool,
    pub default_value: Option<serde_json::Value>,
    pub secret: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamType {
    String,
    Number,
    Boolean,
    Select,
    MultiSelect,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    pub id: String,
    pub name: String,
    pub step_type: StepType,
    pub description: String,
    pub condition: Option<String>,
    pub depends_on: Vec<String>,
    pub timeout_seconds: u64,
    pub retry_count: u32,
    pub continue_on_error: bool,
    pub on_failure: FailureAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StepType {
    Shell {
        command: String,
        working_dir: Option<String>,
        env: HashMap<String, String>,
    },
    Http {
        url: String,
        method: String,
        headers: HashMap<String, String>,
        body: Option<serde_json::Value>,
        expected_status: Option<u16>,
    },
    KubernetesAction {
        action: K8sAction,
        resource_kind: String,
        resource_name: String,
        namespace: String,
        manifest: Option<serde_json::Value>,
    },
    Notification {
        channel: NotificationChannel,
        message: String,
        recipients: Vec<String>,
    },
    Wait {
        duration_seconds: u64,
        message: Option<String>,
    },
    ManualApproval {
        message: String,
        approvers: Vec<String>,
        timeout_seconds: u64,
    },
    SetVariable {
        name: String,
        value: serde_json::Value,
    },
    Conditional {
        condition: String,
        true_steps: Vec<Step>,
        false_steps: Vec<Step>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum K8sAction {
    Apply,
    Delete,
    Restart,
    Scale,
    Patch,
    Get,
    ListPods,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationChannel {
    Slack,
    Email,
    Webhook,
    PagerDuty,
    Teams,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FailureAction {
    Stop,
    Continue,
    Rollback,
    Notify,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Pending,
    Running,
    WaitingForApproval,
    Completed,
    Failed,
    Cancelled,
    TimedOut,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
    WaitingForApproval,
    TimedOut,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Execution {
    pub id: Uuid,
    pub runbook_id: Uuid,
    pub runbook_name: String,
    pub status: ExecutionStatus,
    pub triggered_by: String,
    pub trigger_type: TriggerType,
    pub parameters: HashMap<String, serde_json::Value>,
    pub step_results: Vec<StepResult>,
    pub current_step: Option<String>,
    pub variables: HashMap<String, serde_json::Value>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TriggerType {
    Manual,
    Scheduled,
    Alert,
    Api,
    Webhook,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub step_id: String,
    pub step_name: String,
    pub status: StepStatus,
    pub output: Option<serde_json::Value>,
    pub error: Option<String>,
    pub logs: Vec<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub retry_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: Uuid,
    pub execution_id: Uuid,
    pub step_id: String,
    pub message: String,
    pub approvers: Vec<String>,
    pub approved_by: Option<String>,
    pub rejected_by: Option<String>,
    pub status: ApprovalStatus,
    pub created_at: DateTime<Utc>,
    pub responded_at: Option<DateTime<Utc>>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Rejected,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunbookTrigger {
    pub id: Uuid,
    pub runbook_id: Uuid,
    pub trigger_type: TriggerType,
    pub cron_expression: Option<String>,
    pub alert_source: Option<String>,
    pub alert_condition: Option<String>,
    pub parameters: HashMap<String, serde_json::Value>,
    pub enabled: bool,
    pub last_triggered_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}
