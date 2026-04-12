//! Attribute-Based Access Control (ABAC).
//!
//! Policies evaluate user attributes, resource attributes, and environment
//! conditions to make fine-grained allow/deny decisions beyond RBAC.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// A single attribute condition: attribute_name op value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Condition {
    pub attribute: String,
    pub operator: ConditionOperator,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionOperator {
    Equals,
    NotEquals,
    Contains,
    StartsWith,
    GreaterThan,
    LessThan,
    In,
    NotIn,
    Exists,
}

impl Condition {
    pub fn evaluate(&self, attrs: &HashMap<String, serde_json::Value>) -> bool {
        match self.operator {
            ConditionOperator::Exists => attrs.contains_key(&self.attribute),
            _ => {
                let attr_val = match attrs.get(&self.attribute) {
                    Some(v) => v,
                    None => return false,
                };
                match self.operator {
                    ConditionOperator::Equals => attr_val == &self.value,
                    ConditionOperator::NotEquals => attr_val != &self.value,
                    ConditionOperator::Contains => {
                        if let (Some(s), Some(pat)) =
                            (attr_val.as_str(), self.value.as_str())
                        {
                            s.contains(pat)
                        } else if let Some(arr) = attr_val.as_array() {
                            arr.contains(&self.value)
                        } else {
                            false
                        }
                    }
                    ConditionOperator::StartsWith => {
                        match (attr_val.as_str(), self.value.as_str()) {
                            (Some(s), Some(prefix)) => s.starts_with(prefix),
                            _ => false,
                        }
                    }
                    ConditionOperator::GreaterThan => {
                        match (attr_val.as_f64(), self.value.as_f64()) {
                            (Some(a), Some(b)) => a > b,
                            _ => false,
                        }
                    }
                    ConditionOperator::LessThan => {
                        match (attr_val.as_f64(), self.value.as_f64()) {
                            (Some(a), Some(b)) => a < b,
                            _ => false,
                        }
                    }
                    ConditionOperator::In => {
                        if let Some(arr) = self.value.as_array() {
                            arr.contains(attr_val)
                        } else {
                            false
                        }
                    }
                    ConditionOperator::NotIn => {
                        if let Some(arr) = self.value.as_array() {
                            !arr.contains(attr_val)
                        } else {
                            true
                        }
                    }
                    ConditionOperator::Exists => unreachable!(),
                }
            }
        }
    }
}

/// ABAC policy effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum PolicyEffect {
    Allow,
    Deny,
}

/// An ABAC policy — evaluated against a request context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbacPolicy {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub effect: PolicyEffect,
    /// The action this policy governs, e.g., "secrets:write".
    pub action: String,
    /// All subject conditions must match.
    pub subject_conditions: Vec<Condition>,
    /// All resource conditions must match.
    pub resource_conditions: Vec<Condition>,
    /// All environment conditions must match.
    pub environment_conditions: Vec<Condition>,
    pub priority: i32,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}

impl AbacPolicy {
    pub fn new(name: &str, action: &str, effect: PolicyEffect) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.to_string(),
            description: String::new(),
            effect,
            action: action.to_string(),
            subject_conditions: vec![],
            resource_conditions: vec![],
            environment_conditions: vec![],
            priority: 0,
            enabled: true,
            created_at: Utc::now(),
        }
    }

    /// Evaluate this policy against the given context.
    /// Returns Some(effect) if all conditions match, None if policy doesn't apply.
    pub fn evaluate(&self, ctx: &AbacContext) -> Option<PolicyEffect> {
        if !self.enabled {
            return None;
        }

        // Check if action matches (supports wildcards: "secrets:*", "*")
        if !self.action_matches(&ctx.action) {
            return None;
        }

        let all_match = self
            .subject_conditions
            .iter()
            .all(|c| c.evaluate(&ctx.subject_attributes))
            && self
                .resource_conditions
                .iter()
                .all(|c| c.evaluate(&ctx.resource_attributes))
            && self
                .environment_conditions
                .iter()
                .all(|c| c.evaluate(&ctx.environment));

        if all_match {
            Some(self.effect.clone())
        } else {
            None
        }
    }

    fn action_matches(&self, action: &str) -> bool {
        if self.action == "*" {
            return true;
        }
        if self.action == action {
            return true;
        }
        // "module:*" matches "module:anything"
        if let Some(prefix) = self.action.strip_suffix(":*") {
            if let Some(req_prefix) = action.split(':').next() {
                return prefix == req_prefix;
            }
        }
        false
    }
}

