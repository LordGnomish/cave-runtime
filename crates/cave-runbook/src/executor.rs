// SPDX-License-Identifier: AGPL-3.0-or-later
//! Runbook execution engine — steps in order, retries, timeouts, approvals.

use crate::{
    models::{
        ActionType, ApprovalRequest, ApprovalStatus, ExecutionStatus, OnFailure, Runbook,
        RunbookExecution, RunbookStep, StepResult, StepStatus,
    },
    RunbookState,
};
use chrono::Utc;
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tracing::{info, warn};
use uuid::Uuid;

// ── Public entry point ────────────────────────────────────────────────────────

/// Run all steps of `runbook` in order, collecting results.
///
/// The function writes progress to `state.executions` after every step so
/// callers can poll for live status.  Returns the final execution record.
pub async fn execute_runbook(
    state: Arc<RunbookState>,
    exec_id: Uuid,
    runbook: Runbook,
    triggered_by: String,
    incident_id: Option<Uuid>,
) -> RunbookExecution {
    let mut execution = RunbookExecution {
        id: exec_id,
        runbook_id: runbook.id,
        runbook_name: runbook.name.clone(),
        status: ExecutionStatus::Running,
        started_at: Utc::now(),
        completed_at: None,
        triggered_by,
        step_results: Vec::new(),
        incident_id,
    };

    // Persist initial state so polling callers see it immediately.
    store_execution(&state, &execution).await;

    let mut aborted = false;

    for step in &runbook.steps {
        if aborted {
            break;
        }

        let result = execute_step_with_retry(Arc::clone(&state), step).await;

        match result.status {
            StepStatus::PendingApproval => {
                // Pause execution — resume via the approve API.
                execution.status = ExecutionStatus::PendingApproval;
                execution.step_results.push(result);
                store_execution(&state, &execution).await;
                return execution;
            }
            StepStatus::Failed => match step.on_failure {
                OnFailure::Abort => {
                    aborted = true;
                    execution.step_results.push(result);
                }
                OnFailure::Skip => {
                    execution.step_results.push(StepResult {
                        status: StepStatus::Skipped,
                        ..result
                    });
                }
                OnFailure::Retry => {
                    // Retries already exhausted inside execute_step_with_retry.
                    execution.step_results.push(result);
                }
            },
            _ => {
                execution.step_results.push(result);
            }
        }

        store_execution(&state, &execution).await;
    }

    execution.completed_at = Some(Utc::now());
    execution.status = if aborted {
        ExecutionStatus::Aborted
    } else if execution
        .step_results
        .iter()
        .any(|r| r.status == StepStatus::Failed)
    {
        ExecutionStatus::Failed
    } else {
        ExecutionStatus::Completed
    };

    // Stamp last_run on the runbook.
    {
        let mut runbooks = state.runbooks.lock().await;
        if let Some(rb) = runbooks.get_mut(&runbook.id) {
            rb.last_run = Some(Utc::now());
        }
    }

    store_execution(&state, &execution).await;
    execution
}

// ── Step dispatch ─────────────────────────────────────────────────────────────

/// Wrap a single step with retry logic (used when `on_failure = retry`).
async fn execute_step_with_retry(state: Arc<RunbookState>, step: &RunbookStep) -> StepResult {
    let max_retries = match step.on_failure {
        OnFailure::Retry => step.retry_count.unwrap_or(3),
        _ => 0,
    };

    let mut result = execute_step(Arc::clone(&state), step).await;

    for attempt in 0..max_retries {
        if result.status != StepStatus::Failed {
            break;
        }
        let backoff = 2u64.pow(attempt);
        warn!(
            step_id = %step.id,
            attempt = attempt + 1,
            backoff_secs = backoff,
            "Retrying failed step"
        );
        sleep(Duration::from_secs(backoff)).await;
        result = execute_step(Arc::clone(&state), step).await;
    }

    result
}

/// Execute a single step, enforcing the configured timeout.
pub async fn execute_step(state: Arc<RunbookState>, step: &RunbookStep) -> StepResult {
    let started_at = Utc::now();
    info!(step_id = %step.id, name = %step.name, action = ?step.action, "Executing step");

    // Human approval is handled separately — no timeout wrapper.
    if matches!(step.action, ActionType::HumanApproval) {
        return handle_human_approval(state, step, started_at).await;
    }

    let timeout = Duration::from_secs(step.timeout_secs.max(1));

    match tokio::time::timeout(timeout, dispatch_step(step)).await {
        Ok((output, error, success)) => StepResult {
            step_id: step.id.clone(),
            status: if success {
                StepStatus::Success
            } else {
                StepStatus::Failed
            },
            output,
            error,
            duration_ms: elapsed_ms(started_at),
            started_at: Some(started_at),
            completed_at: Some(Utc::now()),
        },
        Err(_) => {
            warn!(
                step_id = %step.id,
                timeout_secs = step.timeout_secs,
                "Step timed out"
            );
            timeout_result(step, started_at)
        }
    }
}

/// Dispatch to the correct action handler. Returns (output, error, success).
async fn dispatch_step(step: &RunbookStep) -> (Option<String>, Option<String>, bool) {
    match step.action {
        ActionType::ShellCommand => execute_shell(step).await,
        ActionType::ApiCall => execute_api_call(step).await,
        ActionType::CaveModuleAction => execute_cave_action(step).await,
        ActionType::Condition => execute_condition(step).await,
        ActionType::HumanApproval => unreachable!("handled before dispatch"),
    }
}

