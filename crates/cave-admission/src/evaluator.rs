// SPDX-License-Identifier: AGPL-3.0-or-later
//! Policy evaluation engine for cave-admission webhook requests.

use crate::models::{
    AdmissionPolicy, AdmissionRequest, AdmissionResponse, AdmissionStatus,
    EnforcementAction, PolicyEvalRule, RuleOperator, ViolationLog,
};
use base64::Engine as _;
use chrono::Utc;
use serde_json::Value;
use uuid::Uuid;

pub struct PolicyEvaluator;

impl PolicyEvaluator {
    pub fn new() -> Self {
        PolicyEvaluator
    }

    /// Evaluate all applicable policies against an admission request.
    /// Returns an AdmissionResponse and any violation logs generated.
    pub fn evaluate(
        &self,
        request: &AdmissionRequest,
        policies: &[AdmissionPolicy],
    ) -> (AdmissionResponse, Vec<ViolationLog>) {
        let mut allowed = true;
        let mut warnings: Vec<String> = Vec::new();
        let mut deny_messages: Vec<String> = Vec::new();
        let mut violation_logs: Vec<ViolationLog> = Vec::new();

        let resource_kind = request.resource.resource.as_str();

        for policy in policies {
            if !policy.enabled {
                continue;
            }
            // Check if operation matches
            if !policy.operation_types.is_empty()
                && !policy.operation_types.contains(&request.operation)
            {
                continue;
            }
            // Check if resource type matches
            if !policy.resource_types.is_empty()
                && !policy.resource_types.iter().any(|r| r == resource_kind || r == "*")
            {
                continue;
            }

            // Evaluate rules
            let mut policy_violations: Vec<String> = Vec::new();
            for rule in &policy.rules {
                if let Some(msg) = self.evaluate_rule(&request.object, rule) {
                    policy_violations.push(msg);
                }
            }

            if policy_violations.is_empty() {
                continue;
            }

            let violation_msg = policy_violations.join("; ");

            match policy.enforcement_action {
                EnforcementAction::Deny => {
                    allowed = false;
                    deny_messages.push(format!("[{}] {}", policy.name, violation_msg));
                    // Log violation
                    violation_logs.push(self.make_violation_log(policy, request, &violation_msg));
                }
                EnforcementAction::Warn => {
                    warnings.push(format!("[{}] {}", policy.name, violation_msg));
                    violation_logs.push(self.make_violation_log(policy, request, &violation_msg));
                }
                EnforcementAction::Audit => {
                    // Log violation but don't affect the response
                    violation_logs.push(self.make_violation_log(policy, request, &violation_msg));
                }
            }
        }

        let status = if !allowed {
            Some(AdmissionStatus {
                code: 403,
                message: deny_messages.join("; "),
            })
        } else {
            None
        };

        let response = AdmissionResponse {
            uid: request.uid,
            allowed,
            status,
            patch: None,
            patch_type: None,
            warnings,
        };

        (response, violation_logs)
    }

    /// Like evaluate but also injects a cave-admission label patch.
    pub fn evaluate_mutating(
        &self,
        request: &AdmissionRequest,
        policies: &[AdmissionPolicy],
    ) -> (AdmissionResponse, Vec<ViolationLog>) {
        let (mut response, logs) = self.evaluate(request, policies);
        if response.allowed {
            let patch = serde_json::json!([{
                "op": "add",
                "path": "/metadata/labels/cave-admission",
                "value": "true"
            }]);
            let patch_str = serde_json::to_string(&patch).unwrap_or_default();
            response.patch = Some(base64::engine::general_purpose::STANDARD.encode(&patch_str));
            response.patch_type = Some("JSONPatch".to_string());
        }
        (response, logs)
    }

