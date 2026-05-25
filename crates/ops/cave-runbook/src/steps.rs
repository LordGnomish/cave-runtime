// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Step type validation and metadata.
use crate::models::{Step, StepType};

/// Validate a step's configuration.
pub fn validate_step(step: &Step) -> Vec<String> {
    let mut errors = Vec::new();
    if step.id.is_empty() {
        errors.push("step id is required".to_string());
    }
    if step.name.is_empty() {
        errors.push("step name is required".to_string());
    }
    match &step.step_type {
        StepType::Shell { command, .. } => {
            if command.is_empty() {
                errors.push(format!("step {}: shell command is empty", step.id));
            }
        }
        StepType::Http { url, method, .. } => {
            if url.is_empty() {
                errors.push(format!("step {}: HTTP URL is empty", step.id));
            }
            if method.is_empty() {
                errors.push(format!("step {}: HTTP method is empty", step.id));
            }
        }
        StepType::KubernetesAction {
            resource_kind,
            resource_name,
            ..
        } => {
            if resource_kind.is_empty() {
                errors.push(format!("step {}: resource_kind is empty", step.id));
            }
            if resource_name.is_empty() {
                errors.push(format!("step {}: resource_name is empty", step.id));
            }
        }
        StepType::ManualApproval { approvers, .. } => {
            if approvers.is_empty() {
                errors.push(format!(
                    "step {}: approval step requires at least one approver",
                    step.id
                ));
            }
        }
        _ => {}
    }
    errors
}

/// Get the human-readable step type name.
pub fn step_type_name(step: &Step) -> &'static str {
    match &step.step_type {
        StepType::Shell { .. } => "Shell Command",
        StepType::Http { .. } => "HTTP Request",
        StepType::KubernetesAction { .. } => "Kubernetes Action",
        StepType::Notification { .. } => "Notification",
        StepType::Wait { .. } => "Wait",
        StepType::ManualApproval { .. } => "Manual Approval",
        StepType::SetVariable { .. } => "Set Variable",
        StepType::Conditional { .. } => "Conditional",
    }
}

/// Check if a step requires human interaction.
pub fn requires_human_interaction(step: &Step) -> bool {
    matches!(&step.step_type, StepType::ManualApproval { .. })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::*;
    use std::collections::HashMap;

    fn make_shell_step(id: &str, cmd: &str) -> Step {
        Step {
            id: id.to_string(),
            name: "Test Step".to_string(),
            step_type: StepType::Shell {
                command: cmd.to_string(),
                working_dir: None,
                env: HashMap::new(),
            },
            description: "".to_string(),
            condition: None,
            depends_on: vec![],
            timeout_seconds: 30,
            retry_count: 0,
            continue_on_error: false,
            on_failure: FailureAction::Stop,
        }
    }

    #[test]
    fn test_validate_valid_step() {
        let step = make_shell_step("step-1", "echo hello");
        assert!(validate_step(&step).is_empty());
    }

    #[test]
    fn test_validate_empty_command() {
        let step = make_shell_step("step-1", "");
        let errors = validate_step(&step);
        assert!(!errors.is_empty());
    }

    #[test]
    fn test_step_type_name() {
        let step = make_shell_step("s1", "ls");
        assert_eq!(step_type_name(&step), "Shell Command");
    }

    #[test]
    fn test_requires_human_interaction() {
        let step = make_shell_step("s1", "ls");
        assert!(!requires_human_interaction(&step));
        let approval_step = Step {
            id: "approval".to_string(),
            name: "Approval".to_string(),
            step_type: StepType::ManualApproval {
                message: "Approve?".to_string(),
                approvers: vec!["admin".to_string()],
                timeout_seconds: 3600,
            },
            description: "".to_string(),
            condition: None,
            depends_on: vec![],
            timeout_seconds: 3600,
            retry_count: 0,
            continue_on_error: false,
            on_failure: FailureAction::Stop,
        };
        assert!(requires_human_interaction(&approval_step));
    }
}
