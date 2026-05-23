// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Authentication flow executor — `AuthenticationProcessor` parity.
//!
//! A flow is an ordered list of `(Authenticator, Requirement)` pairs.
//! The executor walks the list, calling each authenticator with a
//! mutable context; required steps must succeed, alternatives must
//! produce at least one success per branch.
//!
//! Upstream: `services/src/main/java/org/keycloak/authentication/AuthenticationProcessor.java`
//! + `services/src/main/java/org/keycloak/authentication/DefaultAuthenticationFlow.java`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Requirement {
    Required,
    Alternative,
    Conditional,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthStatus {
    Success,
    Challenge,
    AttemptedFailure,
    Skipped,
    Reset,
}

/// Per-execution context — caller carries facts (presented credentials,
/// session metadata, IP, MFA presence) that each authenticator inspects.
#[derive(Debug, Clone, Default)]
pub struct AuthContext {
    pub facts: BTreeMap<String, String>,
    pub successes: Vec<String>,
    pub failures: Vec<String>,
}

impl AuthContext {
    pub fn fact(&self, k: &str) -> Option<&str> {
        self.facts.get(k).map(|s| s.as_str())
    }

    pub fn set(&mut self, k: &str, v: &str) {
        self.facts.insert(k.to_string(), v.to_string());
    }
}

/// Authenticator identifier — string id lets the registry decouple from
/// the executor (mirrors Keycloak's `Authenticator.getId()`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthenticatorId(pub String);

/// A single step in a flow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthStep {
    pub authenticator: AuthenticatorId,
    pub requirement: Requirement,
}

pub type AuthFn = fn(&mut AuthContext) -> AuthStatus;

/// Executor walks the flow steps and returns an overall status.
pub struct FlowExecutor {
    pub registry: BTreeMap<String, AuthFn>,
}

impl FlowExecutor {
    pub fn new() -> Self {
        Self { registry: BTreeMap::new() }
    }

    pub fn register(&mut self, id: &str, f: AuthFn) {
        self.registry.insert(id.to_string(), f);
    }

    /// Execute the flow. Semantics:
    ///   * `Required` step → must return Success; failure aborts.
    ///   * `Alternative` step → at least one Success among the contiguous
    ///     run of alternatives is sufficient; if none succeeds the flow fails.
    ///   * `Conditional` step → behaves like Required when the matching
    ///     fact is present; otherwise Skipped.
    ///   * `Disabled` step → Skipped unconditionally.
    pub fn execute(&self, flow: &[AuthStep], ctx: &mut AuthContext) -> AuthStatus {
        let mut i = 0;
        while i < flow.len() {
            let step = &flow[i];
            match step.requirement {
                Requirement::Disabled => {
                    i += 1;
                    continue;
                }
                Requirement::Conditional => {
                    if ctx.fact(&format!("cond:{}", step.authenticator.0)).is_none() {
                        i += 1;
                        continue;
                    }
                    let f = match self.registry.get(&step.authenticator.0) {
                        Some(f) => *f,
                        None => return AuthStatus::AttemptedFailure,
                    };
                    let r = f(ctx);
                    if r != AuthStatus::Success {
                        return AuthStatus::AttemptedFailure;
                    }
                    ctx.successes.push(step.authenticator.0.clone());
                    i += 1;
                }
                Requirement::Required => {
                    let f = match self.registry.get(&step.authenticator.0) {
                        Some(f) => *f,
                        None => return AuthStatus::AttemptedFailure,
                    };
                    let r = f(ctx);
                    if r != AuthStatus::Success {
                        ctx.failures.push(step.authenticator.0.clone());
                        return AuthStatus::AttemptedFailure;
                    }
                    ctx.successes.push(step.authenticator.0.clone());
                    i += 1;
                }
                Requirement::Alternative => {
                    // run alternatives as a contiguous block
                    let mut j = i;
                    let mut block_ok = false;
                    while j < flow.len() && matches!(flow[j].requirement, Requirement::Alternative) {
                        let step = &flow[j];
                        if let Some(f) = self.registry.get(&step.authenticator.0).copied() {
                            let r = f(ctx);
                            if r == AuthStatus::Success {
                                ctx.successes.push(step.authenticator.0.clone());
                                block_ok = true;
                                break;
                            } else {
                                ctx.failures.push(step.authenticator.0.clone());
                            }
                        }
                        j += 1;
                    }
                    if !block_ok {
                        return AuthStatus::AttemptedFailure;
                    }
                    // advance past the alt block
                    while i < flow.len() && matches!(flow[i].requirement, Requirement::Alternative) {
                        i += 1;
                    }
                }
            }
        }
        AuthStatus::Success
    }
}

impl Default for FlowExecutor {
    fn default() -> Self {
        Self::new()
    }
}

/// Required action — Keycloak's `RequiredActionProvider` (UPDATE_PASSWORD,
/// VERIFY_EMAIL, CONFIGURE_TOTP, etc.). Surfaced after login so the
/// caller can drive the user through the action UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequiredAction {
    UpdatePassword,
    VerifyEmail,
    ConfigureTotp,
    WebauthnRegister,
    TermsAndConditions,
}

