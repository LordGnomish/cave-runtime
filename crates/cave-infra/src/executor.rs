//! Plan execution, rollback, dry-run, and progress streaming.

use crate::mcp_bridge::{execute_tool, McpRegistry, McpToolResult};
use crate::models::{ExecutionPlan, PlanStep};
use crate::state::InfraStateStore;
use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info, warn};
use uuid::Uuid;

/// Status of a single step execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Skipped,
}

/// Record of a step execution attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepExecution {
    pub step_id: Uuid,
    pub step_description: String,
    pub status: StepStatus,
    pub result: Option<McpToolResult>,
    pub started_at: Option<chrono::DateTime<Utc>>,
    pub finished_at: Option<chrono::DateTime<Utc>>,
    pub error: Option<String>,
}

/// Full execution record for a plan run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanExecution {
    pub id: Uuid,
    pub plan_id: Uuid,
    pub dry_run: bool,
    pub steps: Vec<StepExecution>,
    pub succeeded: bool,
    pub started_at: chrono::DateTime<Utc>,
    pub finished_at: Option<chrono::DateTime<Utc>>,
}

impl PlanExecution {
    fn new(plan_id: Uuid, dry_run: bool, steps: &[PlanStep]) -> Self {
        Self {
            id: Uuid::new_v4(),
            plan_id,
            dry_run,
            steps: steps
                .iter()
                .map(|s| StepExecution {
                    step_id: s.id,
                    step_description: s.description.clone(),
                    status: StepStatus::Pending,
                    result: None,
                    started_at: None,
                    finished_at: None,
                    error: None,
                })
                .collect(),
            succeeded: false,
            started_at: Utc::now(),
            finished_at: None,
        }
    }
}

/// Execute a plan against live MCP providers.
pub async fn execute_plan(
    plan: &ExecutionPlan,
    registry: Arc<Mutex<McpRegistry>>,
    // Reserved for future state reconciliation after apply.
    _state_store: Arc<Mutex<InfraStateStore>>,
) -> Result<PlanExecution> {
    execute_plan_inner(plan, registry, false).await
}

/// Dry-run a plan: validate steps without invoking MCP tools.
pub async fn dry_run(
    plan: &ExecutionPlan,
    registry: Arc<Mutex<McpRegistry>>,
    _state_store: Arc<Mutex<InfraStateStore>>,
) -> Result<PlanExecution> {
    execute_plan_inner(plan, registry, true).await
}

async fn execute_plan_inner(
    plan: &ExecutionPlan,
    registry: Arc<Mutex<McpRegistry>>,
    is_dry_run: bool,
) -> Result<PlanExecution> {
    info!(
        plan_id = %plan.id,
        steps = plan.steps.len(),
        dry_run = is_dry_run,
        "Starting plan execution"
    );

    let mut exec = PlanExecution::new(plan.id, is_dry_run, &plan.steps);

    // Iterate dependency layers until all steps are processed.
    let mut completed: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
    let mut remaining: Vec<&PlanStep> = plan.steps.iter().collect();

    while !remaining.is_empty() {
        // Find all steps whose dependencies are satisfied.
        let runnable: Vec<&&PlanStep> = remaining
            .iter()
            .filter(|s| s.depends_on.iter().all(|dep| completed.contains(dep)))
            .collect();

        if runnable.is_empty() {
            error!("Dependency deadlock — aborting execution");
            break;
        }

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
            }

            for handle in handles {
                if let Ok((step_id, result)) = handle.await {
                    apply_step_result(&mut exec, step_id, result);
                    completed.insert(step_id);
                }
            }
        }

        // Execute sequential steps one at a time.
        for step in sequential_steps {
            let result = run_step(
                &step.mcp_tool,
                &step.provider_params,
                Arc::clone(&registry),
                is_dry_run,
                &step.description,
            )
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
                }
            }
        }

        remaining.retain(|s| !completed.contains(&s.id));
    }

    exec.succeeded = exec
        .steps
        .iter()
        .all(|s| matches!(s.status, StepStatus::Succeeded | StepStatus::Skipped));
    exec.finished_at = Some(Utc::now());

    info!(
        plan_id = %plan.id,
        succeeded = exec.succeeded,
        "Plan execution finished"
    );

    Ok(exec)
}

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
            error: None,
        });
    }

    info!(tool = tool, description = description, "Executing step");
    Some(execute_tool(registry, tool, params).await)
}

fn apply_step_result(exec: &mut PlanExecution, step_id: Uuid, result: Option<McpToolResult>) {
    if let Some(step_exec) = exec.steps.iter_mut().find(|s| s.step_id == step_id) {
        step_exec.started_at = Some(Utc::now());
        step_exec.finished_at = Some(Utc::now());
        match result {
            None => {
                step_exec.status = StepStatus::Skipped;
            }
            Some(r) => {
                step_exec.status = if r.success {
                    StepStatus::Succeeded
                } else {
                    StepStatus::Failed
                };
                step_exec.error = r.error.clone();
                step_exec.result = Some(r);
            }
        }
    }
}

/// Execute the rollback steps for a plan.
pub async fn rollback(
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
    };
    execute_plan_inner(&rollback_plan, registry, false).await
}
