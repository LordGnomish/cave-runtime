use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ===== Projects =====
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: Uuid,
    pub key: String,           // e.g., "CAVE", "PLAT"
    pub name: String,
    pub description: String,
    pub project_type: ProjectType,
    pub workflow_id: Uuid,
    pub lead: String,
    pub members: Vec<ProjectMember>,
    pub issue_types: Vec<IssueType>,
    pub custom_field_ids: Vec<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectType { Scrum, Kanban, Business }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMember {
    pub user_id: String,
    pub username: String,
    pub role: ProjectRole,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectRole { Admin, Developer, Viewer, QA }

// ===== Issue Types =====
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum IssueType { Epic, Story, Task, Bug, Subtask, Custom(String) }

impl std::fmt::Display for IssueType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IssueType::Epic => write!(f, "epic"),
            IssueType::Story => write!(f, "story"),
            IssueType::Task => write!(f, "task"),
            IssueType::Bug => write!(f, "bug"),
            IssueType::Subtask => write!(f, "subtask"),
            IssueType::Custom(s) => write!(f, "{}", s),
        }
    }
}

// ===== Issues =====
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub id: Uuid,
    pub key: String,               // e.g., "CAVE-42"
    pub project_id: Uuid,
    pub project_key: String,
    pub issue_type: IssueType,
    pub summary: String,
    pub description: Option<String>,
    pub status: String,            // current status name
    pub priority: Priority,
    pub assignee: Option<String>,
    pub reporter: String,
    pub labels: Vec<String>,
    pub components: Vec<String>,
    pub fix_versions: Vec<String>,
    pub affects_versions: Vec<String>,
    pub epic_id: Option<Uuid>,
    pub parent_id: Option<Uuid>,   // for subtasks
    pub sprint_id: Option<Uuid>,
    pub story_points: Option<f64>,
    pub time_estimate_seconds: Option<u64>,
    pub time_spent_seconds: u64,
    pub custom_fields: HashMap<String, serde_json::Value>,
    pub watchers: Vec<String>,
    pub votes: u64,
    pub rank: i64,                 // for backlog ordering
    pub resolution: Option<String>,
    pub due_date: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Priority { Critical, High, Medium, Low, Trivial }

// ===== Workflow =====
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub id: Uuid,
    pub name: String,
    pub statuses: Vec<WorkflowStatus>,
    pub transitions: Vec<Transition>,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStatus {
    pub name: String,
    pub category: StatusCategory,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StatusCategory { Todo, InProgress, Done }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transition {
    pub id: String,
    pub name: String,
    pub from_status: Vec<String>,   // empty = from any
    pub to_status: String,
    pub conditions: Vec<TransitionCondition>,
    pub validators: Vec<TransitionValidator>,
    pub post_functions: Vec<PostFunction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionCondition { RequireComment, RequireResolution, UserInRole(String) }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionValidator { FieldRequired(String), SubtasksResolved }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PostFunction { SetResolution(String), ClearAssignee, NotifyAssignee, NotifyWatchers, SetField { field: String, value: serde_json::Value } }

// ===== Sprint =====
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sprint {
    pub id: Uuid,
    pub project_id: Uuid,
    pub board_id: Uuid,
    pub name: String,
    pub goal: Option<String>,
    pub state: SprintState,
    pub start_date: Option<DateTime<Utc>>,
    pub end_date: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub velocity: Option<f64>,     // story points completed
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SprintState { Future, Active, Closed }

// ===== Board =====
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Board {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub board_type: BoardType,
    pub columns: Vec<BoardColumn>,
    pub backlog_enabled: bool,
    pub current_sprint_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BoardType { Scrum, Kanban }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardColumn {
    pub name: String,
    pub statuses: Vec<String>,     // workflow statuses mapped to this column
    pub wip_limit: Option<u32>,
}

// ===== Custom Fields =====
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomFieldDef {
    pub id: Uuid,
    pub name: String,
    pub field_type: CustomFieldType,
    pub description: String,
    pub required: bool,
    pub options: Vec<String>,      // for select/multi-select
    pub default_value: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CustomFieldType { Text, Number, Select, MultiSelect, Date, User, Labels, Checkbox }

// ===== Comments =====
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comment {
    pub id: Uuid,
    pub issue_id: Uuid,
    pub author: String,
    pub body: String,
    pub mentions: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ===== Attachments =====
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub id: Uuid,
    pub issue_id: Uuid,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: u64,
    pub url: String,
    pub uploaded_by: String,
    pub uploaded_at: DateTime<Utc>,
}

// ===== Issue Links =====
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueLink {
    pub id: Uuid,
    pub from_issue_id: Uuid,
    pub to_issue_id: Uuid,
    pub link_type: LinkType,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LinkType { Blocks, IsBlockedBy, RelatesTo, Duplicates, IsDuplicatedBy, Clones, IsClonedBy }

// ===== Time Tracking =====
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeLog {
    pub id: Uuid,
    pub issue_id: Uuid,
    pub author: String,
    pub time_spent_seconds: u64,
    pub work_date: DateTime<Utc>,
    pub comment: Option<String>,
    pub created_at: DateTime<Utc>,
}

// ===== Activity Stream =====
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityEvent {
    pub id: Uuid,
    pub issue_id: Option<Uuid>,
    pub project_id: Option<Uuid>,
    pub actor: String,
    pub event_type: ActivityEventType,
    pub details: serde_json::Value,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityEventType { IssueCreated, IssueUpdated, StatusChanged, CommentAdded, AssigneeChanged, SprintStarted, SprintCompleted, AttachmentAdded, IssueLinked }

// ===== Notifications =====
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub id: Uuid,
    pub recipient: String,
    pub issue_id: Uuid,
    pub notification_type: NotificationType,
    pub message: String,
    pub read: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationType { Assigned, Mentioned, Watched, StatusChanged, CommentAdded, DueSoon }
