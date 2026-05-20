// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Runbook execution engine — sequential, parallel, conditional steps.
use crate::models::*;
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

/// Create a new execution for a runbook.
pub fn create_execution(
    runbook: &Runbook,
    triggered_by: &str,
    trigger_type: TriggerType,
    parameters: HashMap<String, serde_json::Value>,
) -> Execution {
    let step_results = runbook
        .steps
        .iter()
        .map(|s| StepResult {
            step_id: s.id.clone(),
            step_name: s.name.clone(),
            status: StepStatus::Pending,
            output: None,
            error: None,
            logs: vec![],
            started_at: None,
            completed_at: None,
            retry_count: 0,
        })
        .collect();

    Execution {
        id: Uuid::new_v4(),
        runbook_id: runbook.id,
        runbook_name: runbook.name.clone(),
        status: ExecutionStatus::Running,
        triggered_by: triggered_by.to_string(),
        trigger_type,
        parameters,
        step_results,
        current_step: runbook.steps.first().map(|s| s.id.clone()),
        variables: HashMap::new(),
        started_at: Some(Utc::now()),
        completed_at: None,
        created_at: Utc::now(),
    }
}

/// Simulate executing a step (in production this would actually run the command/request/etc).
pub fn simulate_step(step: &Step, parameters: &HashMap<String, serde_json::Value>) -> StepResult {
    let now = Utc::now();
    let (status, output, error) = match &step.step_type {
        StepType::Shell { command, .. } => {
            let resolved = resolve_template(command, parameters);
            (
                StepStatus::Completed,
                Some(serde_json::json!({ "command": resolved, "exit_code": 0, "stdout": "OK" })),
                None,
            )
        }
        StepType::Http {
            url,
            method,
            expected_status,
            ..
        } => {
            let resolved_url = resolve_template(url, parameters);
            let status_code = expected_status.unwrap_or(200);
            (
                StepStatus::Completed,
                Some(
                    serde_json::json!({ "url": resolved_url, "method": method, "status": status_code }),
                ),
                None,
            )
        }
        StepType::KubernetesAction {
            action,
            resource_kind,
            resource_name,
            namespace,
            ..
        } => (
            StepStatus::Completed,
            Some(serde_json::json!({
                "action": format!("{:?}", action),
                "resource": format!("{}/{} in {}", resource_kind, resource_name, namespace)
            })),
            None,
        ),
        StepType::Notification {
            channel,
            message,
            recipients,
        } => {
            let resolved_msg = resolve_template(message, parameters);
            (
                StepStatus::Completed,
                Some(serde_json::json!({
                    "channel": format!("{:?}", channel),
                    "message": resolved_msg,
                    "recipients": recipients
                })),
                None,
            )
        }
        StepType::Wait {
            duration_seconds,
            message,
        } => (
            StepStatus::Completed,
            Some(serde_json::json!({
                "waited_seconds": duration_seconds,
                "message": message
            })),
            None,
        ),
        StepType::ManualApproval {
            message, approvers, ..
        } => (
            StepStatus::WaitingForApproval,
            Some(serde_json::json!({
                "message": message,
                "approvers": approvers
            })),
            None,
        ),
        StepType::SetVariable { name, value } => (
            StepStatus::Completed,
            Some(serde_json::json!({ "variable": name, "value": value })),
            None,
        ),
        StepType::Conditional { condition, .. } => (
            StepStatus::Completed,
            Some(serde_json::json!({ "condition": condition, "evaluated": true })),
            None,
        ),
    };

    StepResult {
        step_id: step.id.clone(),
        step_name: step.name.clone(),
        status,
        output,
        error,
        logs: vec![format!(
            "[{}] Step '{}' executed",
            now.to_rfc3339(),
            step.name
        )],
        started_at: Some(now),
        completed_at: Some(Utc::now()),
        retry_count: 0,
    }
}

