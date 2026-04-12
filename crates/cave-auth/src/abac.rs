//! Attribute-Based Access Control (ABAC) policy engine.
//!
//! Evaluates policies of the form:
//! ```text
//! ALLOW <action> ON <resource> IF subject.team == "SRE"
//!                               AND resource.environment == "production"
//!                               AND context.hour BETWEEN 8 AND 18
//! ```
//!
//! ## OPA-compatible policy format
//!
//! Policies are serialised as JSON so they can be imported/exported to/from an
//! OPA bundle.  The CAVE engine is a lightweight subset: full OPA Rego
//! evaluation is out of scope; complex policies should use the OPA sidecar.
//!
//! ## Example policy (JSON)
//! ```json
//! {
//!   "id": "sre-p1-incidents",
//!   "description": "Only SRE team members can acknowledge P1 incidents in production",
//!   "effect": "allow",
//!   "action": "cave-incidents:manage",
//!   "conditions": [
//!     { "attribute": "subject.team",              "operator": "eq",       "value": "SRE" },
//!     { "attribute": "resource.environment",      "operator": "eq",       "value": "production" },
//!     { "attribute": "resource.severity",         "operator": "eq",       "value": "P1" }
//!   ]
//! }
//! ```

use chrono::{Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use tokio::sync::RwLock;

// ─── Subject / Resource / Context attributes ──────────────────────────────────

/// Attributes describing the user/principal making the request.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubjectAttributes {
    /// Organisational department, e.g. "Engineering"
    pub department: Option<String>,
    /// Team name, e.g. "SRE", "Platform", "Security"
    pub team: Option<String>,
    /// Physical / logical location, e.g. "DE", "US-EAST"
    pub location: Option<String>,
    /// Employment type, e.g. "employee", "contractor"
    pub employment_type: Option<String>,
    /// Arbitrary custom attributes propagated from Okta profile
    #[serde(default)]
    pub custom: HashMap<String, String>,
}

/// Attributes describing the target resource.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceAttributes {
    /// Target environment, e.g. "production", "staging", "dev"
    pub environment: Option<String>,
    /// Data sensitivity level, e.g. "public", "internal", "confidential", "restricted"
    pub sensitivity: Option<String>,
    /// The CAVE module owning this resource, e.g. "cave-incidents"
    pub module: Option<String>,
    /// Opaque resource type, e.g. "incident", "flag", "secret"
    pub resource_type: Option<String>,
    /// Severity / priority, e.g. "P1", "critical"
    pub severity: Option<String>,
    /// Arbitrary extra attributes
    #[serde(default)]
    pub custom: HashMap<String, String>,
}

/// Runtime context for the request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestContext {
    /// Client IP address
    pub ip_address: Option<IpAddr>,
    /// UTC hour of day (0-23) — for time-window policies
    pub hour_utc: u32,
    /// Whether the request originates from within the corporate network
    pub is_internal_network: bool,
    /// Whether the request uses MFA-authenticated session
    pub mfa_verified: bool,
}

