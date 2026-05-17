// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Pipeline CRD models — Tekton-compatible.
//!
//! Covers: Pipeline, PipelineRun, Task, TaskRun, StepAction,
//! workspaces, parameters, results, when expressions, finally tasks,
//! matrix, retry policies, timeouts.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ─── Parameter types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ParamType {
    String,
    Array,
    Object,
}

impl Default for ParamType {
    fn default() -> Self {
        Self::String
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParamValue {
    String(String),
    Array(Vec<String>),
    Object(HashMap<String, String>),
}

impl ParamValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[String]> {
        match self {
            Self::Array(a) => Some(a),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamSpec {
    pub name: String,
    #[serde(default)]
    pub param_type: ParamType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<ParamValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Param {
    pub name: String,
    pub value: ParamValue,
}

// ─── Workspace types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDeclaration {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub optional: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mount_path: Option<String>,
    #[serde(default)]
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum WorkspaceBinding {
    PersistentVolumeClaim {
        claim_name: String,
        #[serde(default)]
        read_only: bool,
    },
    EmptyDir {
        #[serde(skip_serializing_if = "Option::is_none")]
        medium: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        size_limit: Option<String>,
    },
    ConfigMap {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        items: Option<Vec<KeyToPath>>,
    },
    Secret {
        secret_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        items: Option<Vec<KeyToPath>>,
    },
    Projected {
        sources: Vec<ProjectedSource>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyToPath {
    pub key: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum ProjectedSource {
    ConfigMap { name: String },
    Secret { name: String },
    ServiceAccountToken { audience: String, path: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceMapping {
    pub name: String,
    pub workspace: String,
}

// ─── Results ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultSpec {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub result_type: ParamType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub name: String,
    pub value: ParamValue,
}

// ─── Step / StepAction ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvVar {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_from: Option<EnvVarSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EnvVarSource {
    SecretKeyRef { name: String, key: String },
    ConfigMapKeyRef { name: String, key: String },
    FieldRef { field_path: String },
    ResourceFieldRef { resource: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VolumeMount {
    pub name: String,
    pub mount_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_path: Option<String>,
    #[serde(default)]
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceRequirements {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requests: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limits: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_as_user: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_as_group: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_as_non_root: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_privilege_escalation: Option<bool>,
    #[serde(default)]
    pub read_only_root_filesystem: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub name: String,
    pub image: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<Vec<String>>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Vec<EnvVar>,
    #[serde(default)]
    pub volume_mounts: Vec<VolumeMount>,
    /// Inline shell script (mutually exclusive with command).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourceRequirements>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_context: Option<SecurityContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<String>,
    /// Ref to a StepAction in the catalog.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ref_: Option<StepActionRef>,
    /// Results emitted by this step (file paths).
    #[serde(default)]
    pub results: Vec<StepResultSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepActionRef {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResultSpec {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// A reusable step definition (Tekton StepAction).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepAction {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub image: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<Vec<String>>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    #[serde(default)]
    pub params: Vec<ParamSpec>,
    #[serde(default)]
    pub results: Vec<ResultSpec>,
}

// ─── Sidecar ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sidecar {
    pub name: String,
    pub image: String,
    #[serde(default)]
    pub env: Vec<EnvVar>,
    #[serde(default)]
    pub volume_mounts: Vec<VolumeMount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourceRequirements>,
}

// ─── When expressions ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WhenExpression {
    pub input: String,
    pub operator: WhenOperator,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum WhenOperator {
    In,
    NotIn,
}

impl WhenExpression {
    /// Evaluate against a resolved input string.
    pub fn evaluate(&self, resolved: &str) -> bool {
        let matches = self.values.iter().any(|v| v == resolved);
        match self.operator {
            WhenOperator::In => matches,
            WhenOperator::NotIn => !matches,
        }
    }
}

// ─── Matrix ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Matrix {
    /// Fan-out params: each combination generates a separate TaskRun.
    #[serde(default)]
    pub params: Vec<MatrixParam>,
    /// Include: additional params merged into specific combinations.
    #[serde(default)]
    pub include: Vec<MatrixInclude>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatrixParam {
    pub name: String,
    pub value: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatrixInclude {
    pub params: Vec<Param>,
}

impl Matrix {
    /// Expand to all parameter combinations (cartesian product).
    pub fn expand(&self) -> Vec<Vec<Param>> {
        if self.params.is_empty() {
            return vec![];
        }
        let mut combinations: Vec<Vec<Param>> = vec![vec![]];
        for mp in &self.params {
            let mut next = Vec::new();
            for combo in &combinations {
                for val in &mp.value {
                    let mut c = combo.clone();
                    c.push(Param {
                        name: mp.name.clone(),
                        value: ParamValue::String(val.clone()),
                    });
                    next.push(c);
                }
            }
            combinations = next;
        }
        combinations
    }
}

// ─── Retry + Timeout ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryPolicy {
    #[serde(default = "default_retry_limit")]
    pub limit: u32,
    /// Duration string e.g. "30s"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after: Option<String>,
}

fn default_retry_limit() -> u32 {
    3
}

// ─── Task CRD ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSpec {
    #[serde(default)]
    pub params: Vec<ParamSpec>,
    #[serde(default)]
    pub workspaces: Vec<WorkspaceDeclaration>,
    #[serde(default)]
    pub results: Vec<ResultSpec>,
    pub steps: Vec<Step>,
    #[serde(default)]
    pub sidecars: Vec<Sidecar>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_template: Option<StepTemplate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepTemplate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default)]
    pub env: Vec<EnvVar>,
    #[serde(default)]
    pub volume_mounts: Vec<VolumeMount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourceRequirements>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_context: Option<SecurityContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: Uuid,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    pub spec: TaskSpec,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub labels: HashMap<String, String>,
    #[serde(default)]
    pub annotations: HashMap<String, String>,
}

// ─── Pipeline CRD ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineTaskRef {
    pub name: String,
    /// If true, this is a reference to a catalog task.
    #[serde(default)]
    pub catalog: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddedTaskSpec {
    pub steps: Vec<Step>,
    #[serde(default)]
    pub params: Vec<ParamSpec>,
    #[serde(default)]
    pub workspaces: Vec<WorkspaceDeclaration>,
    #[serde(default)]
    pub results: Vec<ResultSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineTask {
    pub name: String,
    /// Task reference OR embedded spec.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_ref: Option<PipelineTaskRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_spec: Option<EmbeddedTaskSpec>,
    /// DAG dependency: run after these tasks complete.
    #[serde(default)]
    pub run_after: Vec<String>,
    #[serde(default)]
    pub params: Vec<Param>,
    #[serde(default)]
    pub workspaces: Vec<WorkspaceMapping>,
    #[serde(default)]
    pub when: Vec<WhenExpression>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matrix: Option<Matrix>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_policy: Option<RetryPolicy>,
    /// Duration string e.g. "1h"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<String>,
    /// Custom task: ref to an external resource.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_task_ref: Option<CustomTaskRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomTaskRef {
    pub api_version: String,
    pub kind: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineSpec {
    #[serde(default)]
    pub params: Vec<ParamSpec>,
    #[serde(default)]
    pub workspaces: Vec<WorkspaceDeclaration>,
    #[serde(default)]
    pub results: Vec<PipelineResult>,
    pub tasks: Vec<PipelineTask>,
    /// Finally tasks: always run after all tasks complete (success or failure).
    #[serde(default)]
    pub finally: Vec<PipelineTask>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Total timeout for the pipeline.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<PipelineTimeout>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineTimeout {
    /// Total pipeline timeout e.g. "2h"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pipeline: Option<String>,
    /// Per-task timeout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tasks: Option<String>,
    /// Finally timeout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finally: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineResult {
    pub name: String,
    /// Expression referencing a task result e.g. "$(tasks.build.results.image-digest)"
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pipeline {
    pub id: Uuid,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    pub spec: PipelineSpec,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub labels: HashMap<String, String>,
}

// ─── Run status ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub enum RunPhase {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunCondition {
    pub condition_type: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub last_transition_time: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildStatus {
    pub name: String,
    pub phase: RunPhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_time: Option<DateTime<Utc>>,
    pub results: Vec<TaskResult>,
    pub retry_count: u32,
}

// ─── TaskRun ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRunSpec {
    pub task_ref: Option<PipelineTaskRef>,
    pub task_spec: Option<EmbeddedTaskSpec>,
    #[serde(default)]
    pub params: Vec<Param>,
    #[serde(default)]
    pub workspaces: Vec<WorkspaceAssignment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceAssignment {
    pub name: String,
    pub binding: WorkspaceBinding,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRun {
    pub id: Uuid,
    pub name: String,
    pub pipeline_run_id: Option<Uuid>,
    pub pipeline_task_name: Option<String>,
    pub spec: TaskRunSpec,
    pub phase: RunPhase,
    pub results: Vec<TaskResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_time: Option<DateTime<Utc>>,
    pub retry_count: u32,
    #[serde(default)]
    pub log_url: Option<String>,
}

// ─── PipelineRun ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineRunSpec {
    pub pipeline_ref: Option<PipelineRef>,
    pub pipeline_spec: Option<PipelineSpec>,
    #[serde(default)]
    pub params: Vec<Param>,
    #[serde(default)]
    pub workspaces: Vec<WorkspaceAssignment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<PipelineTimeout>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineRef {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineRun {
    pub id: Uuid,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    pub spec: PipelineRunSpec,
    pub phase: RunPhase,
    pub task_runs: Vec<ChildStatus>,
    pub results: Vec<TaskResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_time: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    /// Triggering source (git push, webhook, cron, manual).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_source: Option<TriggerSource>,
    #[serde(default)]
    pub labels: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum TriggerSource {
    Manual { user: String },
    GitPush { repo: String, branch: String, commit_sha: String },
    Webhook { endpoint: String },
    Cron { schedule: String },
}

// ─── Build status ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildStatus {
    pub provider: BuildStatusProvider,
    pub repo: String,
    pub commit_sha: String,
    pub state: BuildState,
    pub context: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum BuildStatusProvider {
    GitHub,
    Bitbucket,
    Gitlab,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum BuildState {
    Pending,
    Running,
    Success,
    Failure,
    Error,
}

// ─── Log entry ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub task_run_id: Uuid,
    pub step_name: String,
    pub timestamp: DateTime<Utc>,
    pub message: String,
    pub stream: LogStream,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LogStream {
    Stdout,
    Stderr,
}

// ─── Artifact passing ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactRef {
    pub pipeline_run_id: Uuid,
    pub task_name: String,
    pub artifact_name: String,
    pub media_type: String,
    pub uri: String,
    pub digest: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn when_expression_in_operator() {
        let w = WhenExpression {
            input: "main".to_string(),
            operator: WhenOperator::In,
            values: vec!["main".to_string(), "master".to_string()],
        };
        assert!(w.evaluate("main"));
        assert!(!w.evaluate("feature"));
    }

    #[test]
    fn when_expression_not_in_operator() {
        let w = WhenExpression {
            input: "skip".to_string(),
            operator: WhenOperator::NotIn,
            values: vec!["skip".to_string()],
        };
        assert!(!w.evaluate("skip"));
        assert!(w.evaluate("run"));
    }

    #[test]
    fn matrix_expand_cartesian_product() {
        let m = Matrix {
            params: vec![
                MatrixParam { name: "os".to_string(), value: vec!["linux".to_string(), "windows".to_string()] },
                MatrixParam { name: "arch".to_string(), value: vec!["amd64".to_string(), "arm64".to_string()] },
            ],
            include: vec![],
        };
        let combos = m.expand();
        assert_eq!(combos.len(), 4);
        // Each combination has 2 params
        assert_eq!(combos[0].len(), 2);
    }

    #[test]
    fn matrix_expand_empty() {
        let m = Matrix { params: vec![], include: vec![] };
        assert!(m.expand().is_empty());
    }

    #[test]
    fn param_value_as_str() {
        let p = ParamValue::String("hello".to_string());
        assert_eq!(p.as_str(), Some("hello"));
        let a = ParamValue::Array(vec!["x".to_string()]);
        assert_eq!(a.as_str(), None);
    }

    #[test]
    fn param_value_as_array() {
        let a = ParamValue::Array(vec!["a".to_string(), "b".to_string()]);
        assert_eq!(a.as_array(), Some(&["a".to_string(), "b".to_string()][..]));
    }

    #[test]
    fn pipeline_spec_roundtrip() {
        let spec = PipelineSpec {
            params: vec![ParamSpec {
                name: "env".to_string(),
                param_type: ParamType::String,
                description: Some("Target environment".to_string()),
                default: Some(ParamValue::String("staging".to_string())),
                enum_values: Some(vec!["staging".to_string(), "prod".to_string()]),
            }],
            workspaces: vec![WorkspaceDeclaration {
                name: "source".to_string(),
                description: None,
                optional: false,
                mount_path: None,
                read_only: false,
            }],
            results: vec![],
            tasks: vec![],
            finally: vec![],
            description: None,
            timeout: None,
        };
        let json = serde_json::to_string(&spec).unwrap();
        let back: PipelineSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back.params.len(), 1);
        assert_eq!(back.params[0].name, "env");
    }
}
