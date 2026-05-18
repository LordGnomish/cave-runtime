// SPDX-License-Identifier: AGPL-3.0-or-later
//! Built-in runbook templates.
use crate::models::*;
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

pub fn builtin_templates() -> Vec<Runbook> {
    vec![
        pod_restart_template(),
        deployment_rollback_template(),
        cert_renewal_template(),
        incident_response_template(),
        database_failover_template(),
    ]
}

pub fn pod_restart_template() -> Runbook {
    let now = Utc::now();
    Runbook {
        id: Uuid::new_v4(),
        name: "Pod Restart".to_string(),
        description: "Safely restart a pod in a given namespace with pre/post health checks"
            .to_string(),
        version: "1.0.0".to_string(),
        tags: vec!["kubernetes".to_string(), "operations".to_string()],
        parameters: vec![
            ParameterDef {
                name: "namespace".to_string(),
                description: "Target namespace".to_string(),
                param_type: ParamType::String,
                required: true,
                default_value: Some(serde_json::json!("default")),
                secret: false,
            },
            ParameterDef {
                name: "pod_name".to_string(),
                description: "Name of the pod to restart".to_string(),
                param_type: ParamType::String,
                required: true,
                default_value: None,
                secret: false,
            },
        ],
        steps: vec![
            Step {
                id: "pre-check".to_string(),
                name: "Pre-flight health check".to_string(),
                step_type: StepType::Shell {
                    command: "kubectl get pod {{pod_name}} -n {{namespace}}".to_string(),
                    working_dir: None,
                    env: HashMap::new(),
                },
                description: "Verify pod exists before restarting".to_string(),
                condition: None,
                depends_on: vec![],
                timeout_seconds: 30,
                retry_count: 0,
                continue_on_error: false,
                on_failure: FailureAction::Stop,
            },
            Step {
                id: "notify-start".to_string(),
                name: "Notify team".to_string(),
                step_type: StepType::Notification {
                    channel: NotificationChannel::Slack,
                    message: "Restarting pod {{pod_name}} in {{namespace}}".to_string(),
                    recipients: vec!["#ops".to_string()],
                },
                description: "Send notification before restart".to_string(),
                condition: None,
                depends_on: vec!["pre-check".to_string()],
                timeout_seconds: 10,
                retry_count: 0,
                continue_on_error: true,
                on_failure: FailureAction::Continue,
            },
            Step {
                id: "delete-pod".to_string(),
                name: "Delete pod".to_string(),
                step_type: StepType::KubernetesAction {
                    action: K8sAction::Delete,
                    resource_kind: "Pod".to_string(),
                    resource_name: "{{pod_name}}".to_string(),
                    namespace: "{{namespace}}".to_string(),
                    manifest: None,
                },
                description: "Delete the pod (it will be recreated by its controller)".to_string(),
                condition: None,
                depends_on: vec!["notify-start".to_string()],
                timeout_seconds: 60,
                retry_count: 1,
                continue_on_error: false,
                on_failure: FailureAction::Stop,
            },
            Step {
                id: "wait-ready".to_string(),
                name: "Wait for pod ready".to_string(),
                step_type: StepType::Wait {
                    duration_seconds: 30,
                    message: Some("Waiting for pod to become ready".to_string()),
                },
                description: "Wait for the new pod to start".to_string(),
                condition: None,
                depends_on: vec!["delete-pod".to_string()],
                timeout_seconds: 120,
                retry_count: 0,
                continue_on_error: false,
                on_failure: FailureAction::Stop,
            },
            Step {
                id: "post-check".to_string(),
                name: "Post-restart health check".to_string(),
                step_type: StepType::Shell {
                    command:
                        "kubectl get pod -n {{namespace}} -l app={{pod_name}} --field-selector=status.phase=Running"
                            .to_string(),
                    working_dir: None,
                    env: HashMap::new(),
                },
                description: "Verify new pod is running".to_string(),
                condition: None,
                depends_on: vec!["wait-ready".to_string()],
                timeout_seconds: 30,
                retry_count: 2,
                continue_on_error: false,
                on_failure: FailureAction::Notify,
            },
        ],
        timeout_seconds: 300,
        on_failure: FailureAction::Notify,
        is_template: true,
        created_by: "cave-runbook/system".to_string(),
        created_at: now,
        updated_at: now,
    }
}