/// Execute all non-approval steps of a runbook sequentially (simulation).
pub fn run_execution(runbook: &Runbook, execution: &mut Execution) {
    for step in &runbook.steps {
        if !evaluate_condition(step.condition.as_deref(), &execution.variables) {
            let result = execution
                .step_results
                .iter_mut()
                .find(|r| r.step_id == step.id);
            if let Some(r) = result {
                r.status = StepStatus::Skipped;
            }
            continue;
        }

        let result = simulate_step(step, &execution.parameters);
        if result.status == StepStatus::WaitingForApproval {
            execution.status = ExecutionStatus::WaitingForApproval;
            execution.current_step = Some(step.id.clone());
        }

        if let Some(r) = execution
            .step_results
            .iter_mut()
            .find(|r| r.step_id == step.id)
        {
            *r = result.clone();
        }

        if result.status == StepStatus::Failed && step.on_failure == FailureAction::Stop {
            execution.status = ExecutionStatus::Failed;
            execution.completed_at = Some(Utc::now());
            return;
        }
        if result.status == StepStatus::WaitingForApproval {
            return;
        }
    }

    if execution.status == ExecutionStatus::Running {
        execution.status = ExecutionStatus::Completed;
        execution.completed_at = Some(Utc::now());
    }
}

fn resolve_template(template: &str, params: &HashMap<String, serde_json::Value>) -> String {
    let mut result = template.to_string();
    for (key, value) in params {
        let placeholder = format!("{{{{{}}}}}", key);
        let replacement = match value {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        result = result.replace(&placeholder, &replacement);
    }
    result
}

fn evaluate_condition(
    condition: Option<&str>,
    _variables: &HashMap<String, serde_json::Value>,
) -> bool {
    condition.map_or(true, |c| c != "false")
}

/// Cancel an execution.
pub fn cancel_execution(execution: &mut Execution) {
    execution.status = ExecutionStatus::Cancelled;
    execution.completed_at = Some(Utc::now());
    for step in execution.step_results.iter_mut() {
        if step.status == StepStatus::Pending {
            step.status = StepStatus::Skipped;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::builtin_templates;

    #[test]
    fn test_create_execution() {
        let templates = builtin_templates();
        let runbook = &templates[0];
        let exec = create_execution(runbook, "admin", TriggerType::Manual, HashMap::new());
        assert_eq!(exec.status, ExecutionStatus::Running);
        assert_eq!(exec.step_results.len(), runbook.steps.len());
    }

    #[test]
    fn test_run_execution() {
        let templates = builtin_templates();
        let runbook = templates
            .into_iter()
            .find(|r| r.name == "Pod Restart")
            .unwrap_or_else(|| Runbook {
                id: Uuid::new_v4(),
                name: "Test".to_string(),
                description: "".to_string(),
                version: "1.0".to_string(),
                tags: vec![],
                parameters: vec![],
                steps: vec![Step {
                    id: "step-1".to_string(),
                    name: "Wait".to_string(),
                    step_type: StepType::Wait {
                        duration_seconds: 1,
                        message: None,
                    },
                    description: "".to_string(),
                    condition: None,
                    depends_on: vec![],
                    timeout_seconds: 30,
                    retry_count: 0,
                    continue_on_error: false,
                    on_failure: FailureAction::Stop,
                }],
                timeout_seconds: 300,
                on_failure: FailureAction::Stop,
                is_template: false,
                created_by: "system".to_string(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            });
        let mut exec = create_execution(&runbook, "test", TriggerType::Manual, HashMap::new());
        run_execution(&runbook, &mut exec);
        assert!(
            exec.status == ExecutionStatus::Completed
                || exec.status == ExecutionStatus::WaitingForApproval
        );
    }

    #[test]
    fn test_resolve_template() {
        let mut params = HashMap::new();
        params.insert("namespace".to_string(), serde_json::json!("production"));
        let result = resolve_template("kubectl get pods -n {{namespace}}", &params);
        assert_eq!(result, "kubectl get pods -n production");
    }

    #[test]
    fn test_cancel_execution() {
        let templates = builtin_templates();
        let runbook = &templates[0];
        let mut exec = create_execution(runbook, "admin", TriggerType::Manual, HashMap::new());
        cancel_execution(&mut exec);
        assert_eq!(exec.status, ExecutionStatus::Cancelled);
    }
}