pub fn pending_required_actions(user_attrs: &BTreeMap<String, Vec<String>>) -> Vec<RequiredAction> {
    user_attrs
        .get("required_actions")
        .into_iter()
        .flatten()
        .filter_map(|s| match s.as_str() {
            "UPDATE_PASSWORD" => Some(RequiredAction::UpdatePassword),
            "VERIFY_EMAIL" => Some(RequiredAction::VerifyEmail),
            "CONFIGURE_TOTP" => Some(RequiredAction::ConfigureTotp),
            "WEBAUTHN_REGISTER" => Some(RequiredAction::WebauthnRegister),
            "TERMS_AND_CONDITIONS" => Some(RequiredAction::TermsAndConditions),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn always_ok(_: &mut AuthContext) -> AuthStatus {
        AuthStatus::Success
    }
    fn always_fail(_: &mut AuthContext) -> AuthStatus {
        AuthStatus::AttemptedFailure
    }
    fn password_ok_when_fact_set(ctx: &mut AuthContext) -> AuthStatus {
        if ctx.fact("password").is_some() {
            AuthStatus::Success
        } else {
            AuthStatus::AttemptedFailure
        }
    }

    #[test]
    fn required_failure_short_circuits() {
        let mut e = FlowExecutor::new();
        e.register("a", always_fail);
        e.register("b", always_ok);
        let flow = vec![
            AuthStep { authenticator: AuthenticatorId("a".into()), requirement: Requirement::Required },
            AuthStep { authenticator: AuthenticatorId("b".into()), requirement: Requirement::Required },
        ];
        let mut ctx = AuthContext::default();
        assert_eq!(e.execute(&flow, &mut ctx), AuthStatus::AttemptedFailure);
        assert!(ctx.failures.contains(&"a".to_string()));
        assert!(!ctx.successes.contains(&"b".to_string()));
    }

    #[test]
    fn alternative_block_succeeds_if_any_passes() {
        let mut e = FlowExecutor::new();
        e.register("alt1", always_fail);
        e.register("alt2", always_ok);
        e.register("alt3", always_ok);
        let flow = vec![
            AuthStep { authenticator: AuthenticatorId("alt1".into()), requirement: Requirement::Alternative },
            AuthStep { authenticator: AuthenticatorId("alt2".into()), requirement: Requirement::Alternative },
            AuthStep { authenticator: AuthenticatorId("alt3".into()), requirement: Requirement::Alternative },
        ];
        let mut ctx = AuthContext::default();
        assert_eq!(e.execute(&flow, &mut ctx), AuthStatus::Success);
        assert!(ctx.successes.contains(&"alt2".to_string()));
    }

    #[test]
    fn conditional_only_runs_when_fact_present() {
        let mut e = FlowExecutor::new();
        e.register("cond1", always_fail);
        let flow = vec![AuthStep {
            authenticator: AuthenticatorId("cond1".into()),
            requirement: Requirement::Conditional,
        }];
        let mut ctx = AuthContext::default();
        assert_eq!(e.execute(&flow, &mut ctx), AuthStatus::Success);
        ctx.set("cond:cond1", "y");
        assert_eq!(e.execute(&flow, &mut ctx), AuthStatus::AttemptedFailure);
    }

    #[test]
    fn disabled_step_is_skipped() {
        let mut e = FlowExecutor::new();
        e.register("a", always_fail);
        let flow = vec![AuthStep {
            authenticator: AuthenticatorId("a".into()),
            requirement: Requirement::Disabled,
        }];
        let mut ctx = AuthContext::default();
        assert_eq!(e.execute(&flow, &mut ctx), AuthStatus::Success);
    }

    #[test]
    fn password_then_required_otp_chain() {
        let mut e = FlowExecutor::new();
        e.register("password", password_ok_when_fact_set);
        e.register("otp", always_ok);
        let flow = vec![
            AuthStep { authenticator: AuthenticatorId("password".into()), requirement: Requirement::Required },
            AuthStep { authenticator: AuthenticatorId("otp".into()), requirement: Requirement::Required },
        ];
        let mut ctx = AuthContext::default();
        ctx.set("password", "hunter2-cave");
        assert_eq!(e.execute(&flow, &mut ctx), AuthStatus::Success);
        assert_eq!(ctx.successes, vec!["password", "otp"]);
    }

    #[test]
    fn pending_required_actions_parse_attributes() {
        let mut attrs = BTreeMap::new();
        attrs.insert(
            "required_actions".to_string(),
            vec!["UPDATE_PASSWORD".into(), "VERIFY_EMAIL".into(), "BAD_KEY".into()],
        );
        let r = pending_required_actions(&attrs);
        assert_eq!(r.len(), 2);
        assert!(r.contains(&RequiredAction::UpdatePassword));
        assert!(r.contains(&RequiredAction::VerifyEmail));
    }
}
