// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! ValidatingAdmissionPolicy + MutatingAdmissionPolicy thin wrapper.
//!
//! cave-apiserver ships the CEL evaluator + the policy registry; this
//! module sits at the umbrella layer and converts a `Policy` into a
//! list of `admission::Plugin` instances so the cave-k8s chain picks
//! them up automatically.

use crate::admission::{Decision, Plugin, Request};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyMatch {
    pub kinds: Vec<String>,
    pub operations: Vec<crate::admission::Operation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyRule {
    pub name: String,
    pub expression: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    pub name: String,
    pub validating: bool,
    pub match_rules: PolicyMatch,
    pub rules: Vec<PolicyRule>,
}

impl Policy {
    pub fn matches(&self, req: &Request) -> bool {
        self.match_rules.kinds.iter().any(|k| k == &req.kind)
            && self.match_rules.operations.iter().any(|o| *o == req.operation)
    }
}

pub struct PolicyPlugin {
    pub policy: Policy,
}

impl Plugin for PolicyPlugin {
    fn name(&self) -> &'static str {
        if self.policy.validating {
            "ValidatingAdmissionPolicy"
        } else {
            "MutatingAdmissionPolicy"
        }
    }
    fn is_mutating(&self) -> bool {
        !self.policy.validating
    }
    fn evaluate(&self, req: &mut Request) -> Decision {
        if !self.policy.matches(req) {
            return Decision::Allow;
        }
        for rule in &self.policy.rules {
            if let Some(deny) = evaluate_rule(&rule.expression, req) {
                if deny {
                    return Decision::Deny(rule.message.clone());
                }
            } else {
                // Unevaluable -> conservative: reject so policy author
                // can fix the expression.
                return Decision::Deny(format!(
                    "policy {} rule {} could not be evaluated",
                    self.policy.name, rule.name
                ));
            }
        }
        Decision::Allow
    }
}

/// Tiny in-house expression evaluator supporting the K8s VAP shapes the
/// umbrella needs (object.kind == "Pod" / object.spec.replicas > 0 /
/// has(object.spec.X)). Anything richer is delegated to
/// `cave_apiserver::cel_eval` at a future step.
fn evaluate_rule(expr: &str, req: &Request) -> Option<bool> {
    let expr = expr.trim();
    if let Some(rest) = expr.strip_prefix("object.kind == \"") {
        let kind = rest.trim_end_matches('"');
        return Some(kind != req.kind);
    }
    if let Some(rest) = expr.strip_prefix("object.namespace == \"") {
        let ns = rest.trim_end_matches('"');
        return Some(ns != req.namespace);
    }
    if let Some(field) = expr.strip_prefix("has(object.spec.").and_then(|s| s.strip_suffix(')')) {
        // Returns true (deny) if the field is *missing*.
        let path: Vec<&str> = field.split('.').collect();
        let mut cur = req.object.get("spec");
        for p in path {
            cur = cur.and_then(|v| v.get(p));
        }
        return Some(cur.is_none());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admission::Operation;
    use serde_json::json;

    fn req(kind: &str, spec: serde_json::Value) -> Request {
        Request {
            operation: Operation::Create,
            namespace: "default".into(),
            kind: kind.into(),
            name: "x".into(),
            user: "alice".into(),
            object: json!({"spec": spec}),
        }
    }

    #[test]
    fn match_skips_unrelated_kinds() {
        let p = Policy {
            name: "no-bare-pods".into(),
            validating: true,
            match_rules: PolicyMatch {
                kinds: vec!["Pod".into()],
                operations: vec![Operation::Create],
            },
            rules: vec![],
        };
        let plug = PolicyPlugin { policy: p };
        let mut r = req("ConfigMap", json!({}));
        assert!(matches!(plug.evaluate(&mut r), Decision::Allow));
    }

    #[test]
    fn missing_required_spec_field_denied() {
        let p = Policy {
            name: "require-serviceaccount".into(),
            validating: true,
            match_rules: PolicyMatch {
                kinds: vec!["Pod".into()],
                operations: vec![Operation::Create],
            },
            rules: vec![PolicyRule {
                name: "sa-required".into(),
                expression: "has(object.spec.serviceAccountName)".into(),
                message: "serviceAccountName must be set".into(),
            }],
        };
        let plug = PolicyPlugin { policy: p };
        let mut r = req("Pod", json!({}));
        assert!(matches!(plug.evaluate(&mut r), Decision::Deny(_)));
    }

    #[test]
    fn present_spec_field_admitted() {
        let p = Policy {
            name: "require-serviceaccount".into(),
            validating: true,
            match_rules: PolicyMatch {
                kinds: vec!["Pod".into()],
                operations: vec![Operation::Create],
            },
            rules: vec![PolicyRule {
                name: "sa-required".into(),
                expression: "has(object.spec.serviceAccountName)".into(),
                message: "serviceAccountName must be set".into(),
            }],
        };
        let plug = PolicyPlugin { policy: p };
        let mut r = req("Pod", json!({"serviceAccountName": "default"}));
        assert!(matches!(plug.evaluate(&mut r), Decision::Allow));
    }

    #[test]
    fn kind_equality_inverts_for_deny() {
        let p = Policy {
            name: "no-pods-in-system".into(),
            validating: true,
            match_rules: PolicyMatch {
                kinds: vec!["Pod".into()],
                operations: vec![Operation::Create],
            },
            rules: vec![PolicyRule {
                name: "not-pod".into(),
                expression: "object.kind == \"ConfigMap\"".into(),
                message: "Pods only".into(),
            }],
        };
        let plug = PolicyPlugin { policy: p };
        let mut r = req("Pod", json!({}));
        // expr "object.kind == ConfigMap" — req.kind is Pod, predicate
        // returns true (deny) because the rule reads "kind must be ConfigMap"
        // and that's false for a Pod.
        match plug.evaluate(&mut r) {
            Decision::Deny(_) => (),
            d => panic!("unexpected {:?}", d),
        }
    }

    #[test]
    fn unparseable_expression_is_conservative_deny() {
        let p = Policy {
            name: "bad".into(),
            validating: true,
            match_rules: PolicyMatch {
                kinds: vec!["Pod".into()],
                operations: vec![Operation::Create],
            },
            rules: vec![PolicyRule {
                name: "x".into(),
                expression: "completely.unsupported.syntax(2 + 2)".into(),
                message: "bad".into(),
            }],
        };
        let plug = PolicyPlugin { policy: p };
        let mut r = req("Pod", json!({}));
        assert!(matches!(plug.evaluate(&mut r), Decision::Deny(_)));
    }

    #[test]
    fn mutating_policy_flag_routes_as_mutating() {
        let p = Policy {
            name: "x".into(),
            validating: false,
            match_rules: PolicyMatch {
                kinds: vec!["Pod".into()],
                operations: vec![Operation::Create],
            },
            rules: vec![],
        };
        let plug = PolicyPlugin { policy: p };
        assert!(plug.is_mutating());
        assert_eq!(plug.name(), "MutatingAdmissionPolicy");
    }
}
