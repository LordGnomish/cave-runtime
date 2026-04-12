//! Plan executor — run steps in dependency order, rollback, dry-run, stream progress.

use crate::mcp_bridge::McpRegistry;
use crate::models::{ExecutionPlan, InfraResource, PlanStatus, ResourceState, StepAction};
use crate::state::InfraStateStore;
use chrono::{DateTime, Utc};
use serde::Serialize;
use tracing::{error, info, warn};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct ExecutionResult {
    pub plan_id: Uuid,
    pub succeeded: bool,
    pub steps_completed: usize,
    pub steps_failed: usize,
    pub outputs: Vec<StepOutput>,
    pub error: Option<String>,
    pub completed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StepOutput {
    pub step_id: Uuid,
    pub resource_name: String,
    pub output: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExecutionProgress {
    pub plan_id: Uuid,
    pub current_step: usize,
    pub total_steps: usize,
    pub step_name: String,
    pub status: ProgressStatus,
    pub elapsed_secs: f64,
}

#[derive(Debug, Clone, Serialize)]
pub enum ProgressStatus {
    Pending,
    Running,
    Succeeded,
    Failed(String),
}

/// Execute a plan: run steps in dependency order via MCP, update state on each step.
pub async fn execute_plan(
    plan: &mut ExecutionPlan,
    registry: &McpRegistry,
    store: &mut InfraStateStore,
) -> ExecutionResult {
    let plan_id = plan.id;
    let total = plan.steps.len();
    let mut outputs = Vec::new();
    let mut failed = 0usize;
    let mut error_msg: Option<String> = None;

    store.snapshot();

    for (i, step) in plan.steps.iter().enumerate() {
        let tool = format!("{}_{}", action_verb(&step.action), step.resource_type);

        info!(
            plan_id = %plan_id,
            step = i + 1,
            total,
            resource = %step.resource_name,
            action = ?step.action,
            tool = %tool,
            "Executing plan step"
        );

        match registry.execute_tool(&step.provider, &tool, &step.params).await {
            Ok(output) => {
                outputs.push(StepOutput {
                    step_id: step.id,
                    resource_name: step.resource_name.clone(),
                    output,
                });

                let now = Utc::now();
                let resource = InfraResource {
                    id: step.id,
                    name: step.resource_name.clone(),
                    provider: step.provider.clone(),
                    resource_type: step.resource_type.clone(),
                    config: step.params.clone(),
                    state: if step.action == StepAction::Delete {
                        ResourceState::Deleted
                    } else {
                        ResourceState::Active
                    },
                    dependencies: step.depends_on.clone(),
                    actual_id: None,
                    created_at: now,
                    updated_at: now,
                };
                store.state.resources.insert(step.id, resource);
            }
            Err(e) => {
                error!(
                    step = i + 1,
                    resource = %step.resource_name,
                    error = %e,
                    "Plan step failed"
                );
                failed += 1;
                error_msg = Some(e.to_string());
                break;
            }
        }
    }

    if failed == 0 {
        plan.status = PlanStatus::Applied;
        store.state.last_applied = Some(Utc::now());
    } else {
        plan.status = PlanStatus::Failed(error_msg.clone().unwrap_or_default());
    }

    ExecutionResult {
        plan_id,
        succeeded: failed == 0,
        steps_completed: outputs.len(),
        steps_failed: failed,
        outputs,
        error: error_msg,
        completed_at: Utc::now(),
    }
}

/// Rollback a failed plan by executing its rollback steps in reverse.
pub async fn rollback(
    plan: &mut ExecutionPlan,
    registry: &McpRegistry,
    store: &mut InfraStateStore,
) -> ExecutionResult {
    warn!(plan_id = %plan.id, rollback_steps = plan.rollback_steps.len(), "Rolling back plan");
    store.snapshot();

    let total = plan.rollback_steps.len();
    let mut outputs = Vec::new();
    let mut failed = 0usize;

    for (i, step) in plan.rollback_steps.iter().enumerate() {
        let tool = format!("{}_{}", action_verb(&step.action), step.resource_type);

        info!(
            plan_id = %plan.id,
            step = i + 1,
            total,
            resource = %step.resource_name,
            "Rollback step"
        );

        match registry.execute_tool(&step.provider, &tool, &step.params).await {
            Ok(output) => {
                outputs.push(StepOutput {
                    step_id: step.id,
                    resource_name: step.resource_name.clone(),
                    output,
                });
            }
            Err(e) => {
                error!(step = i + 1, resource = %step.resource_name, error = %e, "Rollback step failed");
                failed += 1;
            }
        }
    }

    plan.status = PlanStatus::RolledBack;

    ExecutionResult {
        plan_id: plan.id,
        succeeded: failed == 0,
        steps_completed: outputs.len(),
        steps_failed: failed,
        outputs,
        error: None,
        completed_at: Utc::now(),
    }
}

/// Simulate execution without making any real changes. Returns a log of what would happen.
pub fn dry_run(plan: &ExecutionPlan) -> Vec<String> {
    plan.steps
        .iter()
        .map(|s| {
            format!(
                "[DRY-RUN] {:?} {} ({}) via {} provider [~{}s, reversible={}]",
                s.action,
                s.resource_name,
                s.resource_type,
                s.provider,
                s.estimated_duration_secs,
                s.reversible
            )
        })
        .collect()
}

/// Return per-step progress descriptors for streaming to a UI.
pub fn stream_progress(plan: &ExecutionPlan) -> Vec<ExecutionProgress> {
    plan.steps
        .iter()
        .enumerate()
        .map(|(i, s)| ExecutionProgress {
            plan_id: plan.id,
            current_step: i + 1,
            total_steps: plan.steps.len(),
            step_name: format!("{:?} {}", s.action, s.resource_name),
            status: ProgressStatus::Pending,
            elapsed_secs: 0.0,
        })
        .collect()
}

fn action_verb(action: &StepAction) -> &'static str {
    match action {
        StepAction::Create => "create",
        StepAction::Update => "update",
        StepAction::Delete => "delete",
        StepAction::NoOp => "noop",
    }
}
//! Plan execution, rollback, dry-run, and progress streaming.
use crate::mcp_bridge::{execute_tool, McpRegistry, McpToolResult};
use crate::models::{ExecutionPlan, PlanStep};
use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
/// Status of a single step execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Failed,
    Skipped,
/// Record of a step execution attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepExecution {
    pub step_description: String,
    pub status: StepStatus,
    pub result: Option<McpToolResult>,
    pub started_at: Option<chrono::DateTime<Utc>>,
    pub finished_at: Option<chrono::DateTime<Utc>>,
/// Full execution record for a plan run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanExecution {
    pub id: Uuid,
    pub dry_run: bool,
    pub steps: Vec<StepExecution>,
    pub started_at: chrono::DateTime<Utc>,
    pub finished_at: Option<chrono::DateTime<Utc>>,
impl PlanExecution {
    fn new(plan_id: Uuid, dry_run: bool, steps: &[PlanStep]) -> Self {
        Self {
            id: Uuid::new_v4(),
            dry_run,
            steps: steps
                .map(|s| StepExecution {
                    step_id: s.id,
                    step_description: s.description.clone(),
                    status: StepStatus::Pending,
                    result: None,
                    started_at: None,
                    finished_at: None,
                .collect(),
            succeeded: false,
            started_at: Utc::now(),
            finished_at: None,
/// Execute a plan against live MCP providers.
    plan: &ExecutionPlan,
    registry: Arc<Mutex<McpRegistry>>,
    // Reserved for future state reconciliation after apply.
    _state_store: Arc<Mutex<InfraStateStore>>,
) -> Result<PlanExecution> {
    execute_plan_inner(plan, registry, false).await
/// Dry-run a plan: validate steps without invoking MCP tools.
pub async fn dry_run(
    plan: &ExecutionPlan,
    registry: Arc<Mutex<McpRegistry>>,
    _state_store: Arc<Mutex<InfraStateStore>>,
) -> Result<PlanExecution> {
    execute_plan_inner(plan, registry, true).await
async fn execute_plan_inner(
    plan: &ExecutionPlan,
    registry: Arc<Mutex<McpRegistry>>,
    is_dry_run: bool,
) -> Result<PlanExecution> {
        steps = plan.steps.len(),
        dry_run = is_dry_run,
        "Starting plan execution"
    let mut exec = PlanExecution::new(plan.id, is_dry_run, &plan.steps);
    // Iterate dependency layers until all steps are processed.
    let mut completed: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
    let mut remaining: Vec<&PlanStep> = plan.steps.iter().collect();
    while !remaining.is_empty() {
        // Find all steps whose dependencies are satisfied.
        let runnable: Vec<&&PlanStep> = remaining
            .filter(|s| s.depends_on.iter().all(|dep| completed.contains(dep)))
            .collect();
        if runnable.is_empty() {
            error!("Dependency deadlock — aborting execution");
        // Separate parallel and sequential runnable steps.
        let (parallel_steps, sequential_steps): (Vec<&&PlanStep>, Vec<&&PlanStep>) =
            runnable.into_iter().partition(|s| s.parallelizable);
        // Execute parallel steps concurrently.
        if !parallel_steps.is_empty() {
            let mut handles = Vec::new();
            for step in &parallel_steps {
                let step_id = step.id;
                let tool = step.mcp_tool.clone();
                let params = step.provider_params.clone();
                let reg = Arc::clone(&registry);
                let dry = is_dry_run;
                let desc = step.description.clone();
                handles.push(tokio::spawn(async move {
                    (step_id, run_step(&tool, &params, reg, dry, &desc).await)
                }));
            for handle in handles {
                if let Ok((step_id, result)) = handle.await {
                    apply_step_result(&mut exec, step_id, result);
                    completed.insert(step_id);
        // Execute sequential steps one at a time.
        for step in sequential_steps {
            let result = run_step(
                &step.mcp_tool,
                &step.provider_params,
                Arc::clone(&registry),
                is_dry_run,
                &step.description,
            .await;
            apply_step_result(&mut exec, step.id, result.clone());
            completed.insert(step.id);
            // Abort on step failure.
            if let Some(r) = &result {
                if !r.success {
                    warn!(step = %step.description, "Step failed — aborting");
                    exec.succeeded = false;
                    exec.finished_at = Some(Utc::now());
                    return Ok(exec);
        remaining.retain(|s| !completed.contains(&s.id));
    exec.succeeded = exec
        .steps
        .all(|s| matches!(s.status, StepStatus::Succeeded | StepStatus::Skipped));
    exec.finished_at = Some(Utc::now());
        succeeded = exec.succeeded,
        "Plan execution finished"
    Ok(exec)
async fn run_step(
    tool: &str,
    params: &std::collections::HashMap<String, serde_json::Value>,
    registry: Arc<Mutex<McpRegistry>>,
    is_dry_run: bool,
    description: &str,
) -> Option<McpToolResult> {
    if is_dry_run {
        info!(tool = tool, "DRY RUN: would execute tool");
        return Some(McpToolResult {
            tool: tool.to_string(),
            provider_id: Uuid::nil(),
            success: true,
            output: serde_json::json!({ "dry_run": true, "tool": tool }),
    info!(tool = tool, description = description, "Executing step");
    Some(execute_tool(registry, tool, params).await)
fn apply_step_result(exec: &mut PlanExecution, step_id: Uuid, result: Option<McpToolResult>) {
    if let Some(step_exec) = exec.steps.iter_mut().find(|s| s.step_id == step_id) {
        step_exec.started_at = Some(Utc::now());
        step_exec.finished_at = Some(Utc::now());
        match result {
            None => {
                step_exec.status = StepStatus::Skipped;
            Some(r) => {
                step_exec.status = if r.success {
                    StepStatus::Succeeded
                    StepStatus::Failed
                step_exec.error = r.error.clone();
                step_exec.result = Some(r);
/// Execute the rollback steps for a plan.
    plan: &ExecutionPlan,
    registry: Arc<Mutex<McpRegistry>>,
    _state_store: Arc<Mutex<InfraStateStore>>,
) -> Result<PlanExecution> {
    info!(plan_id = %plan.id, "Starting rollback");
    let rollback_plan = ExecutionPlan {
        id: Uuid::new_v4(),
        intent_id: plan.intent_id,
        steps: plan.rollback_steps.clone(),
        rollback_steps: Vec::new(),
        cost_estimate: None,
        risk_score: plan.risk_score,
        explanation: format!("Rollback of plan {}", plan.id),
        created_at: Utc::now(),
    execute_plan_inner(&rollback_plan, registry, false).await