    fn make_violation_log(
        &self,
        policy: &AdmissionPolicy,
        request: &AdmissionRequest,
        message: &str,
    ) -> ViolationLog {
        let resource_name = request
            .object
            .pointer("/metadata/name")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let resource_namespace = request
            .object
            .pointer("/metadata/namespace")
            .and_then(Value::as_str)
            .unwrap_or("default")
            .to_string();
        ViolationLog {
            id: Uuid::new_v4(),
            policy_id: policy.id,
            request_uid: request.uid,
            resource_name,
            resource_namespace,
            operation: request.operation,
            message: message.to_string(),
            enforcement_action: policy.enforcement_action,
            logged_at: Utc::now(),
        }
    }

    /// Extract a field value from a JSON object using a dot-notation path
    /// like `spec.securityContext.privileged` -> `/spec/securityContext/privileged`.
    pub fn extract_field<'a>(obj: &'a Value, field_path: &str) -> Option<&'a Value> {
        // Convert dot-notation to JSON pointer
        let pointer = if field_path.starts_with('/') {
            field_path.to_string()
        } else {
            format!("/{}", field_path.replace('.', "/"))
        };
        obj.pointer(&pointer)
    }

    /// Evaluate a single rule against the request object.
    pub fn evaluate_rule(&self, obj: &Value, rule: &PolicyEvalRule) -> Option<String> {
        let field_val = Self::extract_field(obj, &rule.field);

        match rule.operator {
            RuleOperator::Exists => {
                if field_val.is_none() {
                    Some(format!("Field '{}' must exist", rule.field))
                } else {
                    None
                }
            }
            RuleOperator::NotExists => {
                if field_val.is_some() {
                    Some(format!("Field '{}' must not exist", rule.field))
                } else {
                    None
                }
            }
            RuleOperator::Equals => {
                match field_val {
                    None => Some(format!("Field '{}' does not exist", rule.field)),
                    Some(v) if v == &rule.value => None,
                    Some(v) => Some(format!(
                        "Field '{}' has value {:?} but expected {:?}",
                        rule.field, v, rule.value
                    )),
                }
            }
            RuleOperator::NotEquals => {
                match field_val {
                    None => None, // field absent = not equal = rule satisfied
                    Some(v) if v == &rule.value => Some(format!(
                        "Field '{}' must not equal {:?}",
                        rule.field, rule.value
                    )),
                    _ => None,
                }
            }
            RuleOperator::Contains => {
                match field_val {
                    None => Some(format!("Field '{}' does not exist", rule.field)),
                    Some(Value::String(s)) => {
                        let needle = rule.value.as_str().unwrap_or("");
                        if s.contains(needle) {
                            None
                        } else {
                            Some(format!("Field '{}' does not contain {:?}", rule.field, needle))
                        }
                    }
                    Some(Value::Array(arr)) => {
                        if arr.contains(&rule.value) {
                            None
                        } else {
                            Some(format!("Field '{}' does not contain {:?}", rule.field, rule.value))
                        }
                    }
                    _ => Some(format!("Field '{}' is not a string or array", rule.field)),
                }
            }
            RuleOperator::NotContains => {
                match field_val {
                    None => None,
                    Some(Value::String(s)) => {
                        let needle = rule.value.as_str().unwrap_or("");
                        if s.contains(needle) {
                            Some(format!("Field '{}' must not contain {:?}", rule.field, needle))
                        } else {
                            None
                        }
                    }
                    Some(Value::Array(arr)) => {
                        if arr.contains(&rule.value) {
                            Some(format!("Field '{}' must not contain {:?}", rule.field, rule.value))
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            }
            RuleOperator::GreaterThan => {
                let actual = field_val.and_then(Value::as_f64).unwrap_or(0.0);
                let threshold = rule.value.as_f64().unwrap_or(0.0);
                if actual > threshold {
                    None
                } else {
                    Some(format!(
                        "Field '{}' value {} is not greater than {}",
                        rule.field, actual, threshold
                    ))
                }
            }
            RuleOperator::LessThan => {
                let actual = field_val.and_then(Value::as_f64).unwrap_or(0.0);
                let threshold = rule.value.as_f64().unwrap_or(0.0);
                if actual < threshold {
                    None
                } else {
                    Some(format!(
                        "Field '{}' value {} is not less than {}",
                        rule.field, actual, threshold
                    ))
                }
            }
        }
    }
}

impl Default for PolicyEvaluator {
    fn default() -> Self {
        PolicyEvaluator::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AdmissionOperation, GroupVersionResource, UserInfo};

    fn make_request(obj: serde_json::Value) -> AdmissionRequest {
        AdmissionRequest {
            uid: Uuid::new_v4(),
            operation: AdmissionOperation::Create,
            resource: GroupVersionResource {
                group: "".to_string(),
                version: "v1".to_string(),
                resource: "pods".to_string(),
            },
            object: obj,
            old_object: None,
            user_info: UserInfo {
                username: "test-user".to_string(),
                uid: "uid-1".to_string(),
                groups: vec!["system:masters".to_string()],
            },
            dry_run: false,
        }
    }

    fn make_policy(
        rules: Vec<PolicyEvalRule>,
        action: EnforcementAction,
    ) -> AdmissionPolicy {
        AdmissionPolicy {
            id: Uuid::new_v4(),
            name: "test-policy".to_string(),
            description: "".to_string(),
            operation_types: vec![AdmissionOperation::Create],
            resource_types: vec!["pods".to_string()],
            enforcement_action: action,
            rules,
            enabled: true,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn test_equals_operator_match() {
        let evaluator = PolicyEvaluator::new();
        let obj = serde_json::json!({"spec": {"privileged": true}});
        let rule = PolicyEvalRule {
            field: "spec.privileged".to_string(),
            operator: RuleOperator::Equals,
            value: serde_json::json!(true),
        };
        assert!(evaluator.evaluate_rule(&obj, &rule).is_none());
    }

    #[test]
    fn test_not_equals_blocks() {
        let evaluator = PolicyEvaluator::new();
        let obj = serde_json::json!({"spec": {"hostNetwork": true}});
        let rule = PolicyEvalRule {
            field: "spec.hostNetwork".to_string(),
            operator: RuleOperator::NotEquals,
            value: serde_json::json!(true),
        };
        assert!(evaluator.evaluate_rule(&obj, &rule).is_some());
    }

    #[test]
    fn test_exists_passes_when_field_present() {
        let evaluator = PolicyEvaluator::new();
        let obj = serde_json::json!({"metadata": {"labels": {"app": "myapp"}}});
        let rule = PolicyEvalRule {
            field: "metadata.labels.app".to_string(),
            operator: RuleOperator::Exists,
            value: serde_json::json!(null),
        };
        assert!(evaluator.evaluate_rule(&obj, &rule).is_none());
    }

    #[test]
    fn test_deny_policy_blocks_request() {
        let evaluator = PolicyEvaluator::new();
        let obj = serde_json::json!({
            "metadata": {"name": "bad-pod", "namespace": "default"},
            "spec": {"hostNetwork": true}
        });
        let request = make_request(obj);
        // Rule semantics: "field MUST equal false" (constraint validator).
        // Pod has hostNetwork=true → true ≠ false → constraint violated →
        // Deny policy fires → request blocked.
        let policy = make_policy(
            vec![PolicyEvalRule {
                field: "spec.hostNetwork".to_string(),
                operator: RuleOperator::Equals,
                value: serde_json::json!(false),
            }],
            EnforcementAction::Deny,
        );
        let (response, logs) = evaluator.evaluate(&request, &[policy]);
        assert!(!response.allowed);
        assert!(!logs.is_empty());
    }

    #[test]
    fn test_warn_policy_allows_with_warning() {
        let evaluator = PolicyEvaluator::new();
        let obj = serde_json::json!({
            "metadata": {"name": "warn-pod", "namespace": "default"},
            "spec": {}
        });
        let request = make_request(obj);
        let policy = make_policy(
            vec![PolicyEvalRule {
                field: "spec.resources".to_string(),
                operator: RuleOperator::Exists,
                value: serde_json::json!(null),
            }],
            EnforcementAction::Warn,
        );
        let (response, _logs) = evaluator.evaluate(&request, &[policy]);
        assert!(response.allowed);
        assert!(!response.warnings.is_empty());
    }
}