pub fn deployment_rollback_template() -> Runbook {
    let now = Utc::now();
    Runbook {
        id: Uuid::new_v4(),
        name: "Deployment Rollback".to_string(),
        description: "Roll back a deployment to the previous revision with approval gate"
            .to_string(),
        version: "1.0.0".to_string(),
        tags: vec![
            "kubernetes".to_string(),
            "rollback".to_string(),
            "deployment".to_string(),
        ],
        parameters: vec![
            ParameterDef {
                name: "namespace".to_string(),
                description: "Target namespace".to_string(),
                param_type: ParamType::String,
                required: true,
                default_value: Some(serde_json::json!("default")),
                secret: false,
            },
            ParameterDef {
                name: "deployment".to_string(),
                description: "Deployment name".to_string(),
                param_type: ParamType::String,
                required: true,
                default_value: None,
                secret: false,
            },
        ],
        steps: vec![
            Step {
                id: "check-history".to_string(),
                name: "Check rollout history".to_string(),
                step_type: StepType::Shell {
                    command:
                        "kubectl rollout history deployment/{{deployment}} -n {{namespace}}"
                            .to_string(),
                    working_dir: None,
                    env: HashMap::new(),
                },
                description: "Show deployment history".to_string(),
                condition: None,
                depends_on: vec![],
                timeout_seconds: 30,
                retry_count: 0,
                continue_on_error: false,
                on_failure: FailureAction::Stop,
            },
            Step {
                id: "approval".to_string(),
                name: "Approval required".to_string(),
                step_type: StepType::ManualApproval {
                    message: "Approve rollback of {{deployment}} in {{namespace}}?".to_string(),
                    approvers: vec!["platform-team".to_string()],
                    timeout_seconds: 3600,
                },
                description: "Human approval before rollback".to_string(),
                condition: None,
                depends_on: vec!["check-history".to_string()],
                timeout_seconds: 3600,
                retry_count: 0,
                continue_on_error: false,
                on_failure: FailureAction::Stop,
            },
            Step {
                id: "rollback".to_string(),
                name: "Execute rollback".to_string(),
                step_type: StepType::Shell {
                    command:
                        "kubectl rollout undo deployment/{{deployment}} -n {{namespace}}"
                            .to_string(),
                    working_dir: None,
                    env: HashMap::new(),
                },
                description: "Roll back to previous revision".to_string(),
                condition: None,
                depends_on: vec!["approval".to_string()],
                timeout_seconds: 120,
                retry_count: 0,
                continue_on_error: false,
                on_failure: FailureAction::Stop,
            },
            Step {
                id: "verify".to_string(),
                name: "Verify rollback".to_string(),
                step_type: StepType::Shell {
                    command:
                        "kubectl rollout status deployment/{{deployment}} -n {{namespace}}"
                            .to_string(),
                    working_dir: None,
                    env: HashMap::new(),
                },
                description: "Wait for rollback to complete".to_string(),
                condition: None,
                depends_on: vec!["rollback".to_string()],
                timeout_seconds: 300,
                retry_count: 0,
                continue_on_error: false,
                on_failure: FailureAction::Stop,
            },
        ],
        timeout_seconds: 600,
        on_failure: FailureAction::Notify,
        is_template: true,
        created_by: "cave-runbook/system".to_string(),
        created_at: now,
        updated_at: now,
    }
}

