// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! AuthorizationPolicy evaluator.
//!
//! Mirrors `pilot/pkg/security/authz/builder/builder.go` plus the runtime
//! enforcement that runs in ztunnel (L4 source/identity) and in the waypoint
//! (L7 method/path/JWT). One evaluator handles both surfaces.
//!
//! Policy semantics (matches Istio v1.29.2):
//!
//! * `Action::Deny` rules are evaluated first — any match → DENY.
//! * `Action::Allow` rules are evaluated next — any match → ALLOW.
//! * If at least one ALLOW rule exists and none matched → DENY (deny-by-default
//!   only when an ALLOW policy is in scope).
//! * If only DENY rules exist (no ALLOW), the default is ALLOW.

use crate::ambient::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Decision {
    Allow,
    Deny,
}

/// Match clauses on the request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct From {
    /// SPIFFE principal — exact match, e.g. `spiffe://cluster.local/ns/acme/sa/web`.
    pub principal: Option<String>,
    /// Source namespace match.
    pub namespace: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToL7 {
    pub method: Option<String>,
    pub path_exact: Option<String>,
    pub path_prefix: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhenJwt {
    /// JWT claim key (after issuer/audience verification by `cave-auth`).
    pub claim: String,
    /// Required value for the claim.
    pub equals: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub from: From,
    pub to: ToL7,
    pub when: Option<WhenJwt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizationPolicy {
    pub name: String,
    pub namespace: String,
    pub tenant: TenantId,
    pub action: Action,
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthRequest {
    pub source_principal: String,
    pub source_namespace: String,
    pub method: String,
    pub path: String,
    /// JWT claims as already-validated (k, v) pairs from cave-auth.
    pub jwt_claims: Vec<(String, String)>,
}

impl AuthRequest {
    pub fn jwt_claim(&self, key: &str) -> Option<&str> {
        self.jwt_claims.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

impl Rule {
    pub fn matches(&self, req: &AuthRequest) -> bool {
        if let Some(p) = &self.from.principal {
            if p != &req.source_principal {
                return false;
            }
        }
        if let Some(n) = &self.from.namespace {
            if n != &req.source_namespace {
                return false;
            }
        }
        if let Some(m) = &self.to.method {
            if !req.method.eq_ignore_ascii_case(m) {
                return false;
            }
        }
        if let Some(p) = &self.to.path_exact {
            if p != &req.path {
                return false;
            }
        }
        if let Some(p) = &self.to.path_prefix {
            if !req.path.starts_with(p) {
                return false;
            }
        }
        if let Some(w) = &self.when {
            match req.jwt_claim(&w.claim) {
                Some(v) if v == w.equals => {}
                _ => return false,
            }
        }
        true
    }
}

/// Evaluate an entire policy set against a request. Mirrors
/// `authzPolicies.GetAuthorizationPolicies` + the Envoy RBAC evaluation order
/// described in upstream's `security/proto/authorization/v1beta1/policy.proto`.
pub fn evaluate(policies: &[AuthorizationPolicy], tenant: &TenantId, req: &AuthRequest) -> Decision {
    // Tenant-scope: only consider policies owned by `tenant`.
    let scoped: Vec<&AuthorizationPolicy> = policies.iter().filter(|p| &p.tenant == tenant).collect();

    // Pass 1 — DENYs.
    for p in &scoped {
        if p.action == Action::Deny && p.rules.iter().any(|r| r.matches(req)) {
            return Decision::Deny;
        }
    }
    // Pass 2 — ALLOWs.
    let mut had_allow_policy = false;
    for p in &scoped {
        if p.action == Action::Allow {
            had_allow_policy = true;
            if p.rules.iter().any(|r| r.matches(req)) {
                return Decision::Allow;
            }
        }
    }

    if had_allow_policy {
        Decision::Deny
    } else {
        Decision::Allow
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::istio(
    "pilot/pkg/security/authz/builder/builder.go",
    "Builder.Build",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ambient_test_ctx;

    fn req() -> AuthRequest {
        AuthRequest {
            source_principal: "spiffe://cluster.local/ns/acme/sa/web".into(),
            source_namespace: "acme".into(),
            method: "GET".into(),
            path: "/api/users".into(),
            jwt_claims: vec![("group".into(), "viewers".into())],
        }
    }

    fn policy(name: &str, action: Action, rules: Vec<Rule>) -> AuthorizationPolicy {
        AuthorizationPolicy {
            name: name.into(),
            namespace: "acme".into(),
            tenant: TenantId::new("acme").expect("test fixture"),
            action,
            rules,
        }
    }

    #[test]
    fn allow_rule_matching_principal_grants_access() {
        let (_cite, tenant) = ambient_test_ctx!(
            "pilot/pkg/security/authz/builder/builder.go",
            "buildAllowPolicy",
            "acme"
        );
        let p = policy(
            "allow-web",
            Action::Allow,
            vec![Rule {
                from: From { principal: Some("spiffe://cluster.local/ns/acme/sa/web".into()), ..Default::default() },
                ..Default::default()
            }],
        );
        assert_eq!(evaluate(&[p], &tenant, &req()), Decision::Allow);
    }

    #[test]
    fn deny_takes_precedence_over_allow() {
        let (_cite, tenant) = ambient_test_ctx!(
            "pilot/pkg/security/authz/builder/builder.go",
            "buildDenyPolicy",
            "acme"
        );
        let allow = policy(
            "allow-all",
            Action::Allow,
            vec![Rule::default()],
        );
        let deny = policy(
            "deny-writes",
            Action::Deny,
            vec![Rule {
                to: ToL7 { method: Some("POST".into()), ..Default::default() },
                ..Default::default()
            }],
        );
        let mut r = req();
        r.method = "POST".into();
        assert_eq!(evaluate(&[allow, deny], &tenant, &r), Decision::Deny);
    }

    #[test]
    fn http_method_match_is_case_insensitive() {
        let (_cite, tenant) = ambient_test_ctx!(
            "pilot/pkg/security/authz/matcher/header.go",
            "MethodMatcher",
            "acme"
        );
        let p = policy(
            "allow-get",
            Action::Allow,
            vec![Rule { to: ToL7 { method: Some("get".into()), ..Default::default() }, ..Default::default() }],
        );
        assert_eq!(evaluate(&[p], &tenant, &req()), Decision::Allow);
    }

    #[test]
    fn path_prefix_must_match_exactly_at_start() {
        let (_cite, tenant) = ambient_test_ctx!(
            "pilot/pkg/security/authz/matcher/header.go",
            "PathMatcher",
            "acme"
        );
        let p = policy(
            "allow-api",
            Action::Allow,
            vec![Rule { to: ToL7 { path_prefix: Some("/api".into()), ..Default::default() }, ..Default::default() }],
        );
        assert_eq!(evaluate(&[p.clone()], &tenant, &req()), Decision::Allow);
        let mut r = req();
        r.path = "/healthz".into();
        assert_eq!(evaluate(&[p], &tenant, &r), Decision::Deny);
    }

    #[test]
    fn jwt_claim_must_equal_required_value() {
        let (_cite, tenant) = ambient_test_ctx!(
            "pilot/pkg/security/authz/matcher/jwt.go",
            "JwtMatcher",
            "acme"
        );
        let p = policy(
            "allow-admins",
            Action::Allow,
            vec![Rule {
                when: Some(WhenJwt { claim: "group".into(), equals: "admins".into() }),
                ..Default::default()
            }],
        );
        // viewers != admins
        assert_eq!(evaluate(&[p.clone()], &tenant, &req()), Decision::Deny);
        let mut r = req();
        r.jwt_claims = vec![("group".into(), "admins".into())];
        assert_eq!(evaluate(&[p], &tenant, &r), Decision::Allow);
    }

    #[test]
    fn allow_in_scope_with_no_matching_rule_denies_by_default() {
        let (_cite, tenant) = ambient_test_ctx!(
            "pilot/pkg/security/authz/builder/builder.go",
            "Build",
            "acme"
        );
        // ALLOW policy exists but matches nothing → DENY.
        let p = policy(
            "allow-empty",
            Action::Allow,
            vec![Rule {
                to: ToL7 { method: Some("DELETE".into()), ..Default::default() },
                ..Default::default()
            }],
        );
        assert_eq!(evaluate(&[p], &tenant, &req()), Decision::Deny);
    }

    #[test]
    fn no_allow_policy_in_scope_defaults_to_allow() {
        let (_cite, tenant) = ambient_test_ctx!(
            "pilot/pkg/security/authz/builder/builder.go",
            "Build",
            "acme"
        );
        // Only DENYs and none match → ALLOW.
        let p = policy(
            "deny-bots",
            Action::Deny,
            vec![Rule {
                from: From { namespace: Some("bots".into()), ..Default::default() },
                ..Default::default()
            }],
        );
        assert_eq!(evaluate(&[p], &tenant, &req()), Decision::Allow);
    }

    #[test]
    fn cross_tenant_policy_is_ignored_by_evaluator() {
        let (_cite, owner) = ambient_test_ctx!(
            "pilot/pkg/security/authz/builder/builder.go",
            "tenantScope",
            "acme"
        );
        let mut p = policy(
            "allow-all",
            Action::Allow,
            vec![Rule::default()],
        );
        // Reassign ownership to a different tenant.
        p.tenant = TenantId::new("evil").expect("test fixture");
        // The acme tenant sees an empty policy set → default ALLOW.
        assert_eq!(evaluate(&[p], &owner, &req()), Decision::Allow);
    }
}
