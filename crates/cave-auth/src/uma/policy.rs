// SPDX-License-Identifier: AGPL-3.0-or-later
//
// UMA 2.0 Policy Evaluation — scope / role / time / aggregate.
//
// Upstream: keycloak/keycloak  b825ba97b489d715f7ca1984c19bd95afb355a38
//   services/src/main/java/org/keycloak/authorization/policy/evaluation/DefaultPolicyEvaluator.java
//   services/src/main/java/org/keycloak/authorization/policy/provider/
//
// Out of scope (Phase 2, `status="missing"` in parity manifest):
//   - JS policies (Keycloak's Nashorn engine, deprecated upstream)
//   - Drools-based rule policies
//   - Group-aggregate policies
//
// Policies are composable. The result is `Permit` if applicable policies
// permit, else `Deny`.

use serde::{Deserialize, Serialize};

/// Subject context evaluated against policies.
#[derive(Debug, Clone, Default)]
pub struct EvalContext {
    pub sub: String,
    pub roles: Vec<String>,
    pub scopes: Vec<String>,
    /// `now` is supplied by the caller so tests can pin the clock.
    pub now_unix: i64,
}

/// Single policy clause.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Policy {
    /// `Scope { required_scopes }` — every listed scope must be present in
    /// the subject's `scopes`.
    Scope { required_scopes: Vec<String> },
    /// `Role { required_roles, logic }` — `logic` defaults to "positive".
    Role {
        required_roles: Vec<String>,
        #[serde(default = "default_logic")]
        logic: PolicyLogic,
    },
    /// `Time { not_before, not_after }` — both are unix epoch seconds.
    Time {
        #[serde(default)]
        not_before: Option<i64>,
        #[serde(default)]
        not_after: Option<i64>,
    },
    /// Logical AND of inner policies.
    All { policies: Vec<Policy> },
    /// Logical OR — at least one inner policy must permit.
    Any { policies: Vec<Policy> },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PolicyLogic {
    /// Subject must have all listed roles.
    Positive,
    /// Subject must NOT have any of the listed roles.
    Negative,
}

fn default_logic() -> PolicyLogic {
    PolicyLogic::Positive
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Permit,
    Deny,
}