/// The context provided to the ABAC engine for a single authorization decision.
#[derive(Debug, Clone)]
pub struct AbacContext {
    /// The action being requested, e.g., "secrets:write".
    pub action: String,
    /// Attributes of the requesting subject (user/service).
    pub subject_attributes: HashMap<String, serde_json::Value>,
    /// Attributes of the resource being accessed.
    pub resource_attributes: HashMap<String, serde_json::Value>,
    /// Environmental attributes (time, IP, region, etc.).
    pub environment: HashMap<String, serde_json::Value>,
}

impl AbacContext {
    pub fn new(action: &str) -> Self {
        Self {
            action: action.to_string(),
            subject_attributes: HashMap::new(),
            resource_attributes: HashMap::new(),
            environment: HashMap::new(),
        }
    }

    pub fn with_subject(mut self, key: &str, value: impl Into<serde_json::Value>) -> Self {
        self.subject_attributes.insert(key.to_string(), value.into());
        self
    }

    pub fn with_resource(mut self, key: &str, value: impl Into<serde_json::Value>) -> Self {
        self.resource_attributes.insert(key.to_string(), value.into());
        self
    }

    pub fn with_env(mut self, key: &str, value: impl Into<serde_json::Value>) -> Self {
        self.environment.insert(key.to_string(), value.into());
        self
    }
}

/// Decision returned by ABAC evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbacDecision {
    Allow,
    Deny,
    /// No applicable policy found — fall through to RBAC or default deny.
    NotApplicable,
}

/// ABAC policy engine.
#[derive(Clone)]
pub struct AbacEngine {
    policies: Arc<RwLock<Vec<AbacPolicy>>>,
}

