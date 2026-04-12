use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Policy {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub rules: Vec<PolicyRule>,
    pub enforcement: EnforcementMode,
    pub created_at: DateTime<Utc>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PolicyRule {
    pub id: Uuid,
    pub name: String,
    pub condition: RuleCondition,
    pub severity: ViolationSeverity,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuleCondition {
    FieldEquals { field: String, value: serde_json::Value },
    FieldExists { field: String },
    FieldNotExists { field: String },
    FieldGreaterThan { field: String, threshold: f64 },
    FieldLessThan { field: String, threshold: f64 },
    AllOf { conditions: Vec<RuleCondition> },
    AnyOf { conditions: Vec<RuleCondition> },
    Not { condition: Box<RuleCondition> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EnforcementMode {
    Audit,
    Enforce,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ViolationSeverity {
    Critical,
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Violation {
    pub rule_id: Uuid,
    pub rule_name: String,
    pub severity: ViolationSeverity,
    pub message: String,
    pub resource_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyDecision {
    pub allowed: bool,
    pub violations: Vec<Violation>,
    pub enforcement: EnforcementMode,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_rule_condition_field_equals_serde() {
        let cond = RuleCondition::FieldEquals {
            field: "status".to_string(),
            value: serde_json::json!("active"),
        };
        let json = serde_json::to_string(&cond).unwrap();
        let back: RuleCondition = serde_json::from_str(&json).unwrap();
        assert_eq!(cond, back);
    }

    #[test]
    fn test_rule_condition_not_serde() {
        let inner = RuleCondition::FieldExists { field: "owner".to_string() };
        let cond = RuleCondition::Not { condition: Box::new(inner) };
        let json = serde_json::to_string(&cond).unwrap();
        let back: RuleCondition = serde_json::from_str(&json).unwrap();
        assert_eq!(cond, back);
    }

    #[test]
    fn test_rule_condition_all_of_serde() {
        let cond = RuleCondition::AllOf {
            conditions: vec![
                RuleCondition::FieldExists { field: "name".to_string() },
                RuleCondition::FieldExists { field: "version".to_string() },
            ],
        };
        let json = serde_json::to_string(&cond).unwrap();
        let back: RuleCondition = serde_json::from_str(&json).unwrap();
        assert_eq!(cond, back);
    }

    #[test]
    fn test_enforcement_mode_serde() {
        assert_eq!(serde_json::to_string(&EnforcementMode::Audit).unwrap(), "\"audit\"");
        assert_eq!(serde_json::to_string(&EnforcementMode::Enforce).unwrap(), "\"enforce\"");
        assert_eq!(serde_json::to_string(&EnforcementMode::Disabled).unwrap(), "\"disabled\"");
    }

    #[test]
    fn test_violation_severity_serde() {
        assert_eq!(serde_json::to_string(&ViolationSeverity::Critical).unwrap(), "\"critical\"");
        assert_eq!(serde_json::to_string(&ViolationSeverity::High).unwrap(), "\"high\"");
        assert_eq!(serde_json::to_string(&ViolationSeverity::Medium).unwrap(), "\"medium\"");
        assert_eq!(serde_json::to_string(&ViolationSeverity::Low).unwrap(), "\"low\"");
    }

    #[test]
    fn test_policy_rule_serde() {
        let rule = PolicyRule {
            id: Uuid::new_v4(),
            name: "no-root".to_string(),
            condition: RuleCondition::FieldEquals {
                field: "user".to_string(),
                value: serde_json::json!("root"),
            },
            severity: ViolationSeverity::Critical,
            message: "Root user not allowed".to_string(),
        };
        let json = serde_json::to_string(&rule).unwrap();
        let back: PolicyRule = serde_json::from_str(&json).unwrap();
        assert_eq!(rule, back);
    }

    #[test]
    fn test_policy_serde_roundtrip() {
        let policy = Policy {
            id: Uuid::new_v4(),
            name: "test-policy".to_string(),
            description: "A test".to_string(),
            rules: vec![],
            enforcement: EnforcementMode::Audit,
            created_at: Utc::now(),
            enabled: true,
        };
        let json = serde_json::to_string(&policy).unwrap();
        let back: Policy = serde_json::from_str(&json).unwrap();
        assert_eq!(policy, back);
    }

    #[test]
    fn test_violation_serde() {
        let v = Violation {
            rule_id: Uuid::new_v4(),
            rule_name: "rule-1".to_string(),
            severity: ViolationSeverity::High,
            message: "Field missing".to_string(),
            resource_path: Some("spec.containers[0]".to_string()),
        };
        let json = serde_json::to_string(&v).unwrap();
        let back: Violation = serde_json::from_str(&json).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn test_condition_greater_than_serde() {
        let cond = RuleCondition::FieldGreaterThan {
            field: "replicas".to_string(),
            threshold: 10.0,
        };
        let json = serde_json::to_string(&cond).unwrap();
        let back: RuleCondition = serde_json::from_str(&json).unwrap();
        assert_eq!(cond, back);
    }
}
