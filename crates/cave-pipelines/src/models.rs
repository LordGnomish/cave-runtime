//! Core data models: Pipeline, PipelineRun, Task, TaskRun, Step, Workspace, Parameter, Result.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Status enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Skipped,
    WaitingApproval,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskType {
    GitClone,
    Build,
    Test,
    Deploy,
    Script,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ParamType {
    String,
    Array,
    Object,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WhenOperator {
    In,
    NotIn,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Rejected,
}

// ---------------------------------------------------------------------------
// Workspace
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceKind {
    EmptyDir,
    Pvc { claim_name: String },
    ConfigMap { name: String },
    Secret { secret_name: String },
    HostPath { path: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceDeclaration {
    pub name: String,
    pub description: Option<String>,
    pub optional: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceBinding {
    pub name: String,
    pub kind: WorkspaceKind,
}

// ---------------------------------------------------------------------------
// Parameters & Results
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterSpec {
    pub name: String,
    pub description: Option<String>,
    pub param_type: ParamType,
    pub default: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterValue {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultSpec {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultValue {
    pub name: String,
    pub value: String,
}

// ---------------------------------------------------------------------------
// Step
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    pub name: String,
    pub image: Option<String>,
    pub command: Vec<String>,
    pub args: Vec<String>,
    pub env: Vec<EnvVar>,
    pub working_dir: Option<String>,
    pub timeout_seconds: Option<u64>,
    pub script: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepLog {
    pub step_name: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Task
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub task_type: TaskType,
    pub params: Vec<ParameterSpec>,
    pub steps: Vec<Step>,
    pub workspaces: Vec<WorkspaceDeclaration>,
    pub results: Vec<ResultSpec>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Task {
    pub fn new(name: impl Into<String>, task_type: TaskType) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            description: None,
            task_type,
            params: Vec::new(),
            steps: Vec::new(),
            workspaces: Vec::new(),
            results: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }
}

// ---------------------------------------------------------------------------
// When expression (conditional execution)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhenExpression {
    pub input: String,
    pub operator: WhenOperator,
    pub values: Vec<String>,
}

// ---------------------------------------------------------------------------
// Task spec (reference inside a pipeline)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSpec {
    pub name: String,
    pub task_ref: Option<String>,
    pub task_inline: Option<Task>,
    pub params: Vec<ParameterValue>,
    pub workspaces: Vec<WorkspaceBinding>,
    pub run_after: Vec<String>,
    pub when: Vec<WhenExpression>,
    pub retries: u32,
    pub timeout_seconds: Option<u64>,
}

impl TaskSpec {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            task_ref: None,
            task_inline: None,
            params: Vec::new(),
            workspaces: Vec::new(),
            run_after: Vec::new(),
            when: Vec::new(),
            retries: 0,
            timeout_seconds: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Pipeline
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pipeline {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub params: Vec<ParameterSpec>,
    pub tasks: Vec<TaskSpec>,
    pub workspaces: Vec<WorkspaceDeclaration>,
    pub results: Vec<ResultSpec>,
    pub finally: Vec<TaskSpec>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Pipeline {
    pub fn new(name: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            description: None,
            params: Vec::new(),
            tasks: Vec::new(),
            workspaces: Vec::new(),
            results: Vec::new(),
            finally: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }
}

// ---------------------------------------------------------------------------
// TaskRun
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRun {
    pub id: Uuid,
    pub task_id: Option<Uuid>,
    pub task_name: String,
    pub pipeline_run_id: Option<Uuid>,
    pub status: RunStatus,
    pub params: Vec<ParameterValue>,
    pub results: Vec<ResultValue>,
    pub step_logs: Vec<StepLog>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl TaskRun {
    pub fn new(task_name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            task_id: None,
            task_name: task_name.into(),
            pipeline_run_id: None,
            status: RunStatus::Pending,
            params: Vec::new(),
            results: Vec::new(),
            step_logs: Vec::new(),
            started_at: None,
            completed_at: None,
            created_at: Utc::now(),
        }
    }
}

// ---------------------------------------------------------------------------
// PipelineRun
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineRun {
    pub id: Uuid,
    pub pipeline_id: Uuid,
    pub pipeline_name: String,
    pub status: RunStatus,
    pub params: Vec<ParameterValue>,
    pub task_runs: Vec<Uuid>,
    pub results: Vec<ResultValue>,
    pub workspaces: Vec<WorkspaceBinding>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl PipelineRun {
    pub fn new(pipeline_id: Uuid, pipeline_name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            pipeline_id,
            pipeline_name: pipeline_name.into(),
            status: RunStatus::Pending,
            params: Vec::new(),
            task_runs: Vec::new(),
            results: Vec::new(),
            workspaces: Vec::new(),
            started_at: None,
            completed_at: None,
            created_at: Utc::now(),
        }
    }
}

// ---------------------------------------------------------------------------
// Artifact
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: Uuid,
    pub task_run_id: Uuid,
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub content_type: String,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Approval gate
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalGate {
    pub id: Uuid,
    pub pipeline_run_id: Uuid,
    pub task_name: String,
    pub status: ApprovalStatus,
    pub approver: Option<String>,
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
}
