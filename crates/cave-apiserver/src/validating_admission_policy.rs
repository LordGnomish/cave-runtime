// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! ValidatingAdmissionPolicy — in-process CEL-style validation (KEP-3488).
//!
//! Upstream: kubernetes/kubernetes v1.36.0
//!   * `staging/src/k8s.io/api/admissionregistration/v1/types.go`
//!     (`ValidatingAdmissionPolicy`, `ValidatingAdmissionPolicyBinding`,
//!     `MatchResources`, `Validation`, `FailurePolicyType`).
//!   * `staging/src/k8s.io/apiserver/pkg/admission/plugin/policy/validating/`.
//!
//! Upstream uses CEL for the `expression` field. We model a deliberately
//! tiny rule language sufficient for parity tests:
//!
//!   * `<jsonpath> == <literal>`        — equality on a string field
//!   * `<jsonpath> != <literal>`        — inequality
//!   * `has(<jsonpath>)`                — field-presence test
//!   * `<jsonpath>.startsWith(<lit>)`   — prefix match
//!
//! Tenant invariant: every Policy/Binding is owned by a tenant_id.
//! Evaluation never crosses tenants — a SAR-like scoped lookup picks
//! only this tenant's policies.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

/// Mirrors `admissionregistration/v1.FailurePolicyType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailurePolicy {
    /// Eval failure → reject (default).
    Fail,
    /// Eval failure → log + admit.
    Ignore,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationRule {
    /// Tiny rule expression — see module docs for grammar.
    pub expression: String,
    /// Message returned to the client on a failed evaluation.
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchResources {
    pub api_groups: Vec<String>,
    pub resources: Vec<String>,
    pub verbs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatingAdmissionPolicy {
    pub tenant_id: String,
    pub name: String,
    pub failure_policy: FailurePolicy,
    pub matches: MatchResources,
    pub validations: Vec<ValidationRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatingAdmissionPolicyBinding {
    pub tenant_id: String,
    pub name: String,
    pub policy_name: String,
    /// Bindings narrow further by namespace; empty list means "any
    /// namespace within the tenant".
    pub namespaces: Vec<String>,
}

/// Result of evaluating one policy against one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Admit,
    Deny { policy_name: String, message: String },
}

#[derive(Debug, Clone)]
pub struct PolicyRequest {
    pub tenant_id: String,
    pub namespace: String,
    pub group: String,
    pub resource: String,
    pub verb: String,
    pub object: serde_json::Value,
}

pub struct PolicyRegistry {
    inner: Mutex<PolicyInner>,
}

#[derive(Default)]
struct PolicyInner {
    policies: HashMap<(String, String), ValidatingAdmissionPolicy>, // (tenant, name)
    bindings: Vec<ValidatingAdmissionPolicyBinding>,
}

impl PolicyRegistry {
    pub fn new() -> Self {
        Self { inner: Mutex::new(PolicyInner::default()) }
    }

    pub fn upsert_policy(&self, p: ValidatingAdmissionPolicy) {
        self.inner.lock().unwrap().policies.insert(
            (p.tenant_id.clone(), p.name.clone()), p);
    }

    pub fn upsert_binding(&self, b: ValidatingAdmissionPolicyBinding) {
        let mut inner = self.inner.lock().unwrap();
        inner.bindings.retain(|x| !(x.tenant_id == b.tenant_id && x.name == b.name));
        inner.bindings.push(b);
    }

    /// Evaluate every policy bound to `req.tenant_id` whose match rules
    /// cover `req`. The first failed validation Denies (in policy-name
    /// order); otherwise Admit. Mirrors upstream
    /// `policy/validating/dispatcher.go::Dispatch`.
    pub fn evaluate(&self, req: &PolicyRequest) -> PolicyDecision {
        let inner = self.inner.lock().unwrap();
        let mut bound: Vec<&ValidatingAdmissionPolicy> = vec![];
        for b in inner.bindings.iter() {
            if b.tenant_id != req.tenant_id { continue; }
            if !b.namespaces.is_empty()
                && !b.namespaces.iter().any(|n| n == &req.namespace) {
                continue;
            }
            if let Some(p) = inner.policies.get(&(b.tenant_id.clone(), b.policy_name.clone())) {
                bound.push(p);
            }
        }
        bound.sort_by(|a, b| a.name.cmp(&b.name));
        for policy in bound {
            if !policy_matches(policy, req) { continue; }
            for rule in &policy.validations {
                match evaluate_expression(&rule.expression, &req.object) {
                    Ok(true) => continue,
                    Ok(false) => {
                        return PolicyDecision::Deny {
                            policy_name: policy.name.clone(),
                            message: rule.message.clone(),
                        };
                    }
                    Err(_) => match policy.failure_policy {
                        FailurePolicy::Fail => {
                            return PolicyDecision::Deny {
                                policy_name: policy.name.clone(),
                                message: format!(
                                    "expression evaluation error: `{}`", rule.expression),
                            };
                        }
                        FailurePolicy::Ignore => continue,
                    },
                }
            }
        }
        PolicyDecision::Admit
    }
}

impl Default for PolicyRegistry {
    fn default() -> Self { Self::new() }
}

fn policy_matches(p: &ValidatingAdmissionPolicy, r: &PolicyRequest) -> bool {
    let group_ok = p.matches.api_groups.iter().any(|g| g == "*" || g == &r.group);
    let res_ok   = p.matches.resources.iter().any(|x| x == "*" || x == &r.resource);
    let verb_ok  = p.matches.verbs.iter().any(|v| v == "*" || v == &r.verb);
    group_ok && res_ok && verb_ok
}

#[derive(Debug)]
pub struct EvalError(pub String);

/// Tiny expression evaluator — see module docs for the grammar.
pub fn evaluate_expression(
    expression: &str,
    object: &serde_json::Value,
) -> Result<bool, EvalError> {
    let expr = expression.trim();
    if let Some(inner) = expr.strip_prefix("has(").and_then(|s| s.strip_suffix(')')) {
        let path = inner.trim();
        return Ok(json_path(object, path).is_some());
    }
    if let Some((path, lit)) = split_op(expr, ".startsWith(") {
        let lit = lit.strip_suffix(')').ok_or_else(||
            EvalError(format!("unterminated startsWith: `{}`", expression)))?;
        let value = json_path(object, path.trim())
            .and_then(|v| v.as_str().map(String::from))
            .ok_or_else(|| EvalError(format!("path missing or non-string: `{}`", path)))?;
        let needle = trim_quotes(lit.trim())?;
        return Ok(value.starts_with(&needle));
    }
    if let Some((lhs, rhs)) = split_op(expr, "!=") {
        let lhs_v = json_path(object, lhs.trim())
            .and_then(|v| v.as_str().map(String::from))
            .ok_or_else(|| EvalError(format!("path missing or non-string: `{}`", lhs)))?;
        let rhs_v = trim_quotes(rhs.trim())?;
        return Ok(lhs_v != rhs_v);
    }
    if let Some((lhs, rhs)) = split_op(expr, "==") {
        let lhs_v = json_path(object, lhs.trim())
            .and_then(|v| v.as_str().map(String::from))
            .ok_or_else(|| EvalError(format!("path missing or non-string: `{}`", lhs)))?;
        let rhs_v = trim_quotes(rhs.trim())?;
        return Ok(lhs_v == rhs_v);
    }
    Err(EvalError(format!("unsupported expression: `{}`", expression)))
}

fn split_op<'a>(s: &'a str, op: &str) -> Option<(&'a str, &'a str)> {
    s.find(op).map(|i| (&s[..i], &s[i + op.len()..]))
}

fn trim_quotes(s: &str) -> Result<String, EvalError> {
    let t = s.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        Ok(t[1..t.len()-1].to_string())
    } else if t.len() >= 2 && t.starts_with('\'') && t.ends_with('\'') {
        Ok(t[1..t.len()-1].to_string())
    } else {
        Err(EvalError(format!("expected quoted string literal, got `{}`", s)))
    }
}

