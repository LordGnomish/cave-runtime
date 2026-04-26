//! In-memory store for cave-admission webhook policies and violations.

use crate::models::{
    AdmissionOperation, AdmissionPolicy, AdmissionRequest, EnforcementAction, PolicyEvalRule,
    RuleOperator, ViolationLog,
};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::RwLock;
use uuid::Uuid;

#[derive(Default)]
pub struct AdmissionStore {
    pub policies: RwLock<HashMap<Uuid, AdmissionPolicy>>,
    pub violations: RwLock<Vec<ViolationLog>>,
}

impl AdmissionStore {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Policies ──────────────────────────────────────────────────────────────

    pub fn insert_policy(&self, policy: AdmissionPolicy) {
        self.policies.write().unwrap().insert(policy.id, policy);
    }

    pub fn get_policy(&self, id: Uuid) -> Option<AdmissionPolicy> {
        self.policies.read().unwrap().get(&id).cloned()
    }

    pub fn list_policies(&self) -> Vec<AdmissionPolicy> {
        let mut v: Vec<_> = self.policies.read().unwrap().values().cloned().collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }

    pub fn update_policy(&self, id: Uuid, updated: AdmissionPolicy) -> Option<AdmissionPolicy> {
        let mut policies = self.policies.write().unwrap();
        if policies.contains_key(&id) {
            policies.insert(id, updated.clone());
            Some(updated)
        } else {
            None
        }
    }

    pub fn delete_policy(&self, id: Uuid) -> bool {
        self.policies.write().unwrap().remove(&id).is_some()
    }

    pub fn set_policy_enabled(&self, id: Uuid, enabled: bool) -> Option<AdmissionPolicy> {
        let mut policies = self.policies.write().unwrap();
        if let Some(p) = policies.get_mut(&id) {
            p.enabled = enabled;
            Some(p.clone())
        } else {
            None
        }
    }

    // ── Violations ────────────────────────────────────────────────────────────

    pub fn log_violation(&self, violation: ViolationLog) {
        self.violations.write().unwrap().push(violation);
    }

    pub fn log_violation_for_request(
        &self,
        policy_id: Uuid,
        request: &AdmissionRequest,
        message: &str,
        enforcement_action: EnforcementAction,
    ) {
        let resource_name = request
            .object
            .pointer("/metadata/name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let resource_namespace = request
            .object
            .pointer("/metadata/namespace")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("default")
            .to_string();
        let violation = ViolationLog {
            id: Uuid::new_v4(),
            policy_id,
            request_uid: request.uid,
            resource_name,
            resource_namespace,
            operation: request.operation,
            message: message.to_string(),
            enforcement_action,
            logged_at: Utc::now(),
        };
        self.violations.write().unwrap().push(violation);
    }

    pub fn recent_violations(&self, limit: usize) -> Vec<ViolationLog> {
        let violations = self.violations.read().unwrap();
        let total = violations.len();
        if total <= limit {
            violations.clone()
        } else {
            violations[total - limit..].to_vec()
        }
    }

    pub fn violations_by_policy(&self, policy_id: Uuid) -> Vec<ViolationLog> {
        self.violations
            .read()
            .unwrap()
            .iter()
            .filter(|v| v.policy_id == policy_id)
            .cloned()
            .collect()
    }

    pub fn stats(&self) -> AdmissionStats {
        let policies = self.policies.read().unwrap();
        let violations = self.violations.read().unwrap();
        let today = Utc::now().date_naive();
        let total_policies = policies.len() as u64;
        let enabled_policies = policies.values().filter(|p| p.enabled).count() as u64;
        let total_violations = violations.len() as u64;
        let violations_today = violations
            .iter()
            .filter(|v| v.logged_at.date_naive() == today)
            .count() as u64;
        AdmissionStats {
            total_policies,
            enabled_policies,
            total_violations,
            violations_today,
        }
    }

    // ── Seed default policies ─────────────────────────────────────────────────

    pub fn seed_default_policies(&self) {
        let now = Utc::now();

        let policies: Vec<AdmissionPolicy> = vec![
            AdmissionPolicy {
                id: Uuid::new_v4(),
                name: "no-privileged-containers".to_string(),
                description: "Deny pods with privileged containers".to_string(),
                operation_types: vec![AdmissionOperation::Create, AdmissionOperation::Update],
                resource_types: vec!["pods".to_string()],
                enforcement_action: EnforcementAction::Deny,
                rules: vec![PolicyEvalRule {
                    field: "spec.securityContext.privileged".to_string(),
                    operator: RuleOperator::NotEquals,
                    value: serde_json::json!(true),
                }],
                enabled: true,
                created_at: now,
            },
            AdmissionPolicy {
                id: Uuid::new_v4(),
                name: "require-resource-limits".to_string(),
                description: "Warn when pods are missing resource limits".to_string(),
                operation_types: vec![AdmissionOperation::Create],
                resource_types: vec!["pods".to_string()],
                enforcement_action: EnforcementAction::Warn,
                rules: vec![PolicyEvalRule {
                    field: "spec.containers".to_string(),
                    operator: RuleOperator::Exists,
                    value: serde_json::json!(null),
                }],
                enabled: true,
                created_at: now,
            },
            AdmissionPolicy {
                id: Uuid::new_v4(),
                name: "no-latest-image".to_string(),
                description: "Deny pods using :latest image tag".to_string(),
                operation_types: vec![AdmissionOperation::Create, AdmissionOperation::Update],
                resource_types: vec!["pods".to_string()],
                enforcement_action: EnforcementAction::Deny,
                rules: vec![PolicyEvalRule {
                    field: "spec.containers.0.image".to_string(),
                    operator: RuleOperator::NotContains,
                    value: serde_json::json!(":latest"),
                }],
                enabled: true,
                created_at: now,
            },
            AdmissionPolicy {
                id: Uuid::new_v4(),
                name: "require-labels".to_string(),
                description: "Warn when deployments are missing app label".to_string(),
                operation_types: vec![AdmissionOperation::Create],
                resource_types: vec!["deployments".to_string()],
                enforcement_action: EnforcementAction::Warn,
                rules: vec![PolicyEvalRule {
                    field: "metadata.labels.app".to_string(),
                    operator: RuleOperator::Exists,
                    value: serde_json::json!(null),
                }],
                enabled: true,
                created_at: now,
            },
            AdmissionPolicy {
                id: Uuid::new_v4(),
                name: "no-host-network".to_string(),
                description: "Deny pods using host network".to_string(),
                operation_types: vec![AdmissionOperation::Create, AdmissionOperation::Update],
                resource_types: vec!["pods".to_string()],
                enforcement_action: EnforcementAction::Deny,
                rules: vec![PolicyEvalRule {
                    field: "spec.hostNetwork".to_string(),
                    operator: RuleOperator::NotEquals,
                    value: serde_json::json!(true),
                }],
                enabled: true,
                created_at: now,
            },
        ];

        for policy in policies {
            self.insert_policy(policy);
        }
    }
}

#[derive(serde::Serialize)]
pub struct AdmissionStats {
    pub total_policies: u64,
    pub enabled_policies: u64,
    pub total_violations: u64,
    pub violations_today: u64,
}