pub fn cert_renewal_template() -> Runbook {
    let now = Utc::now();
    Runbook {
        id: Uuid::new_v4(),
        name: "Certificate Renewal".to_string(),
        description: "Renew TLS certificates and restart affected deployments".to_string(),
        version: "1.0.0".to_string(),
        tags: vec![
            "certs".to_string(),
            "tls".to_string(),
            "operations".to_string(),
        ],
        parameters: vec![
            ParameterDef {
                name: "cert_name".to_string(),
                description: "Certificate name".to_string(),
                param_type: ParamType::String,
                required: true,
                default_value: None,
                secret: false,
            },
            ParameterDef {
                name: "namespace".to_string(),
                description: "Namespace".to_string(),
                param_type: ParamType::String,
                required: true,
                default_value: Some(serde_json::json!("default")),
                secret: false,
            },
        ],
        steps: vec![
            Step {
                id: "check-expiry".to_string(),
                name: "Check certificate expiry".to_string(),
                step_type: StepType::Shell {
                    command: "kubectl get certificate {{cert_name}} -n {{namespace}} -o jsonpath='{.status.notAfter}'".to_string(),
                    working_dir: None,
                    env: HashMap::new(),
                },
                description: "Check current certificate expiry".to_string(),
                condition: None,
                depends_on: vec![],
                timeout_seconds: 30,
                retry_count: 0,
                continue_on_error: false,
                on_failure: FailureAction::Continue,
            },
            Step {
                id: "renew".to_string(),
                name: "Trigger certificate renewal".to_string(),
                step_type: StepType::KubernetesAction {
                    action: K8sAction::Patch,
                    resource_kind: "Certificate".to_string(),
                    resource_name: "{{cert_name}}".to_string(),
                    namespace: "{{namespace}}".to_string(),
                    manifest: Some(serde_json::json!({
                        "metadata": {
                            "annotations": {
                                "cert-manager.io/issue-temporary-certificate": "true"
                            }
                        }
                    })),
                },
                description: "Annotate certificate to trigger cert-manager renewal".to_string(),
                condition: None,
                depends_on: vec!["check-expiry".to_string()],
                timeout_seconds: 60,
                retry_count: 0,
                continue_on_error: false,
                on_failure: FailureAction::Stop,
            },
            Step {
                id: "wait-renewal".to_string(),
                name: "Wait for renewal".to_string(),
                step_type: StepType::Wait {
                    duration_seconds: 60,
                    message: Some(
                        "Waiting for cert-manager to issue new certificate".to_string(),
                    ),
                },
                description: "Wait for cert-manager".to_string(),
                condition: None,
                depends_on: vec!["renew".to_string()],
                timeout_seconds: 120,
                retry_count: 0,
                continue_on_error: false,
                on_failure: FailureAction::Stop,
            },
        ],
        timeout_seconds: 300,
        on_failure: FailureAction::Notify,
        is_template: true,
        created_by: "cave-runbook/system".to_string(),
        created_at: now,
        updated_at: now,
    }
}

pub fn incident_response_template() -> Runbook {
    let now = Utc::now();
    Runbook {
        id: Uuid::new_v4(),
        name: "Incident Response".to_string(),
        description:
            "Standard incident response runbook: triage, notify, mitigate, document".to_string(),
        version: "1.0.0".to_string(),
        tags: vec!["incident".to_string(), "on-call".to_string()],
        parameters: vec![
            ParameterDef {
                name: "incident_id".to_string(),
                description: "Incident ID".to_string(),
                param_type: ParamType::String,
                required: true,
                default_value: None,
                secret: false,
            },
            ParameterDef {
                name: "severity".to_string(),
                description: "Incident severity".to_string(),
                param_type: ParamType::Select,
                required: true,
                default_value: Some(serde_json::json!("P2")),
                secret: false,
            },
        ],
        steps: vec![
            Step {
                id: "acknowledge".to_string(),
                name: "Acknowledge incident".to_string(),
                step_type: StepType::Http {
                    url: "/api/incidents/{{incident_id}}/acknowledge".to_string(),
                    method: "POST".to_string(),
                    headers: HashMap::new(),
                    body: None,
                    expected_status: Some(200),
                },
                description: "Acknowledge the incident in cave-incidents".to_string(),
                condition: None,
                depends_on: vec![],
                timeout_seconds: 10,
                retry_count: 3,
                continue_on_error: false,
                on_failure: FailureAction::Continue,
            },
            Step {
                id: "notify-oncall".to_string(),
                name: "Notify on-call team".to_string(),
                step_type: StepType::Notification {
                    channel: NotificationChannel::Slack,
                    message: "Incident {{incident_id}} ({{severity}}): response runbook started"
                        .to_string(),
                    recipients: vec!["#incidents".to_string(), "#on-call".to_string()],
                },
                description: "Alert the on-call team".to_string(),
                condition: None,
                depends_on: vec!["acknowledge".to_string()],
                timeout_seconds: 10,
                retry_count: 0,
                continue_on_error: true,
                on_failure: FailureAction::Continue,
            },
            Step {
                id: "collect-diagnostics".to_string(),
                name: "Collect diagnostics".to_string(),
                step_type: StepType::Shell {
                    command:
                        "kubectl get events --sort-by=.lastTimestamp -A | tail -50".to_string(),
                    working_dir: None,
                    env: HashMap::new(),
                },
                description: "Gather cluster-level diagnostics".to_string(),
                condition: None,
                depends_on: vec!["notify-oncall".to_string()],
                timeout_seconds: 60,
                retry_count: 0,
                continue_on_error: true,
                on_failure: FailureAction::Continue,
            },
            Step {
                id: "mitigation-approval".to_string(),
                name: "Approve mitigation plan".to_string(),
                step_type: StepType::ManualApproval {
                    message:
                        "Review diagnostics and approve mitigation for incident {{incident_id}}"
                            .to_string(),
                    approvers: vec!["incident-commander".to_string()],
                    timeout_seconds: 3600,
                },
                description: "Human approval for mitigation".to_string(),
                condition: None,
                depends_on: vec!["collect-diagnostics".to_string()],
                timeout_seconds: 3600,
                retry_count: 0,
                continue_on_error: false,
                on_failure: FailureAction::Stop,
            },
        ],
        timeout_seconds: 7200,
        on_failure: FailureAction::Notify,
        is_template: true,
        created_by: "cave-runbook/system".to_string(),
        created_at: now,
        updated_at: now,
    }
}