// ── Action implementations (simulated) ───────────────────────────────────────

async fn execute_shell(step: &RunbookStep) -> (Option<String>, Option<String>, bool) {
    let cmd = str_param(step, "command").unwrap_or("<no command>");
    info!(step_id = %step.id, command = %cmd, "Simulated shell_command");
    sleep(Duration::from_millis(100)).await;
    (
        Some(format!("[SIMULATED] $ {cmd}\nExit code: 0")),
        None,
        true,
    )
}

async fn execute_api_call(step: &RunbookStep) -> (Option<String>, Option<String>, bool) {
    let url = str_param(step, "url").unwrap_or("<no url>");
    let method = str_param(step, "method").unwrap_or("GET");
    info!(step_id = %step.id, method = %method, url = %url, "Simulated api_call");
    sleep(Duration::from_millis(200)).await;
    (
        Some(format!("[SIMULATED] {method} {url} → 200 OK")),
        None,
        true,
    )
}

async fn execute_cave_action(step: &RunbookStep) -> (Option<String>, Option<String>, bool) {
    let module = str_param(step, "module").unwrap_or("unknown");
    let action = str_param(step, "action").unwrap_or("unknown");
    info!(step_id = %step.id, module = %module, action = %action, "CAVE module action");
    sleep(Duration::from_millis(150)).await;

    let output = match (module, action) {
        ("cave-deploy", "rollback") => {
            let target = str_param(step, "target").unwrap_or("latest");
            format!("[SIMULATED] cave-deploy rollback → {target}: success")
        }
        ("cave-chaos", "stop") => {
            let experiment = str_param(step, "experiment").unwrap_or("all");
            format!("[SIMULATED] cave-chaos stop {experiment}: success")
        }
        ("cave-incidents", "update") => {
            let iid = str_param(step, "incident_id").unwrap_or("?");
            let status = str_param(step, "status").unwrap_or("resolved");
            format!("[SIMULATED] cave-incidents update {iid} → {status}")
        }
        ("cave-vault", "rotate") => {
            let secret = str_param(step, "secret").unwrap_or("all");
            format!("[SIMULATED] cave-vault rotate {secret}: success")
        }
        ("cave-certs", "renew") => {
            let domain = str_param(step, "domain").unwrap_or("*");
            format!("[SIMULATED] cave-certs renew {domain}: success")
        }
        _ => format!("[SIMULATED] {module} {action}: success"),
    };
    (Some(output), None, true)
}

async fn execute_condition(step: &RunbookStep) -> (Option<String>, Option<String>, bool) {
    let expr = str_param(step, "expression").unwrap_or("true");
    info!(step_id = %step.id, expression = %expr, "Evaluating condition");
    sleep(Duration::from_millis(10)).await;
    // Simulated — always passes; real implementation would evaluate against step context.
    (
        Some(format!("[SIMULATED] condition '{expr}' evaluated: true")),
        None,
        true,
    )
}

async fn handle_human_approval(
    state: Arc<RunbookState>,
    step: &RunbookStep,
    started_at: chrono::DateTime<Utc>,
) -> StepResult {
    let message = str_param(step, "message")
        .unwrap_or("Approval required to proceed")
        .to_string();

    let approval_id = Uuid::new_v4();
    let approval = ApprovalRequest {
        id: approval_id,
        execution_id: Uuid::nil(), // caller patches this after the fact
        step_id: step.id.clone(),
        message: message.clone(),
        status: ApprovalStatus::Pending,
        requested_at: Utc::now(),
        responded_at: None,
        responder: None,
    };

    info!(
        step_id = %step.id,
        approval_id = %approval_id,
        message = %message,
        "Human approval requested — execution paused"
    );

    {
        let mut approvals = state.approvals.lock().await;
        approvals.insert(approval_id, approval);
    }

    StepResult {
        step_id: step.id.clone(),
        status: StepStatus::PendingApproval,
        output: Some(format!(
            "Awaiting approval (id={approval_id}): {message}"
        )),
        error: None,
        duration_ms: elapsed_ms(started_at),
        started_at: Some(started_at),
        completed_at: Some(Utc::now()),
    }
}

// ── Failure handler ───────────────────────────────────────────────────────────

/// Apply `on_failure` policy to a failed result (for external callers).
pub fn handle_failure(on_failure: &OnFailure, result: StepResult) -> StepResult {
    match on_failure {
        OnFailure::Skip => StepResult {
            status: StepStatus::Skipped,
            ..result
        },
        OnFailure::Abort | OnFailure::Retry => result,
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

async fn store_execution(state: &RunbookState, execution: &RunbookExecution) {
    let mut execs = state.executions.lock().await;
    execs.insert(execution.id, execution.clone());
}

fn str_param<'a>(step: &'a RunbookStep, key: &str) -> Option<&'a str> {
    step.params.get(key)?.as_str()
}

fn elapsed_ms(started_at: chrono::DateTime<Utc>) -> u64 {
    (Utc::now() - started_at).num_milliseconds().max(0) as u64
}

fn timeout_result(step: &RunbookStep, started_at: chrono::DateTime<Utc>) -> StepResult {
    StepResult {
        step_id: step.id.clone(),
        status: StepStatus::Failed,
        output: None,
        error: Some(format!("Step timed out after {}s", step.timeout_secs)),
        duration_ms: step.timeout_secs * 1000,
        started_at: Some(started_at),
        completed_at: Some(Utc::now()),
    }
}
