use crate::models::{EnforcementMode, Policy, PolicyDecision, PolicyRule, RuleCondition, Violation};
use serde_json::Value;

/// Evaluate a single rule condition against a resource
pub fn evaluate_condition(condition: &RuleCondition, resource: &Value) -> bool {
    match condition {
        RuleCondition::FieldEquals { field, value } => {
            get_field(resource, field).map_or(false, |v| v == value)
        }
        RuleCondition::FieldExists { field } => get_field(resource, field).is_some(),
        RuleCondition::FieldNotExists { field } => get_field(resource, field).is_none(),
        RuleCondition::FieldGreaterThan { field, threshold } => get_field(resource, field)
            .and_then(|v| v.as_f64())
            .map_or(false, |n| n > *threshold),
        RuleCondition::FieldLessThan { field, threshold } => get_field(resource, field)
            .and_then(|v| v.as_f64())
            .map_or(false, |n| n < *threshold),
        RuleCondition::AllOf { conditions } => {
            conditions.iter().all(|c| evaluate_condition(c, resource))
        }
        RuleCondition::AnyOf { conditions } => {
            conditions.iter().any(|c| evaluate_condition(c, resource))
        }
        RuleCondition::Not { condition } => !evaluate_condition(condition, resource),
    }
}

/// Get a field value from a JSON object using dot notation
fn get_field<'a>(resource: &'a Value, field: &str) -> Option<&'a Value> {
    let parts: Vec<&str> = field.split('.').collect();
    let mut current = resource;
    for part in parts {
        current = current.get(part)?;
    }
    Some(current)
}

/// Evaluate a policy rule and return a Violation if the condition is violated
/// Note: a rule fires a violation when the condition is TRUE (it's a violation trigger)
pub fn evaluate_rule(rule: &PolicyRule, resource: &Value) -> Option<Violation> {
    if evaluate_condition(&rule.condition, resource) {
        Some(Violation {
            rule_id: rule.id,
            rule_name: rule.name.clone(),
            severity: rule.severity.clone(),
            message: rule.message.clone(),
            resource_path: None,
        })
    } else {
        None
    }
}

/// Evaluate all rules in a policy and return all violations
pub fn evaluate_policy(policy: &Policy, resource: &Value) -> PolicyDecision {
    if !policy.enabled {
        return PolicyDecision {
            allowed: true,
            violations: vec![],
            enforcement: policy.enforcement.clone(),
        };
    }
    let violations: Vec<Violation> = policy
        .rules
        .iter()
        .filter_map(|rule| evaluate_rule(rule, resource))
        .collect();
    let allowed = violations.is_empty() || policy.enforcement == EnforcementMode::Audit;
    PolicyDecision {
        allowed,
        violations,
        enforcement: policy.enforcement.clone(),
    }
}