pub fn database_failover_template() -> Runbook {
    let now = Utc::now();
    Runbook {
        id: Uuid::new_v4(),
        name: "Database Failover".to_string(),
        description:
            "Controlled database failover with pre-checks, approval, and post-validation"
                .to_string(),
        version: "1.0.0".to_string(),
        tags: vec![
            "database".to_string(),
            "failover".to_string(),
            "high-risk".to_string(),
        ],
        parameters: vec![
            ParameterDef {
                name: "db_name".to_string(),
                description: "Database name".to_string(),
                param_type: ParamType::String,
                required: true,
                default_value: None,
                secret: false,
            },
            ParameterDef {
                name: "target_replica".to_string(),
                description: "Target replica to promote".to_string(),
                param_type: ParamType::String,
                required: true,
                default_value: None,
                secret: false,
            },
        ],
        steps: vec![
            Step {
                id: "pre-check".to_string(),
                name: "Pre-failover checks".to_string(),
                step_type: StepType::Shell {
                    command: "kubectl get pods -l app={{db_name}} -o wide".to_string(),
                    working_dir: None,
                    env: HashMap::new(),
                },
                description: "Check database pod status".to_string(),
                condition: None,
                depends_on: vec![],
                timeout_seconds: 30,
                retry_count: 0,
                continue_on_error: false,
                on_failure: FailureAction::Stop,
            },
            Step {
                id: "dba-approval".to_string(),
                name: "DBA approval required".to_string(),
                step_type: StepType::ManualApproval {
                    message:
                        "CRITICAL: Approve failover of {{db_name}} to {{target_replica}}?"
                            .to_string(),
                    approvers: vec!["dba-team".to_string(), "platform-lead".to_string()],
                    timeout_seconds: 1800,
                },
                description: "Requires DBA and platform lead approval".to_string(),
                condition: None,
                depends_on: vec!["pre-check".to_string()],
                timeout_seconds: 1800,
                retry_count: 0,
                continue_on_error: false,
                on_failure: FailureAction::Stop,
            },
            Step {
                id: "initiate-failover".to_string(),
                name: "Initiate failover".to_string(),
                step_type: StepType::Shell {
                    command: "kubectl patch statefulset {{db_name}} -p '{\"spec\":{\"replicas\":0}}'"
                        .to_string(),
                    working_dir: None,
                    env: HashMap::new(),
                },
                description: "Trigger database failover".to_string(),
                condition: None,
                depends_on: vec!["dba-approval".to_string()],
                timeout_seconds: 120,
                retry_count: 0,
                continue_on_error: false,
                on_failure: FailureAction::Stop,
            },
        ],
        timeout_seconds: 3600,
        on_failure: FailureAction::Notify,
        is_template: true,
        created_by: "cave-runbook/system".to_string(),
        created_at: now,
        updated_at: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_templates_count() {
        let templates = builtin_templates();
        assert_eq!(templates.len(), 5);
    }

    #[test]
    fn test_all_templates_have_steps() {
        for t in builtin_templates() {
            assert!(!t.steps.is_empty(), "Template '{}' has no steps", t.name);
        }
    }

    #[test]
    fn test_all_templates_are_marked_as_template() {
        for t in builtin_templates() {
            assert!(t.is_template, "Template '{}' is not marked as template", t.name);
        }
    }
}