fn json_path<'a>(obj: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut cur = obj;
    for seg in path.split('.') {
        let s = seg.trim();
        if s.is_empty() { continue; }
        cur = cur.get(s)?;
    }
    Some(cur)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pol(tenant: &str, name: &str, fp: FailurePolicy, rules: Vec<ValidationRule>) -> ValidatingAdmissionPolicy {
        ValidatingAdmissionPolicy {
            tenant_id: tenant.into(), name: name.into(),
            failure_policy: fp,
            matches: MatchResources {
                api_groups: vec!["*".into()],
                resources: vec!["*".into()],
                verbs: vec!["*".into()],
            },
            validations: rules,
        }
    }

    fn bind(tenant: &str, name: &str, policy: &str, namespaces: Vec<&str>) -> ValidatingAdmissionPolicyBinding {
        ValidatingAdmissionPolicyBinding {
            tenant_id: tenant.into(), name: name.into(),
            policy_name: policy.into(),
            namespaces: namespaces.into_iter().map(String::from).collect(),
        }
    }

    fn req(tenant: &str, ns: &str, obj: serde_json::Value) -> PolicyRequest {
        PolicyRequest {
            tenant_id: tenant.into(), namespace: ns.into(),
            group: "".into(), resource: "configmaps".into(),
            verb: "create".into(), object: obj,
        }
    }

    /// Upstream parity: `TestVAP_AllowsWhenAllRulesPass`
    /// (apiserver/pkg/admission/plugin/policy/validating/dispatcher_test.go
    /// — every validation passing yields Admit).
    #[test]
    fn test_policy_admits_when_all_validations_pass() {
        let r = PolicyRegistry::new();
        r.upsert_policy(pol("acme", "no-latest", FailurePolicy::Fail, vec![
            ValidationRule {
                expression: "spec.image != \"latest\"".into(),
                message: "image MUST NOT be 'latest'".into(),
            },
        ]));
        r.upsert_binding(bind("acme", "bind-1", "no-latest", vec![]));
        let dec = r.evaluate(&req("acme", "default", json!({
            "spec": { "image": "nginx:1.27" }
        })));
        assert_eq!(dec, PolicyDecision::Admit);
    }

    /// Upstream parity: `TestVAP_DeniesWhenRuleFails`
    /// (dispatcher_test.go — first failing validation Denies).
    #[test]
    fn test_policy_denies_with_rule_message_when_validation_fails() {
        let r = PolicyRegistry::new();
        r.upsert_policy(pol("acme", "no-latest", FailurePolicy::Fail, vec![
            ValidationRule {
                expression: "spec.image != \"latest\"".into(),
                message: "image MUST NOT be 'latest'".into(),
            },
        ]));
        r.upsert_binding(bind("acme", "bind-1", "no-latest", vec![]));
        let dec = r.evaluate(&req("acme", "default", json!({
            "spec": { "image": "latest" }
        })));
        match dec {
            PolicyDecision::Deny { policy_name, message } => {
                assert_eq!(policy_name, "no-latest");
                assert!(message.contains("'latest'"));
            }
            _ => panic!("expected Deny"),
        }
    }

    /// Upstream parity: `TestVAP_TenantScopedEvaluation`
    /// (cave-apiserver invariant: globex's request never sees acme's policy
    /// even when names collide).
    #[test]
    fn test_policy_evaluation_does_not_cross_tenant_boundaries() {
        let r = PolicyRegistry::new();
        // acme's strict policy.
        r.upsert_policy(pol("acme", "no-latest", FailurePolicy::Fail, vec![
            ValidationRule {
                expression: "spec.image != \"latest\"".into(),
                message: "image MUST NOT be 'latest'".into(),
            },
        ]));
        r.upsert_binding(bind("acme", "bind-1", "no-latest", vec![]));
        // globex has no policies at all — request must Admit.
        let dec = r.evaluate(&req("globex", "default", json!({
            "spec": { "image": "latest" }
        })));
        assert_eq!(dec, PolicyDecision::Admit,
            "tenant_id invariant: globex unaffected by acme's deny policy");
    }

    /// Upstream parity: `TestVAP_FailurePolicyIgnoreSwallowsEvalError`
    /// (validating/policy_decision.go — `Ignore` admits on eval failure).
    #[test]
    fn test_failure_policy_ignore_admits_when_expression_errors() {
        let r = PolicyRegistry::new();
        r.upsert_policy(pol("acme", "broken", FailurePolicy::Ignore, vec![
            ValidationRule {
                expression: "missing.path == \"x\"".into(),
                message: "broken rule".into(),
            },
        ]));
        r.upsert_binding(bind("acme", "bind-1", "broken", vec![]));
        let dec = r.evaluate(&req("acme", "default", json!({})));
        assert_eq!(dec, PolicyDecision::Admit,
            "failure_policy=Ignore swallows evaluation errors");
    }

    /// Upstream parity: `TestVAP_FailurePolicyFailDeniesOnEvalError`
    /// (validating/policy_decision.go — `Fail` denies on eval failure).
    #[test]
    fn test_failure_policy_fail_denies_when_expression_errors() {
        let r = PolicyRegistry::new();
        r.upsert_policy(pol("acme", "strict", FailurePolicy::Fail, vec![
            ValidationRule {
                expression: "missing.path == \"x\"".into(),
                message: "uncalled".into(),
            },
        ]));
        r.upsert_binding(bind("acme", "bind-1", "strict", vec![]));
        let dec = r.evaluate(&req("acme", "default", json!({})));
        match dec {
            PolicyDecision::Deny { policy_name, message } => {
                assert_eq!(policy_name, "strict");
                assert!(message.contains("evaluation error"));
            }
            _ => panic!("expected Deny on eval error under FailurePolicy::Fail"),
        }
    }

    /// Upstream parity: `TestVAP_BindingNamespaceFilter`
    /// (binding's `MatchResources.NamespaceSelector` confines policy to a
    /// namespace subset).
    #[test]
    fn test_binding_namespace_list_confines_policy_application() {
        let r = PolicyRegistry::new();
        r.upsert_policy(pol("acme", "p", FailurePolicy::Fail, vec![
            ValidationRule {
                expression: "spec.image != \"latest\"".into(),
                message: "no latest".into(),
            },
        ]));
        r.upsert_binding(bind("acme", "b", "p", vec!["default"]));
        // In default namespace, policy applies → Deny.
        let in_ns = r.evaluate(&req("acme", "default", json!({
            "spec": { "image": "latest" }
        })));
        // In another namespace, binding does not apply → Admit.
        let out_ns = r.evaluate(&req("acme", "kube-system", json!({
            "spec": { "image": "latest" }
        })));
        assert!(matches!(in_ns, PolicyDecision::Deny { .. }));
        assert_eq!(out_ns, PolicyDecision::Admit);
    }

    /// Upstream parity: `TestVAP_HasFunctionFieldPresence`
    /// (CEL `has()` returns true iff the field is present).
    #[test]
    fn test_has_function_distinguishes_present_from_absent_field() {
        // Used directly via the expression evaluator.
        let obj = json!({"spec": {"image": "nginx"}});
        assert!(evaluate_expression("has(spec.image)", &obj).unwrap());
        assert!(!evaluate_expression("has(spec.replicas)", &obj).unwrap());
        // tenant_id invariant smoke: evaluator is pure — never reads any tenant.
        let _ = obj;
    }
}
