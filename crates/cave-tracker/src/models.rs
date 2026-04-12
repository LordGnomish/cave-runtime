//! Domain models for cave-tracker.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Enumerations ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum IssueType {
    Epic,
    Story,
    Task,
    Bug,
    Subtask,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Critical,
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SprintStatus {
    Planning,
    Active,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CustomFieldType {
    Text,
    Number,
    Date,
    Select,
    MultiSelect,
    User,
}

// ── Core entities ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: Uuid,
    /// Short uppercase key used as issue prefix, e.g. "CAVE".
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub lead: Option<String>,
    pub default_workflow: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub id: Uuid,
    /// Human-readable key, e.g. "CAVE-123".
    pub key: String,
    pub project_id: Uuid,
    pub title: String,
    /// Markdown body.
    pub description: Option<String>,
    pub issue_type: IssueType,
    pub status: String,
    pub priority: Priority,
    pub assignee: Option<String>,
    pub reporter: String,
    pub labels: Vec<Uuid>,
    pub sprint_id: Option<Uuid>,
    pub epic_id: Option<Uuid>,
    pub story_points: Option<f32>,
    /// Seconds.
    pub time_estimate: Option<u64>,
    /// Seconds logged.
    pub time_spent: Option<u64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub due_date: Option<DateTime<Utc>>,
    pub parent_id: Option<Uuid>,
    /// IDs of issues this issue depends on (blockers).
    pub dependencies: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sprint {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub goal: Option<String>,
    pub start_date: Option<DateTime<Utc>>,
    pub end_date: Option<DateTime<Utc>>,
    pub status: SprintStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardColumn {
    pub id: String,
    pub name: String,
    /// Issue statuses that map to this column.
    pub status_mappings: Vec<String>,
    pub wip_limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Board {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub columns: Vec<BoardColumn>,
    pub filters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowTransition {
    pub from: String,
    pub to: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub id: Uuid,
    pub name: String,
    pub states: Vec<String>,
    pub transitions: Vec<WorkflowTransition>,
    pub initial_state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comment {
    pub id: Uuid,
    pub issue_id: Uuid,
    pub author: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Activity {
    pub id: Uuid,
    pub issue_id: Uuid,
    pub actor: String,
    pub action: String,
    pub field_changed: Option<String>,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Label {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    /// CSS hex color, e.g. "#ff0000".
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomField {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub field_type: CustomFieldType,
    pub options: Vec<String>,
}

// ── Planning / metrics ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycleMetrics {
    pub project_id: Uuid,
    pub sprint_id: Option<Uuid>,
    pub avg_cycle_time_hours: f64,
    pub avg_lead_time_hours: f64,
    pub throughput: usize,
    pub velocity: f32,
    pub calculated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacklogItem {
    pub issue_id: Uuid,
    /// Lower rank = higher priority.
    pub rank: f64,
    pub added_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Milestone {
    pub id: Uuid,
    pub name: String,
    pub target_date: DateTime<Utc>,
    pub epic_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Roadmap {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub milestones: Vec<Milestone>,
    pub created_at: DateTime<Utc>,
}

// ── Automation ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AutomationTrigger {
    IssueCreated,
    StatusChanged {
        from: Option<String>,
        to: Option<String>,
    },
    SprintStarted,
    DueDateApproaching {
        days_before: u32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AutomationCondition {
    Always,
    IssueType { issue_type: IssueType },
    Priority { priority: Priority },
    HasLabel { label_name: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AutomationAction {
    Assign { to: String },
    Transition { to_status: String },
    AddLabel { label_name: String },
    Notify { message: String },
    CreateSubtask { title: String, assignee: Option<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Automation {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub trigger: AutomationTrigger,
    pub condition: AutomationCondition,
    pub action: AutomationAction,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}