/// Find all violations across multiple policies
pub fn find_violations(policies: &[Policy], resource: &Value) -> Vec<Violation> {
    policies
        .iter()
        .filter(|p| p.enabled)
        .flat_map(|p| evaluate_policy(p, resource).violations)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{EnforcementMode, PolicyRule, RuleCondition, ViolationSeverity};
    use chrono::Utc;
    use serde_json::json;
    use uuid::Uuid;

    fn make_rule(condition: RuleCondition, severity: ViolationSeverity) -> PolicyRule {
        PolicyRule {
            id: Uuid::new_v4(),
            name: "test-rule".to_string(),
            condition,
            severity,
            message: "Violation triggered".to_string(),
        }
    }

    fn make_policy(rules: Vec<PolicyRule>, enforcement: EnforcementMode, enabled: bool) -> Policy {
        Policy {
            id: Uuid::new_v4(),
            name: "test-policy".to_string(),
            description: "".to_string(),
            rules,
            enforcement,
            created_at: Utc::now(),
            enabled,
        }
    }

    #[test]
    fn test_field_equals_match() {
        let cond = RuleCondition::FieldEquals {
            field: "status".to_string(),
            value: json!("active"),
        };
        let resource = json!({"status": "active"});
        assert!(evaluate_condition(&cond, &resource));
    }

    #[test]
    fn test_field_equals_no_match() {
        let cond = RuleCondition::FieldEquals {
            field: "status".to_string(),
            value: json!("active"),
        };
        let resource = json!({"status": "inactive"});
        assert!(!evaluate_condition(&cond, &resource));
    }

    #[test]
    fn test_field_exists_true() {
        let cond = RuleCondition::FieldExists { field: "name".to_string() };
        let resource = json!({"name": "my-app"});
        assert!(evaluate_condition(&cond, &resource));
    }

    #[test]
    fn test_field_not_exists_true() {
        let cond = RuleCondition::FieldNotExists { field: "owner".to_string() };
        let resource = json!({"name": "my-app"});
        assert!(evaluate_condition(&cond, &resource));
    }

    #[test]
    fn test_field_greater_than() {
        let cond = RuleCondition::FieldGreaterThan {
            field: "replicas".to_string(),
            threshold: 5.0,
        };
        let resource = json!({"replicas": 10});
        assert!(evaluate_condition(&cond, &resource));

        let resource_low = json!({"replicas": 3});
        assert!(!evaluate_condition(&cond, &resource_low));
    }

    #[test]
    fn test_field_less_than() {
        let cond = RuleCondition::FieldLessThan {
            field: "cpu".to_string(),
            threshold: 0.5,
        };
        let resource = json!({"cpu": 0.1});
        assert!(evaluate_condition(&cond, &resource));

        let resource_high = json!({"cpu": 0.9});
        assert!(!evaluate_condition(&cond, &resource_high));
    }

    #[test]
    fn test_all_of_both_true() {
        let cond = RuleCondition::AllOf {
            conditions: vec![
                RuleCondition::FieldExists { field: "name".to_string() },
                RuleCondition::FieldExists { field: "version".to_string() },
            ],
        };
        let resource = json!({"name": "app", "version": "1.0"});
        assert!(evaluate_condition(&cond, &resource));
    }

    #[test]
    fn test_all_of_one_false() {
        let cond = RuleCondition::AllOf {
            conditions: vec![
                RuleCondition::FieldExists { field: "name".to_string() },
                RuleCondition::FieldExists { field: "missing_field".to_string() },
            ],
        };
        let resource = json!({"name": "app"});
        assert!(!evaluate_condition(&cond, &resource));
    }

    #[test]
    fn test_any_of_one_true() {
        let cond = RuleCondition::AnyOf {
            conditions: vec![
                RuleCondition::FieldExists { field: "owner".to_string() },
                RuleCondition::FieldExists { field: "name".to_string() },
            ],
        };
        let resource = json!({"name": "app"});
        assert!(evaluate_condition(&cond, &resource));
    }

    #[test]
    fn test_not_condition() {
        let cond = RuleCondition::Not {
            condition: Box::new(RuleCondition::FieldExists { field: "blocked".to_string() }),
        };
        let resource_without = json!({"name": "app"});
        assert!(evaluate_condition(&cond, &resource_without));

        let resource_with = json!({"blocked": true});
        assert!(!evaluate_condition(&cond, &resource_with));
    }

    #[test]
    fn test_nested_field_access() {
        let cond = RuleCondition::FieldEquals {
            field: "metadata.name".to_string(),
            value: json!("my-deployment"),
        };
        let resource = json!({"metadata": {"name": "my-deployment", "namespace": "default"}});
        assert!(evaluate_condition(&cond, &resource));

        let resource_wrong = json!({"metadata": {"name": "other"}});
        assert!(!evaluate_condition(&cond, &resource_wrong));
    }

    #[test]
    fn test_evaluate_rule_violation() {
        let rule = make_rule(
            RuleCondition::FieldExists { field: "dangerous_flag".to_string() },
            ViolationSeverity::Critical,
        );
        let resource = json!({"dangerous_flag": true});
        let result = evaluate_rule(&rule, &resource);
        assert!(result.is_some());
        assert_eq!(result.unwrap().severity, ViolationSeverity::Critical);
    }

    #[test]
    fn test_evaluate_rule_no_violation() {
        let rule = make_rule(
            RuleCondition::FieldExists { field: "dangerous_flag".to_string() },
            ViolationSeverity::Critical,
        );
        let resource = json!({"safe_field": true});
        assert!(evaluate_rule(&rule, &resource).is_none());
    }

    #[test]
    fn test_evaluate_policy_disabled() {
        let rule = make_rule(
            RuleCondition::FieldExists { field: "anything".to_string() },
            ViolationSeverity::High,
        );
        let policy = make_policy(vec![rule], EnforcementMode::Enforce, false);
        let resource = json!({"anything": "value"});
        let decision = evaluate_policy(&policy, &resource);
        assert!(decision.allowed);
        assert!(decision.violations.is_empty());
    }

    #[test]
    fn test_evaluate_policy_enforce_blocks() {
        let rule = make_rule(
            RuleCondition::FieldExists { field: "bad_field".to_string() },
            ViolationSeverity::High,
        );
        let policy = make_policy(vec![rule], EnforcementMode::Enforce, true);
        let resource = json!({"bad_field": "present"});
        let decision = evaluate_policy(&policy, &resource);
        assert!(!decision.allowed);
        assert_eq!(decision.violations.len(), 1);
    }

    #[test]
    fn test_evaluate_policy_audit_allows() {
        let rule = make_rule(
            RuleCondition::FieldExists { field: "bad_field".to_string() },
            ViolationSeverity::High,
        );
        let policy = make_policy(vec![rule], EnforcementMode::Audit, true);
        let resource = json!({"bad_field": "present"});
        let decision = evaluate_policy(&policy, &resource);
        assert!(decision.allowed);
        assert_eq!(decision.violations.len(), 1);
    }

    #[test]
    fn test_find_violations_multiple_policies() {
        let rule1 = make_rule(
            RuleCondition::FieldExists { field: "field_a".to_string() },
            ViolationSeverity::High,
        );
        let rule2 = make_rule(
            RuleCondition::FieldExists { field: "field_b".to_string() },
            ViolationSeverity::Medium,
        );
        let policies = vec![
            make_policy(vec![rule1], EnforcementMode::Audit, true),
            make_policy(vec![rule2], EnforcementMode::Audit, true),
        ];
        let resource = json!({"field_a": 1, "field_b": 2});
        let violations = find_violations(&policies, &resource);
        assert_eq!(violations.len(), 2);
    }

    #[test]
    fn test_find_violations_skips_disabled_policies() {
        let rule = make_rule(
            RuleCondition::FieldExists { field: "anything".to_string() },
            ViolationSeverity::Low,
        );
        let policies = vec![make_policy(vec![rule], EnforcementMode::Enforce, false)];
        let resource = json!({"anything": "value"});
        let violations = find_violations(&policies, &resource);
        assert!(violations.is_empty());
    }
}
