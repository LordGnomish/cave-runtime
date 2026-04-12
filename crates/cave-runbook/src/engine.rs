//! Runbook execution engine.

use crate::models::{
    CronSchedule, Execution, ExecutionStatus, FailureAction, RunbookStep, Runbook,
    StepExecution, StepType,
};
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

pub struct RunbookEngine;

impl RunbookEngine {
    /// Create a new execution record for the given runbook.
    pub fn create_execution(
        runbook: &Runbook,
        triggered_by: Uuid,
        params: HashMap<String, serde_json::Value>,
    ) -> Execution {
        Execution {
            id: Uuid::new_v4(),
            runbook_id: runbook.id,
            runbook_name: runbook.name.clone(),
            status: ExecutionStatus::Pending,
            triggered_by,
            parameters: params,
            step_executions: vec![],
            started_at: Utc::now(),
            completed_at: None,
            error: None,
        }
    }

    /// Simulate a single step execution (no real shell/HTTP calls).
    pub fn simulate_step(step: &RunbookStep) -> StepExecution {
        let now = Utc::now();
        match step.step_type {
            StepType::Command => StepExecution {
                step_id: step.id,
                step_name: step.name.clone(),
                status: ExecutionStatus::Completed,
                started_at: Some(now),
                completed_at: Some(now),
                exit_code: Some(0),
                stdout: format!(
                    "Simulated command execution for step '{}'",
                    step.name
                ),
                stderr: String::new(),
                error: None,
                retries: 0,
            },
            StepType::Script => StepExecution {
                step_id: step.id,
                step_name: step.name.clone(),
                status: ExecutionStatus::Completed,
                started_at: Some(now),
                completed_at: Some(now),
                exit_code: Some(0),
                stdout: "Script executed successfully".to_string(),
                stderr: String::new(),
                error: None,
                retries: 0,
            },
            StepType::HttpRequest => StepExecution {
                step_id: step.id,
                step_name: step.name.clone(),
                status: ExecutionStatus::Completed,
                started_at: Some(now),
                completed_at: Some(now),
                exit_code: Some(0),
                stdout: "HTTP 200 OK".to_string(),
                stderr: String::new(),
                error: None,
                retries: 0,
            },
            StepType::ApprovalGate => StepExecution {
                step_id: step.id,
                step_name: step.name.clone(),
                status: ExecutionStatus::WaitingApproval,
                started_at: Some(now),
                completed_at: None,
                exit_code: None,
                stdout: String::new(),
                stderr: String::new(),
                error: None,
                retries: 0,
            },
            StepType::ConditionalBranch => StepExecution {
                step_id: step.id,
                step_name: step.name.clone(),
                status: ExecutionStatus::Completed,
                started_at: Some(now),
                completed_at: Some(now),
                exit_code: Some(0),
                stdout: "Condition evaluated: true".to_string(),
                stderr: String::new(),
                error: None,
                retries: 0,
            },
        }
    }

    /// Execute all steps sequentially, respecting on_failure actions.
    pub fn execute_sequential(execution: &mut Execution, runbook: &Runbook) {
        execution.status = ExecutionStatus::Running;

        if runbook.steps.is_empty() {
            execution.status = ExecutionStatus::Completed;
            execution.completed_at = Some(Utc::now());
            return;
        }

        let mut overall_failed = false;

        for step in &runbook.steps {
            let mut step_exec = Self::simulate_step(step);

            // Handle WaitingApproval — halt sequential execution
            if step_exec.status == ExecutionStatus::WaitingApproval {
                execution.step_executions.push(step_exec);
                execution.status = ExecutionStatus::WaitingApproval;
                return;
            }

            // Simulate retry logic
            if step_exec.status == ExecutionStatus::Failed {
                match &step.on_failure {
                    FailureAction::Retry { max_retries } => {
                        let mut retries = 0u8;
                        while retries < *max_retries && step_exec.status == ExecutionStatus::Failed {
                            retries += 1;
                            step_exec = Self::simulate_step(step);
                            step_exec.retries = retries;
                        }
                        if step_exec.status == ExecutionStatus::Failed {
                            overall_failed = true;
                        }
                    }
                    FailureAction::Abort => {
                        overall_failed = true;
                        execution.step_executions.push(step_exec);
                        break;
                    }
                    FailureAction::Continue => {
                        // Log the failure but keep going
                    }
                }
            }

            execution.step_executions.push(step_exec);
        }

        execution.completed_at = Some(Utc::now());
        execution.status = if overall_failed {
            ExecutionStatus::Failed
        } else {
            ExecutionStatus::Completed
        };
    }