impl AbacEngine {
    pub fn new() -> Self {
        Self {
            policies: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub async fn add_policy(&self, policy: AbacPolicy) {
        let mut policies = self.policies.write().await;
        policies.push(policy);
        // Sort by priority descending (higher priority evaluated first)
        policies.sort_by(|a, b| b.priority.cmp(&a.priority));
    }

    pub async fn remove_policy(&self, id: Uuid) {
        self.policies.write().await.retain(|p| p.id != id);
    }

    /// Evaluate all policies against the context.
    /// Deny takes precedence over Allow. Returns first matching Deny, then first matching Allow.
    pub async fn evaluate(&self, ctx: &AbacContext) -> AbacDecision {
        let policies = self.policies.read().await;

        let mut has_allow = false;
        for policy in policies.iter() {
            if let Some(effect) = policy.evaluate(ctx) {
                match effect {
                    PolicyEffect::Deny => return AbacDecision::Deny,
                    PolicyEffect::Allow => has_allow = true,
                }
            }
        }

        if has_allow {
            AbacDecision::Allow
        } else {
            AbacDecision::NotApplicable
        }
    }
}

impl Default for AbacEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_allow_policy(action: &str) -> AbacPolicy {
        AbacPolicy::new("test-allow", action, PolicyEffect::Allow)
    }

    fn make_deny_policy(action: &str) -> AbacPolicy {
        AbacPolicy::new("test-deny", action, PolicyEffect::Deny)
    }

    #[test]
    fn condition_equals_matches() {
        let cond = Condition {
            attribute: "department".to_string(),
            operator: ConditionOperator::Equals,
            value: serde_json::json!("engineering"),
        };
        let mut attrs = HashMap::new();
        attrs.insert("department".to_string(), serde_json::json!("engineering"));
        assert!(cond.evaluate(&attrs));
        attrs.insert("department".to_string(), serde_json::json!("finance"));
        assert!(!cond.evaluate(&attrs));
    }

    #[test]
    fn condition_in_operator() {
        let cond = Condition {
            attribute: "region".to_string(),
            operator: ConditionOperator::In,
            value: serde_json::json!(["eu-west", "eu-central"]),
        };
        let mut attrs = HashMap::new();
        attrs.insert("region".to_string(), serde_json::json!("eu-west"));
        assert!(cond.evaluate(&attrs));
        attrs.insert("region".to_string(), serde_json::json!("us-east"));
        assert!(!cond.evaluate(&attrs));
    }

    #[test]
    fn condition_exists_operator() {
        let cond = Condition {
            attribute: "mfa_verified".to_string(),
            operator: ConditionOperator::Exists,
            value: serde_json::Value::Null,
        };
        let mut attrs = HashMap::new();
        assert!(!cond.evaluate(&attrs));
        attrs.insert("mfa_verified".to_string(), serde_json::json!(true));
        assert!(cond.evaluate(&attrs));
    }

    #[tokio::test]
    async fn abac_allow_policy_grants_access() {
        let engine = AbacEngine::new();
        let mut policy = make_allow_policy("secrets:read");
        policy.subject_conditions.push(Condition {
            attribute: "clearance".to_string(),
            operator: ConditionOperator::Equals,
            value: serde_json::json!("high"),
        });
        engine.add_policy(policy).await;

        let ctx = AbacContext::new("secrets:read")
            .with_subject("clearance", "high");

        assert_eq!(engine.evaluate(&ctx).await, AbacDecision::Allow);
    }

    #[tokio::test]
    async fn abac_deny_policy_blocks_access() {
        let engine = AbacEngine::new();
        let mut deny = make_deny_policy("secrets:write");
        deny.subject_conditions.push(Condition {
            attribute: "account_locked".to_string(),
            operator: ConditionOperator::Equals,
            value: serde_json::json!(true),
        });
        engine.add_policy(deny).await;

        let ctx = AbacContext::new("secrets:write")
            .with_subject("account_locked", true);

        assert_eq!(engine.evaluate(&ctx).await, AbacDecision::Deny);
    }

    #[tokio::test]
    async fn abac_deny_overrides_allow() {
        let engine = AbacEngine::new();

        // Allow all secrets:read
        let mut allow = make_allow_policy("secrets:read");
        allow.priority = 0;
        engine.add_policy(allow).await;

        // Deny if IP is blocked
        let mut deny = make_deny_policy("secrets:read");
        deny.priority = 10; // Higher priority
        deny.environment_conditions.push(Condition {
            attribute: "ip_blocked".to_string(),
            operator: ConditionOperator::Equals,
            value: serde_json::json!(true),
        });
        engine.add_policy(deny).await;

        let ctx = AbacContext::new("secrets:read")
            .with_env("ip_blocked", true);

        assert_eq!(engine.evaluate(&ctx).await, AbacDecision::Deny);
    }

    #[tokio::test]
    async fn abac_no_matching_policy_returns_not_applicable() {
        let engine = AbacEngine::new();
        // No policies added
        let ctx = AbacContext::new("flags:write");
        assert_eq!(engine.evaluate(&ctx).await, AbacDecision::NotApplicable);
    }

    #[tokio::test]
    async fn abac_wildcard_action_matches_any() {
        let engine = AbacEngine::new();
        let mut policy = make_allow_policy("secrets:*");
        policy.subject_conditions.push(Condition {
            attribute: "role".to_string(),
            operator: ConditionOperator::Equals,
            value: serde_json::json!("vault-admin"),
        });
        engine.add_policy(policy).await;

        let read_ctx = AbacContext::new("secrets:read")
            .with_subject("role", "vault-admin");
        let write_ctx = AbacContext::new("secrets:write")
            .with_subject("role", "vault-admin");
        let delete_ctx = AbacContext::new("secrets:delete")
            .with_subject("role", "vault-admin");

        assert_eq!(engine.evaluate(&read_ctx).await, AbacDecision::Allow);
        assert_eq!(engine.evaluate(&write_ctx).await, AbacDecision::Allow);
        assert_eq!(engine.evaluate(&delete_ctx).await, AbacDecision::Allow);
    }
}
