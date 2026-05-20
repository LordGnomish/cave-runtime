// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../authorization/policy/evaluation/DefaultPolicyEvaluator.java + Kantara UMA-FedAuthz §3.2
//
//! UMA 2.0 policy decision point.
//!
//! Inputs:
//!   - Permission requests bundled in a ticket.
//!   - The requesting party identity (sub claim) + pushed claims (from `claim_token`).
//!   - Per-resource policies (resource owner consent + scope grants).
//!
//! Output: a `PolicyDecision` carrying which (resource, scope) tuples are
//! granted vs denied.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::claim_token::PushedClaims;
use super::permission_ticket::PermissionTicket;

/// One scope grant per (resource, scope).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ScopeGrant {
    pub resource_id: String,
    pub scope: String,
}

/// Per-resource policy — the resource owner has consented to allow `subject`
/// to use any of the listed scopes. Optionally a `required_claim` may be
/// pushed via claim_token.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ResourcePolicy {
    pub resource_id: String,
    /// `(subject, scopes_allowed)`.
    pub subject_grants: HashMap<String, Vec<String>>,
    /// Required pushed claims — all must be present + match.
    pub required_claims: HashMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PolicyDecision {
    pub granted: Vec<ScopeGrant>,
    pub denied: Vec<ScopeGrant>,
}

impl PolicyDecision {
    pub fn is_fully_granted(&self) -> bool {
        self.denied.is_empty() && !self.granted.is_empty()
    }
}

pub struct PolicyEngine {
    policies: Mutex<HashMap<String, ResourcePolicy>>,
}

impl Default for PolicyEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl PolicyEngine {
    pub fn new() -> Self {
        Self {
            policies: Mutex::new(HashMap::new()),
        }
    }

    pub fn upsert(&self, policy: ResourcePolicy) {
        self.policies
            .lock()
            .unwrap()
            .insert(policy.resource_id.clone(), policy);
    }

    /// Evaluates a ticket against the configured policies + pushed claims.
    pub fn evaluate(
        &self,
        ticket: &PermissionTicket,
        subject: &str,
        pushed: &PushedClaims,
    ) -> PolicyDecision {
        let policies = self.policies.lock().unwrap();
        let mut granted = Vec::new();
        let mut denied = Vec::new();

        for req in &ticket.permissions {
            let policy = match policies.get(&req.resource_id) {
                Some(p) => p,
                None => {
                    // No policy → deny everything for this resource.
                    for scope in &req.resource_scopes {
                        denied.push(ScopeGrant {
                            resource_id: req.resource_id.clone(),
                            scope: scope.clone(),
                        });
                    }
                    continue;
                }
            };

            // Claim gating
            let claims_ok = policy
                .required_claims
                .iter()
                .all(|(k, v)| pushed.get(k).map(|x| x == v).unwrap_or(false));

            let allowed_scopes = policy
                .subject_grants
                .get(subject)
                .cloned()
                .unwrap_or_default();

            for scope in &req.resource_scopes {
                let in_grant = allowed_scopes.iter().any(|s| s == scope);
                let g = ScopeGrant {
                    resource_id: req.resource_id.clone(),
                    scope: scope.clone(),
                };
                if claims_ok && in_grant {
                    granted.push(g);
                } else {
                    denied.push(g);
                }
            }
        }

        PolicyDecision { granted, denied }
    }
}

#[cfg(test)]
mod tests {
    use super::super::permission_ticket::PermissionRequest;
    use super::*;
    use chrono::Utc;

    fn ticket(perms: &[(&str, &[&str])]) -> PermissionTicket {
        PermissionTicket {
            ticket: "t".into(),
            permissions: perms
                .iter()
                .map(|(r, ss)| PermissionRequest {
                    resource_id: (*r).into(),
                    resource_scopes: ss.iter().map(|s| (*s).into()).collect(),
                })
                .collect(),
            resource_owner: "alice".into(),
            issued_at: Utc::now(),
            expires_at: Utc::now(),
            redeemed: false,
        }
    }