    /// Identify groups of steps that can run in parallel.
    /// Steps with an empty `depends_on` list can run concurrently with others in the same group.
    /// Steps with dependencies must be in a later group.
    pub fn identify_parallel_groups<'a>(runbook: &'a Runbook) -> Vec<Vec<&'a RunbookStep>> {
        let mut groups: Vec<Vec<&'a RunbookStep>> = vec![];
        let mut completed_ids: Vec<Uuid> = vec![];

        let mut remaining: Vec<&RunbookStep> = runbook.steps.iter().collect();

        while !remaining.is_empty() {
            let mut group: Vec<&RunbookStep> = vec![];
            let mut next_remaining: Vec<&RunbookStep> = vec![];

            for step in &remaining {
                let deps_met = step
                    .depends_on
                    .iter()
                    .all(|dep_id| completed_ids.contains(dep_id));
                if deps_met {
                    group.push(step);
                } else {
                    next_remaining.push(step);
                }
            }

            if group.is_empty() {
                // Break cycles — push remaining as-is to avoid infinite loop
                break;
            }

            for s in &group {
                completed_ids.push(s.id);
            }
            groups.push(group);
            remaining = next_remaining;
        }

        groups
    }

    /// Validate that all required parameters are provided and allowed values respected.
    pub fn validate_parameters(
        runbook: &Runbook,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<(), Vec<String>> {
        let mut errors: Vec<String> = vec![];

        for param in &runbook.parameters {
            match params.get(&param.name) {
                None => {
                    if param.required && param.default_value.is_none() {
                        errors.push(format!("Required parameter '{}' is missing", param.name));
                    }
                }
                Some(value) => {
                    if param.param_type == crate::models::ParamType::Select
                        && !param.allowed_values.is_empty()
                    {
                        let val_str = match value {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        if !param.allowed_values.contains(&val_str) {
                            errors.push(format!(
                                "Parameter '{}' value '{}' is not in allowed values: {:?}",
                                param.name, val_str, param.allowed_values
                            ));
                        }
                    }
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Check whether a cron schedule is due.
    /// Simplified implementation: always returns true for in-memory testing.
    pub fn is_schedule_due(_schedule: &CronSchedule) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        FailureAction, ParamType, RunbookParameter, RunbookStep, StepType,
    };

    fn make_step(name: &str, step_type: StepType) -> RunbookStep {
        RunbookStep {
            id: Uuid::new_v4(),
            name: name.to_string(),
            description: None,
            step_type,
            step_config: serde_json::Value::Null,
            on_failure: FailureAction::Abort,
            depends_on: vec![],
        }
    }

    fn make_runbook(steps: Vec<RunbookStep>) -> Runbook {
        Runbook {
            id: Uuid::new_v4(),
            name: "Test Runbook".to_string(),
            description: "A test runbook".to_string(),
            steps,
            parameters: vec![],
            schedule: None,
            access_control: vec![],
            notifications: crate::models::RunbookNotifications::default(),
            timeout_seconds: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            created_by: Uuid::new_v4(),
            enabled: true,
        }
    }

    #[test]
    fn test_create_execution() {
        let runbook = make_runbook(vec![]);
        let user_id = Uuid::new_v4();
        let exec = RunbookEngine::create_execution(&runbook, user_id, HashMap::new());
        assert_eq!(exec.runbook_id, runbook.id);
        assert_eq!(exec.triggered_by, user_id);
        assert_eq!(exec.status, ExecutionStatus::Pending);
    }

    #[test]
    fn test_simulate_command_step() {
        let step = make_step("deploy", StepType::Command);
        let result = RunbookEngine::simulate_step(&step);
        assert_eq!(result.status, ExecutionStatus::Completed);
        assert_eq!(result.exit_code, Some(0));
        assert!(!result.stdout.is_empty());
    }

    #[test]
    fn test_simulate_script_step() {
        let step = make_step("run_script", StepType::Script);
        let result = RunbookEngine::simulate_step(&step);
        assert_eq!(result.status, ExecutionStatus::Completed);
        assert_eq!(result.stdout, "Script executed successfully");
    }

    #[test]
    fn test_simulate_http_step() {
        let step = make_step("call_api", StepType::HttpRequest);
        let result = RunbookEngine::simulate_step(&step);
        assert_eq!(result.status, ExecutionStatus::Completed);
        assert_eq!(result.stdout, "HTTP 200 OK");
    }

    #[test]
    fn test_simulate_approval_step() {
        let step = make_step("wait_approval", StepType::ApprovalGate);
        let result = RunbookEngine::simulate_step(&step);
        assert_eq!(result.status, ExecutionStatus::WaitingApproval);
        assert!(result.completed_at.is_none());
    }

    #[test]
    fn test_simulate_branch_step() {
        let step = make_step("branch", StepType::ConditionalBranch);
        let result = RunbookEngine::simulate_step(&step);
        assert_eq!(result.status, ExecutionStatus::Completed);
        assert!(result.stdout.contains("Condition evaluated"));
    }

    #[test]
    fn test_execute_sequential_empty_runbook() {
        let runbook = make_runbook(vec![]);
        let mut exec =
            RunbookEngine::create_execution(&runbook, Uuid::new_v4(), HashMap::new());
        RunbookEngine::execute_sequential(&mut exec, &runbook);
        assert_eq!(exec.status, ExecutionStatus::Completed);
        assert!(exec.completed_at.is_some());
    }

    #[test]
    fn test_execute_sequential_all_steps_complete() {
        let steps = vec![
            make_step("step1", StepType::Command),
            make_step("step2", StepType::Script),
            make_step("step3", StepType::HttpRequest),
        ];
        let runbook = make_runbook(steps);
        let mut exec =
            RunbookEngine::create_execution(&runbook, Uuid::new_v4(), HashMap::new());
        RunbookEngine::execute_sequential(&mut exec, &runbook);
        assert_eq!(exec.status, ExecutionStatus::Completed);
        assert_eq!(exec.step_executions.len(), 3);
    }

    #[test]
    fn test_execute_sequential_approval_gate_pauses() {
        let steps = vec![
            make_step("step1", StepType::Command),
            make_step("gate", StepType::ApprovalGate),
            make_step("step3", StepType::Command),
        ];
        let runbook = make_runbook(steps);
        let mut exec =
            RunbookEngine::create_execution(&runbook, Uuid::new_v4(), HashMap::new());
        RunbookEngine::execute_sequential(&mut exec, &runbook);
        assert_eq!(exec.status, ExecutionStatus::WaitingApproval);
        // Only 2 steps pushed (step1 + gate), step3 not reached
        assert_eq!(exec.step_executions.len(), 2);
    }

    #[test]
    fn test_parallel_group_identification_no_deps() {
        let steps = vec![
            make_step("a", StepType::Command),
            make_step("b", StepType::Command),
            make_step("c", StepType::Command),
        ];
        let runbook = make_runbook(steps);
        let groups = RunbookEngine::identify_parallel_groups(&runbook);
        // All have no deps → single group with 3 steps
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 3);
    }

    #[test]
    fn test_parallel_group_identification_with_deps() {
        let step_a = make_step("a", StepType::Command);
        let step_b_id = Uuid::new_v4();
        let step_b = RunbookStep {
            id: step_b_id,
            depends_on: vec![step_a.id],
            ..make_step("b", StepType::Command)
        };
        let runbook = make_runbook(vec![step_a, step_b]);
        let groups = RunbookEngine::identify_parallel_groups(&runbook);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].len(), 1); // a
        assert_eq!(groups[1].len(), 1); // b (depends on a)
    }

    #[test]
    fn test_validate_parameters_required_missing() {
        let runbook = {
            let mut r = make_runbook(vec![]);
            r.parameters.push(RunbookParameter {
                name: "env".to_string(),
                description: "Target environment".to_string(),
                param_type: ParamType::String,
                required: true,
                default_value: None,
                allowed_values: vec![],
            });
            r
        };
        let result = RunbookEngine::validate_parameters(&runbook, &HashMap::new());
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| e.contains("env")));
    }

    #[test]
    fn test_validate_parameters_required_with_default_ok() {
        let runbook = {
            let mut r = make_runbook(vec![]);
            r.parameters.push(RunbookParameter {
                name: "env".to_string(),
                description: "Target environment".to_string(),
                param_type: ParamType::String,
                required: true,
                default_value: Some("prod".to_string()),
                allowed_values: vec![],
            });
            r
        };
        let result = RunbookEngine::validate_parameters(&runbook, &HashMap::new());
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_parameters_select_invalid_value() {
        let runbook = {
            let mut r = make_runbook(vec![]);
            r.parameters.push(RunbookParameter {
                name: "region".to_string(),
                description: "Target region".to_string(),
                param_type: ParamType::Select,
                required: true,
                default_value: None,
                allowed_values: vec!["us-east-1".to_string(), "eu-west-1".to_string()],
            });
            r
        };
        let mut params = HashMap::new();
        params.insert(
            "region".to_string(),
            serde_json::Value::String("ap-southeast-1".to_string()),
        );
        let result = RunbookEngine::validate_parameters(&runbook, &params);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_parameters_select_valid_value() {
        let runbook = {
            let mut r = make_runbook(vec![]);
            r.parameters.push(RunbookParameter {
                name: "region".to_string(),
                description: "Target region".to_string(),
                param_type: ParamType::Select,
                required: true,
                default_value: None,
                allowed_values: vec!["us-east-1".to_string(), "eu-west-1".to_string()],
            });
            r
        };
        let mut params = HashMap::new();
        params.insert(
            "region".to_string(),
            serde_json::Value::String("us-east-1".to_string()),
        );
        let result = RunbookEngine::validate_parameters(&runbook, &params);
        assert!(result.is_ok());
    }

    #[test]
    fn test_is_schedule_due() {
        let schedule = CronSchedule {
            expression: "0 * * * *".to_string(),
            timezone: "UTC".to_string(),
            enabled: true,
            last_run: None,
        };
        assert!(RunbookEngine::is_schedule_due(&schedule));
    }
}