impl Default for RequestContext {
    fn default() -> Self {
        Self {
            ip_address: None,
            hour_utc: Utc::now().hour(),
            is_internal_network: false,
            mfa_verified: false,
//! Attribute-Based Access Control (ABAC).
//! Policies evaluate user attributes, resource attributes, and environment
//! conditions to make fine-grained allow/deny decisions beyond RBAC.
use chrono::{DateTime, Utc};
use uuid::Uuid;
/// A single attribute condition: attribute_name op value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Condition {
    pub attribute: String,
    pub operator: ConditionOperator,
    pub value: serde_json::Value,
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
                    ConditionOperator::StartsWith => {
                        match (attr_val.as_str(), self.value.as_str()) {
                            (Some(s), Some(prefix)) => s.starts_with(prefix),
                            _ => false,
                    ConditionOperator::GreaterThan => {
                        match (attr_val.as_f64(), self.value.as_f64()) {
                            (Some(a), Some(b)) => a > b,
                            _ => false,
                    ConditionOperator::LessThan => {
                        match (attr_val.as_f64(), self.value.as_f64()) {
                            (Some(a), Some(b)) => a < b,
                            _ => false,
                    ConditionOperator::In => {
                        if let Some(arr) = self.value.as_array() {
                            arr.contains(attr_val)
                        } else {
                            false
                    ConditionOperator::NotIn => {
                        if let Some(arr) = self.value.as_array() {
                            !arr.contains(attr_val)
                        } else {
                            true
                    ConditionOperator::Exists => unreachable!(),
        }
    }
}

// ─── Policy model ─────────────────────────────────────────────────────────────

/// Effect of a policy: explicit allow or explicit deny.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
/// ABAC policy effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum PolicyEffect {
    Allow,
    Deny,
}

/// Comparison operator for a condition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConditionOperator {
    Eq,
    NotEq,
    In,
    NotIn,
    Contains,
    StartsWith,
    /// Numeric: less-than-or-equal (works on `hour_utc` etc.)
    Lte,
    /// Numeric: greater-than-or-equal
    Gte,
}

/// A single condition that must hold for the policy to match.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyCondition {
    /// Dot-path attribute reference: "subject.team", "resource.environment",
    /// "context.hour_utc", "context.mfa_verified", etc.
    pub attribute: String,
    pub operator: ConditionOperator,
    /// String value to compare against (numeric comparisons parse at eval time)
    pub value: String,
}

/// A single ABAC policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbacPolicy {
    pub id: String,
    pub description: String,
    /// The action this policy applies to, e.g. "cave-incidents:manage"
    /// Use "*" to match any action.
    pub action: String,
    pub effect: PolicyEffect,
    /// All conditions must be satisfied for the policy to fire.
    pub conditions: Vec<PolicyCondition>,
    /// Numeric priority — higher numbers evaluated first (useful for deny overrides)
    #[serde(default)]
    pub priority: i32,
}

/// Result of evaluating all policies for a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    /// Explicitly allowed by at least one policy, no denies
    Allow,
    /// Explicitly denied by at least one policy
    Deny { policy_id: String, reason: String },
    /// No policy matched — caller falls through to RBAC
    NoMatch,
}

// ─── Policy engine ────────────────────────────────────────────────────────────

/// Thread-safe ABAC policy engine.
#[derive(Clone)]
pub struct AbacPolicyEngine {
    policies: Arc<RwLock<Vec<AbacPolicy>>>,
}

impl AbacPolicyEngine {
/// An ABAC policy — evaluated against a request context.
    pub id: Uuid,
    pub name: String,
    /// The action this policy governs, e.g., "secrets:write".
    /// All subject conditions must match.
    pub subject_conditions: Vec<Condition>,
    /// All resource conditions must match.
    pub resource_conditions: Vec<Condition>,
    /// All environment conditions must match.
    pub environment_conditions: Vec<Condition>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
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
    /// Evaluate this policy against the given context.
    /// Returns Some(effect) if all conditions match, None if policy doesn't apply.
    pub fn evaluate(&self, ctx: &AbacContext) -> Option<PolicyEffect> {
        if !self.enabled {
            return None;
        // Check if action matches (supports wildcards: "secrets:*", "*")
        if !self.action_matches(&ctx.action) {
            return None;
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
    fn action_matches(&self, action: &str) -> bool {
        if self.action == "*" {
            return true;
        if self.action == action {
            return true;
        // "module:*" matches "module:anything"
        if let Some(prefix) = self.action.strip_suffix(":*") {
            if let Some(req_prefix) = action.split(':').next() {
                return prefix == req_prefix;
        false
/// The context provided to the ABAC engine for a single authorization decision.
#[derive(Debug, Clone)]
pub struct AbacContext {
    /// The action being requested, e.g., "secrets:write".
    /// Attributes of the requesting subject (user/service).
    pub subject_attributes: HashMap<String, serde_json::Value>,
    /// Attributes of the resource being accessed.
    pub resource_attributes: HashMap<String, serde_json::Value>,
    /// Environmental attributes (time, IP, region, etc.).
    pub environment: HashMap<String, serde_json::Value>,
impl AbacContext {
    pub fn new(action: &str) -> Self {
        Self {
            action: action.to_string(),
            subject_attributes: HashMap::new(),
            resource_attributes: HashMap::new(),
            environment: HashMap::new(),
    pub fn with_subject(mut self, key: &str, value: impl Into<serde_json::Value>) -> Self {
        self.subject_attributes.insert(key.to_string(), value.into());
        self
    pub fn with_resource(mut self, key: &str, value: impl Into<serde_json::Value>) -> Self {
        self.resource_attributes.insert(key.to_string(), value.into());
        self
    pub fn with_env(mut self, key: &str, value: impl Into<serde_json::Value>) -> Self {
        self.environment.insert(key.to_string(), value.into());
        self
/// Decision returned by ABAC evaluation.
pub enum AbacDecision {
    Deny,
    /// No applicable policy found — fall through to RBAC or default deny.
    NotApplicable,
/// ABAC policy engine.
pub struct AbacEngine {
impl AbacEngine {
    pub fn new() -> Self {
        Self {
            policies: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Load an initial set of policies (e.g., from config file on startup).
    pub fn with_policies(policies: Vec<AbacPolicy>) -> Self {
        let engine = Self::new();
        // Block-on is acceptable here because this is called at startup
        let mut store = engine.policies.blocking_write();
        store.extend(policies);
        drop(store);
        engine
    }

    /// Add or replace a policy at runtime.
    pub async fn upsert_policy(&self, policy: AbacPolicy) {
        let mut store = self.policies.write().await;
        store.retain(|p| p.id != policy.id);
        store.push(policy);
        store.sort_by(|a, b| b.priority.cmp(&a.priority));
    }

    /// Remove a policy by ID.
    pub async fn remove_policy(&self, id: &str) {
        self.policies.write().await.retain(|p| p.id != id);
    }

    /// List all policies.
    pub async fn list_policies(&self) -> Vec<AbacPolicy> {
        self.policies.read().await.clone()
    }

    /// Evaluate all policies for the given action + attributes.
    ///
    /// Evaluation order: highest `priority` first.
    /// First matching DENY wins (deny-override semantics).
    /// If no match: `NoMatch` → caller should fall through to RBAC.
    pub async fn evaluate(
        &self,
        action: &str,
        subject: &SubjectAttributes,
        resource: &ResourceAttributes,
        context: &RequestContext,
    ) -> PolicyDecision {
        let store = self.policies.read().await;

        let mut allow_matched: Option<&AbacPolicy> = None;

        for policy in store.iter() {
            // Action filter
            if policy.action != "*" && policy.action != action {
                continue;
            }

            if !all_conditions_match(&policy.conditions, action, subject, resource, context) {
                continue;
            }

            match policy.effect {
                PolicyEffect::Deny => {
                    return PolicyDecision::Deny {
                        policy_id: policy.id.clone(),
                        reason: policy.description.clone(),
                    };
                }
                PolicyEffect::Allow => {
                    if allow_matched.is_none() {
                        allow_matched = Some(policy);
                    }
    pub async fn add_policy(&self, policy: AbacPolicy) {
        let mut policies = self.policies.write().await;
        policies.push(policy);
        // Sort by priority descending (higher priority evaluated first)
        policies.sort_by(|a, b| b.priority.cmp(&a.priority));
    pub async fn remove_policy(&self, id: Uuid) {
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

        if allow_matched.is_some() {
            PolicyDecision::Allow
        } else {
            PolicyDecision::NoMatch
        if has_allow {
            AbacDecision::Allow
            AbacDecision::NotApplicable
        }
    }
}

impl Default for AbacPolicyEngine {
impl Default for AbacEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Condition evaluation ─────────────────────────────────────────────────────

fn all_conditions_match(
    conditions: &[PolicyCondition],
    _action: &str,
    subject: &SubjectAttributes,
    resource: &ResourceAttributes,
    context: &RequestContext,
) -> bool {
    conditions
        .iter()
        .all(|c| evaluate_condition(c, subject, resource, context))
}

fn evaluate_condition(
    cond: &PolicyCondition,
    subject: &SubjectAttributes,
    resource: &ResourceAttributes,
    context: &RequestContext,
) -> bool {
    let actual = resolve_attribute(&cond.attribute, subject, resource, context);
    let actual = match actual {
        Some(v) => v,
        None => return false, // attribute not present → condition fails
    };

    match cond.operator {
        ConditionOperator::Eq => actual.eq_ignore_ascii_case(&cond.value),
        ConditionOperator::NotEq => !actual.eq_ignore_ascii_case(&cond.value),
        ConditionOperator::In => cond
            .value
            .split(',')
            .map(str::trim)
            .any(|v| actual.eq_ignore_ascii_case(v)),
        ConditionOperator::NotIn => !cond
            .value
            .split(',')
            .map(str::trim)
            .any(|v| actual.eq_ignore_ascii_case(v)),
        ConditionOperator::Contains => actual
            .to_lowercase()
            .contains(&cond.value.to_lowercase()),
        ConditionOperator::StartsWith => actual
            .to_lowercase()
            .starts_with(&cond.value.to_lowercase()),
        ConditionOperator::Lte => {
            let a: f64 = actual.parse().unwrap_or(f64::MAX);
            let b: f64 = cond.value.parse().unwrap_or(f64::MIN);
            a <= b
        }
        ConditionOperator::Gte => {
            let a: f64 = actual.parse().unwrap_or(f64::MIN);
            let b: f64 = cond.value.parse().unwrap_or(f64::MAX);
            a >= b
        }
    }
}

/// Resolve a dot-path attribute reference to a string value.
fn resolve_attribute(
    path: &str,
    subject: &SubjectAttributes,
    resource: &ResourceAttributes,
    context: &RequestContext,
) -> Option<String> {
    match path {
        // Subject attributes
        "subject.department" => subject.department.clone(),
        "subject.team" => subject.team.clone(),
        "subject.location" => subject.location.clone(),
        "subject.employment_type" => subject.employment_type.clone(),
        // Resource attributes
        "resource.environment" => resource.environment.clone(),
        "resource.sensitivity" => resource.sensitivity.clone(),
        "resource.module" => resource.module.clone(),
        "resource.resource_type" => resource.resource_type.clone(),
        "resource.severity" => resource.severity.clone(),
        // Context attributes
        "context.hour_utc" => Some(context.hour_utc.to_string()),
        "context.is_internal_network" => Some(context.is_internal_network.to_string()),
        "context.mfa_verified" => Some(context.mfa_verified.to_string()),
        "context.ip_address" => context.ip_address.map(|ip| ip.to_string()),
        // Custom maps: "subject.custom.foo" or "resource.custom.bar"
        other => {
            if let Some(key) = other.strip_prefix("subject.custom.") {
                subject.custom.get(key).cloned()
            } else if let Some(key) = other.strip_prefix("resource.custom.") {
                resource.custom.get(key).cloned()
            } else {
                None
            }
        }
    }
}

// ─── Built-in example policies ────────────────────────────────────────────────

/// Example policies that ship with the platform.
/// Load with `AbacPolicyEngine::with_policies(example_policies())`.
pub fn example_policies() -> Vec<AbacPolicy> {
    vec![
        AbacPolicy {
            id: "sre-p1-incidents-prod".to_string(),
            description: "Only SRE team members can acknowledge P1 incidents in production"
                .to_string(),
            action: "cave-incidents:manage".to_string(),
            effect: PolicyEffect::Allow,
            priority: 100,
            conditions: vec![
                PolicyCondition {
                    attribute: "subject.team".to_string(),
                    operator: ConditionOperator::Eq,
                    value: "SRE".to_string(),
                },
                PolicyCondition {
                    attribute: "resource.environment".to_string(),
                    operator: ConditionOperator::Eq,
                    value: "production".to_string(),
                },
                PolicyCondition {
                    attribute: "resource.severity".to_string(),
                    operator: ConditionOperator::Eq,
                    value: "P1".to_string(),
                },
            ],
        },
        AbacPolicy {
            id: "deny-restricted-contractor".to_string(),
            description: "Contractors cannot access restricted-sensitivity resources".to_string(),
            action: "*".to_string(),
            effect: PolicyEffect::Deny,
            priority: 200, // evaluated before allows
            conditions: vec![
                PolicyCondition {
                    attribute: "subject.employment_type".to_string(),
                    operator: ConditionOperator::Eq,
                    value: "contractor".to_string(),
                },
                PolicyCondition {
                    attribute: "resource.sensitivity".to_string(),
                    operator: ConditionOperator::Eq,
                    value: "restricted".to_string(),
                },
            ],
        },
        AbacPolicy {
            id: "business-hours-secrets-write".to_string(),
            description: "Secret writes only allowed during business hours (08–18 UTC)".to_string(),
            action: "cave-secrets:write".to_string(),
            effect: PolicyEffect::Deny,
            priority: 150,
            conditions: vec![
                PolicyCondition {
                    attribute: "resource.environment".to_string(),
                    operator: ConditionOperator::Eq,
                    value: "production".to_string(),
                },
                // Deny if hour_utc < 8
                PolicyCondition {
                    attribute: "context.hour_utc".to_string(),
                    operator: ConditionOperator::Lte,
                    value: "7".to_string(),
                },
            ],
        },
    ]
}
#[cfg(test)]
mod tests {
    use super::*;
    fn make_allow_policy(action: &str) -> AbacPolicy {
        AbacPolicy::new("test-allow", action, PolicyEffect::Allow)
    fn make_deny_policy(action: &str) -> AbacPolicy {
        AbacPolicy::new("test-deny", action, PolicyEffect::Deny)
    #[test]
    fn condition_equals_matches() {
        let cond = Condition {
            attribute: "department".to_string(),
            operator: ConditionOperator::Equals,
            value: serde_json::json!("engineering"),
        let mut attrs = HashMap::new();
        attrs.insert("department".to_string(), serde_json::json!("engineering"));
        assert!(cond.evaluate(&attrs));
        attrs.insert("department".to_string(), serde_json::json!("finance"));
        assert!(!cond.evaluate(&attrs));
    #[test]
    fn condition_in_operator() {
        let cond = Condition {
            attribute: "region".to_string(),
            operator: ConditionOperator::In,
            value: serde_json::json!(["eu-west", "eu-central"]),
        let mut attrs = HashMap::new();
        attrs.insert("region".to_string(), serde_json::json!("eu-west"));
        assert!(cond.evaluate(&attrs));
        attrs.insert("region".to_string(), serde_json::json!("us-east"));
        assert!(!cond.evaluate(&attrs));
    #[test]
    fn condition_exists_operator() {
        let cond = Condition {
            attribute: "mfa_verified".to_string(),
            operator: ConditionOperator::Exists,
            value: serde_json::Value::Null,
        let mut attrs = HashMap::new();
        assert!(!cond.evaluate(&attrs));
        attrs.insert("mfa_verified".to_string(), serde_json::json!(true));
        assert!(cond.evaluate(&attrs));
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
    #[tokio::test]
    async fn abac_no_matching_policy_returns_not_applicable() {
        let engine = AbacEngine::new();
        // No policies added
        let ctx = AbacContext::new("flags:write");
        assert_eq!(engine.evaluate(&ctx).await, AbacDecision::NotApplicable);
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