impl Policy {
    pub fn evaluate(&self, ctx: &EvalContext) -> Decision {
        match self {
            Policy::Scope { required_scopes } => {
                if required_scopes.iter().all(|s| ctx.scopes.iter().any(|cs| cs == s)) {
                    Decision::Permit
                } else {
                    Decision::Deny
                }
            }
            Policy::Role { required_roles, logic } => {
                let has_any =
                    required_roles.iter().any(|r| ctx.roles.iter().any(|cr| cr == r));
                let has_all =
                    required_roles.iter().all(|r| ctx.roles.iter().any(|cr| cr == r));
                let permit = match logic {
                    PolicyLogic::Positive => has_all,
                    PolicyLogic::Negative => !has_any,
                };
                if permit { Decision::Permit } else { Decision::Deny }
            }
            Policy::Time { not_before, not_after } => {
                let ok_before = not_before.map(|nb| ctx.now_unix >= nb).unwrap_or(true);
                let ok_after = not_after.map(|na| ctx.now_unix <= na).unwrap_or(true);
                if ok_before && ok_after { Decision::Permit } else { Decision::Deny }
            }
            Policy::All { policies } => {
                if policies.iter().all(|p| p.evaluate(ctx) == Decision::Permit) {
                    Decision::Permit
                } else {
                    Decision::Deny
                }
            }
            Policy::Any { policies } => {
                if policies.iter().any(|p| p.evaluate(ctx) == Decision::Permit) {
                    Decision::Permit
                } else {
                    Decision::Deny
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_with(roles: &[&str], scopes: &[&str]) -> EvalContext {
        EvalContext {
            sub: "alice".into(),
            roles: roles.iter().map(|s| s.to_string()).collect(),
            scopes: scopes.iter().map(|s| s.to_string()).collect(),
            now_unix: 1_700_000_000,
        }
    }

    // upstream: keycloak ScopePolicyProvider — every required scope must be
    // present.
    #[test]
    fn scope_policy_permits_when_all_present() {
        let p = Policy::Scope { required_scopes: vec!["view".into(), "edit".into()] };
        assert_eq!(p.evaluate(&ctx_with(&[], &["view", "edit", "share"])), Decision::Permit);
    }

    // upstream: keycloak ScopePolicyProvider — missing scope denies.
    #[test]
    fn scope_policy_denies_when_missing() {
        let p = Policy::Scope { required_scopes: vec!["edit".into()] };
        assert_eq!(p.evaluate(&ctx_with(&[], &["view"])), Decision::Deny);
    }

    // upstream: keycloak RolePolicyProvider — positive logic = all roles
    // required.
    #[test]
    fn role_policy_positive_requires_all() {
        let p = Policy::Role {
            required_roles: vec!["admin".into(), "auditor".into()],
            logic: PolicyLogic::Positive,
        };
        assert_eq!(p.evaluate(&ctx_with(&["admin", "auditor"], &[])), Decision::Permit);
        assert_eq!(p.evaluate(&ctx_with(&["admin"], &[])), Decision::Deny);
    }

    // upstream: keycloak RolePolicyProvider — negative logic = none of the
    // listed roles may be present.
    #[test]
    fn role_policy_negative_excludes() {
        let p = Policy::Role {
            required_roles: vec!["blacklisted".into()],
            logic: PolicyLogic::Negative,
        };
        assert_eq!(p.evaluate(&ctx_with(&["user"], &[])), Decision::Permit);
        assert_eq!(p.evaluate(&ctx_with(&["blacklisted"], &[])), Decision::Deny);
    }

    // upstream: keycloak TimePolicyProvider — `now` must be within
    // [not_before, not_after].
    #[test]
    fn time_policy_window() {
        let p = Policy::Time {
            not_before: Some(1_699_000_000),
            not_after: Some(1_701_000_000),
        };
        assert_eq!(p.evaluate(&ctx_with(&[], &[])), Decision::Permit);

        let early = EvalContext {
            sub: "x".into(),
            roles: vec![],
            scopes: vec![],
            now_unix: 1_000_000_000,
        };
        assert_eq!(p.evaluate(&early), Decision::Deny);
    }

    // upstream: keycloak AggregatePolicyProvider — AND of two policies.
    #[test]
    fn all_aggregator_is_logical_and() {
        let p = Policy::All {
            policies: vec![
                Policy::Scope { required_scopes: vec!["view".into()] },
                Policy::Role {
                    required_roles: vec!["admin".into()],
                    logic: PolicyLogic::Positive,
                },
            ],
        };
        assert_eq!(p.evaluate(&ctx_with(&["admin"], &["view"])), Decision::Permit);
        assert_eq!(p.evaluate(&ctx_with(&["admin"], &[])), Decision::Deny);
        assert_eq!(p.evaluate(&ctx_with(&[], &["view"])), Decision::Deny);
    }

    // upstream: keycloak AggregatePolicyProvider — OR.
    #[test]
    fn any_aggregator_is_logical_or() {
        let p = Policy::Any {
            policies: vec![
                Policy::Role { required_roles: vec!["admin".into()], logic: PolicyLogic::Positive },
                Policy::Scope { required_scopes: vec!["god-mode".into()] },
            ],
        };
        assert_eq!(p.evaluate(&ctx_with(&["admin"], &[])), Decision::Permit);
        assert_eq!(p.evaluate(&ctx_with(&[], &["god-mode"])), Decision::Permit);
        assert_eq!(p.evaluate(&ctx_with(&["user"], &["other"])), Decision::Deny);
    }

    // upstream: keycloak — empty Time bounds permit unconditionally.
    #[test]
    fn time_policy_open_bounds_always_permit() {
        let p = Policy::Time { not_before: None, not_after: None };
        assert_eq!(p.evaluate(&ctx_with(&[], &[])), Decision::Permit);
    }

    // upstream: keycloak ScopePolicyProvider — empty required_scopes vacuously
    // permits (matches keycloak's default "no constraints = permit").
    #[test]
    fn scope_policy_empty_required_permits() {
        let p = Policy::Scope { required_scopes: vec![] };
        assert_eq!(p.evaluate(&ctx_with(&[], &[])), Decision::Permit);
    }
}