    fn policy_grant(
        rs: &str,
        subject: &str,
        scopes: &[&str],
        req_claims: &[(&str, &str)],
    ) -> ResourcePolicy {
        let mut sg = HashMap::new();
        sg.insert(subject.into(), scopes.iter().map(|s| (*s).into()).collect());
        let mut rc = HashMap::new();
        for (k, v) in req_claims {
            rc.insert((*k).into(), (*v).into());
        }
        ResourcePolicy {
            resource_id: rs.into(),
            subject_grants: sg,
            required_claims: rc,
        }
    }

    #[test]
    fn unknown_resource_denies_all() {
        let engine = PolicyEngine::new();
        let dec = engine.evaluate(
            &ticket(&[("rs1", &["view"])]),
            "bob",
            &PushedClaims::empty(),
        );
        assert!(dec.granted.is_empty());
        assert_eq!(dec.denied.len(), 1);
    }

    #[test]
    fn matching_grant_succeeds() {
        let engine = PolicyEngine::new();
        engine.upsert(policy_grant("rs1", "bob", &["view"], &[]));
        let dec = engine.evaluate(
            &ticket(&[("rs1", &["view"])]),
            "bob",
            &PushedClaims::empty(),
        );
        assert!(dec.is_fully_granted());
    }

    #[test]
    fn missing_scope_denied() {
        let engine = PolicyEngine::new();
        engine.upsert(policy_grant("rs1", "bob", &["view"], &[]));
        let dec = engine.evaluate(
            &ticket(&[("rs1", &["edit"])]),
            "bob",
            &PushedClaims::empty(),
        );
        assert!(!dec.is_fully_granted());
        assert_eq!(dec.denied.len(), 1);
    }

    #[test]
    fn different_subject_denied() {
        let engine = PolicyEngine::new();
        engine.upsert(policy_grant("rs1", "bob", &["view"], &[]));
        let dec = engine.evaluate(
            &ticket(&[("rs1", &["view"])]),
            "eve",
            &PushedClaims::empty(),
        );
        assert!(dec.granted.is_empty());
    }

    #[test]
    fn claim_gate_satisfied() {
        let engine = PolicyEngine::new();
        engine.upsert(policy_grant("rs1", "bob", &["view"], &[("dept", "eng")]));
        let pushed = PushedClaims::from_pairs(&[("dept", "eng")]);
        let dec = engine.evaluate(&ticket(&[("rs1", &["view"])]), "bob", &pushed);
        assert!(dec.is_fully_granted());
    }

    #[test]
    fn claim_gate_missing_denies() {
        let engine = PolicyEngine::new();
        engine.upsert(policy_grant("rs1", "bob", &["view"], &[("dept", "eng")]));
        let dec = engine.evaluate(
            &ticket(&[("rs1", &["view"])]),
            "bob",
            &PushedClaims::empty(),
        );
        assert!(!dec.is_fully_granted());
    }

    #[test]
    fn claim_gate_wrong_value_denies() {
        let engine = PolicyEngine::new();
        engine.upsert(policy_grant("rs1", "bob", &["view"], &[("dept", "eng")]));
        let pushed = PushedClaims::from_pairs(&[("dept", "marketing")]);
        let dec = engine.evaluate(&ticket(&[("rs1", &["view"])]), "bob", &pushed);
        assert!(!dec.is_fully_granted());
    }

    #[test]
    fn partial_decision_returns_both() {
        let engine = PolicyEngine::new();
        engine.upsert(policy_grant("rs1", "bob", &["view"], &[]));
        let dec = engine.evaluate(
            &ticket(&[("rs1", &["view", "edit"])]),
            "bob",
            &PushedClaims::empty(),
        );
        assert_eq!(dec.granted.len(), 1);
        assert_eq!(dec.denied.len(), 1);
        assert!(!dec.is_fully_granted());
    }
}
