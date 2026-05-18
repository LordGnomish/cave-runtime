// SPDX-License-Identifier: AGPL-3.0-or-later
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
        }
    }
}

// ─── Policy model ─────────────────────────────────────────────────────────────

/// Effect of a policy: explicit allow or explicit deny.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
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
                }
            }
        }

        if allow_matched.is_some() {
            PolicyDecision::Allow
        } else {
            PolicyDecision::NoMatch
        }
    }
}

impl Default for AbacPolicyEngine {
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
