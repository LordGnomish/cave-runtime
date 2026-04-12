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
