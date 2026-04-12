//! JWT validation (RequestAuthentication) and authorization policy engine
//! (AuthorizationPolicy).
//!
//! AuthorizationPolicy logic follows Istio semantics:
//!   1. If any DENY policy matches → DENY.
//!   2. If no ALLOW policy exists for the workload → ALLOW (default-allow mesh).
//!   3. If at least one ALLOW policy exists, the request must match one → ALLOW.
//!   4. Otherwise → DENY.

use crate::{
    error::{MeshError, MeshResult},
    models::{
        AuthorizationPolicy, AuthzAction, AuthzRule, Condition, Operation, RequestAuthentication,
        RequestContext, Source,
    },
};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};
use tracing::debug;

// ─────────────────────────────────────────────────────────────
// AuthEngine
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AuthEngine {
    request_auth: Arc<RwLock<HashMap<String, RequestAuthentication>>>,
    authz_policies: Arc<RwLock<HashMap<String, AuthorizationPolicy>>>,
    /// Shared HMAC secret for HS256 JWT validation (test / dev only).
    /// Production deployments inject RSA public keys per issuer.
    pub hmac_secret: Arc<RwLock<String>>,
}

impl Default for AuthEngine {
    fn default() -> Self {
        Self::new("cave-mesh-dev-secret")
    }
}

impl AuthEngine {
    pub fn new(hmac_secret: impl Into<String>) -> Self {
        Self {
            request_auth: Arc::new(RwLock::new(HashMap::new())),
            authz_policies: Arc::new(RwLock::new(HashMap::new())),
            hmac_secret: Arc::new(RwLock::new(hmac_secret.into())),
        }
    }

    // ─── RequestAuthentication CRUD ──────────────────────────

    pub fn upsert_request_auth(&self, policy: RequestAuthentication) {
        let key = format!("{}/{}", policy.namespace, policy.name);
        self.request_auth.write().unwrap().insert(key, policy);
    }

    pub fn remove_request_auth(&self, namespace: &str, name: &str) {
        let key = format!("{namespace}/{name}");
        self.request_auth.write().unwrap().remove(&key);
    }

    pub fn list_request_auth(&self) -> Vec<RequestAuthentication> {
        self.request_auth.read().unwrap().values().cloned().collect()
    }

    // ─── AuthorizationPolicy CRUD ────────────────────────────

    pub fn upsert_authz_policy(&self, policy: AuthorizationPolicy) {
        let key = format!("{}/{}", policy.namespace, policy.name);
        self.authz_policies.write().unwrap().insert(key, policy);
    }

    pub fn remove_authz_policy(&self, namespace: &str, name: &str) {
        let key = format!("{namespace}/{name}");
        self.authz_policies.write().unwrap().remove(&key);
    }

    pub fn list_authz_policies(&self) -> Vec<AuthorizationPolicy> {
        self.authz_policies
            .read()
            .unwrap()
            .values()
            .cloned()
            .collect()
    }

    // ─── JWT Validation ──────────────────────────────────────

    /// Validate a JWT token using the configured HMAC secret.
    /// Returns the decoded claims on success.
    pub fn validate_jwt(&self, token: &str) -> MeshResult<HashMap<String, Value>> {
        let secret = self.hmac_secret.read().unwrap();
        let key = DecodingKey::from_secret(secret.as_bytes());
        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_exp = true;
        validation.set_required_spec_claims(&["sub"]);

        let data = decode::<HashMap<String, Value>>(token, &key, &validation)
            .map_err(|e| MeshError::Jwt(e.to_string()))?;
        Ok(data.claims)
    }

    /// Validate JWT and attach claims to the request context.
    pub fn authenticate_request(
        &self,
        namespace: &str,
        workload_labels: &HashMap<String, String>,
        token: Option<&str>,
        ctx: &mut RequestContext,
    ) -> MeshResult<()> {
        // Find applicable RequestAuthentication policies
        let policies: Vec<RequestAuthentication> = {
            let map = self.request_auth.read().unwrap();
            map.values()
                .filter(|p| {
                    p.namespace == namespace
                        && p.selector
                            .as_ref()
                            .map(|sel| {
                                sel.iter().all(|(k, v)| {
                                    workload_labels.get(k).map(|vv| vv == v).unwrap_or(false)
                                })
                            })
                            .unwrap_or(true)
                })
                .cloned()
                .collect()
        };

        if policies.is_empty() {
            // No JWT policy → pass-through
            return Ok(());
        }

        // If a token was provided, validate it
        if let Some(raw_token) = token {
            match self.validate_jwt(raw_token) {
                Ok(claims) => {
                    ctx.jwt_claims = Some(claims);
                    return Ok(());
                }
                Err(e) => return Err(e),
            }
        }

        // Token required but absent
        Err(MeshError::Jwt("JWT required but not provided".to_string()))
    }

    // ─── Authorization ───────────────────────────────────────

    /// Check whether the request is authorized to reach the workload.
    pub fn check_authz(
        &self,
        namespace: &str,
        workload_labels: &HashMap<String, String>,
        ctx: &RequestContext,
    ) -> MeshResult<()> {
        let policies: Vec<AuthorizationPolicy> = {
            let map = self.authz_policies.read().unwrap();
            map.values()
                .filter(|p| {
                    p.namespace == namespace
                        && p.selector
                            .as_ref()
                            .map(|sel| {
                                sel.iter().all(|(k, v)| {
                                    workload_labels.get(k).map(|vv| vv == v).unwrap_or(false)
                                })
                            })
                            .unwrap_or(true)
                })
                .cloned()
                .collect()
        };

        debug!(
            namespace = %namespace,
            path = %ctx.path,
            method = %ctx.method,
            policies = %policies.len(),
            "AuthzPolicy check"
        );

        // ── Step 1: check DENY policies ──────────────────────
        for policy in policies.iter().filter(|p| p.action == AuthzAction::Deny) {
            if policy.rules.is_empty() || self.rules_match(&policy.rules, ctx) {
                return Err(MeshError::AuthzDenied(format!(
                    "DENY policy '{}' matched",
                    policy.name
                )));
            }
        }

        // ── Step 2: collect ALLOW policies ───────────────────
        let allow_policies: Vec<&AuthorizationPolicy> = policies
            .iter()
            .filter(|p| p.action == AuthzAction::Allow)
            .collect();

        if allow_policies.is_empty() {
            // No ALLOW policies → default-allow
            return Ok(());
        }

        // ── Step 3: at least one ALLOW rule must match ───────
        for policy in &allow_policies {
            if policy.rules.is_empty() || self.rules_match(&policy.rules, ctx) {
                return Ok(());
            }
        }

        Err(MeshError::AuthzDenied(
            "No ALLOW policy matched".to_string(),
        ))
    }

    // ─── Internal match helpers ──────────────────────────────

    /// Returns true if ANY rule in the slice matches the request (OR semantics).
    fn rules_match(&self, rules: &[AuthzRule], ctx: &RequestContext) -> bool {
        rules.iter().any(|rule| self.single_rule_matches(rule, ctx))
    }

    fn single_rule_matches(&self, rule: &AuthzRule, ctx: &RequestContext) -> bool {
        // FROM: all specified sources must match (AND)
        if !rule.from.is_empty()
            && !rule
                .from
                .iter()
                .all(|src| self.source_matches(src, ctx))
        {
            return false;
        }
        // TO: all specified operations must match (AND)
        if !rule.to.is_empty()
            && !rule.to.iter().all(|op| self.op_matches(op, ctx))
        {
            return false;
        }
        // WHEN: all conditions must match (AND)
        if !rule.when.is_empty()
            && !rule.when.iter().all(|cond| self.cond_matches(cond, ctx))
        {
            return false;
        }
        true
    }

    fn source_matches(&self, src: &Source, ctx: &RequestContext) -> bool {
        // Positive matches
        if !src.principals.is_empty() {
            let principal = ctx.source_principal.as_deref().unwrap_or("");
            if !src.principals.iter().any(|p| glob_match(p, principal)) {
                return false;
            }
        }
        if !src.namespaces.is_empty() {
            let ns = ctx.source_namespace.as_deref().unwrap_or("");
            if !src.namespaces.iter().any(|n| n == ns || n == "*") {
                return false;
            }
        }
        if !src.ip_blocks.is_empty() {
            let ip = ctx.source_ip.as_deref().unwrap_or("");
            if !src.ip_blocks.iter().any(|b| ip_in_block(ip, b)) {
                return false;
            }
        }
        // Negative matches
        if !src.not_principals.is_empty() {
            let principal = ctx.source_principal.as_deref().unwrap_or("");
            if src.not_principals.iter().any(|p| glob_match(p, principal)) {
                return false;
            }
        }
        if !src.not_namespaces.is_empty() {
            let ns = ctx.source_namespace.as_deref().unwrap_or("");
            if src.not_namespaces.iter().any(|n| n == ns || n == "*") {
                return false;
            }
        }
        true
    }

    fn op_matches(&self, op: &Operation, ctx: &RequestContext) -> bool {
        if !op.methods.is_empty()
            && !op.methods.iter().any(|m| m == &ctx.method || m == "*")
        {
            return false;
        }
        if !op.paths.is_empty()
            && !op.paths.iter().any(|p| glob_match(p, &ctx.path))
        {
            return false;
        }
        if !op.hosts.is_empty()
            && !op.hosts.iter().any(|h| h == &ctx.host || h == "*")
        {
            return false;
        }
        if !op.ports.is_empty() {
            let port_str = ctx.port.map(|p| p.to_string()).unwrap_or_default();
            if !op.ports.iter().any(|p| p == &port_str || p == "*") {
                return false;
            }
        }
        // Negative
        if !op.not_methods.is_empty()
            && op.not_methods.iter().any(|m| m == &ctx.method || m == "*")
        {
            return false;
        }
        if !op.not_paths.is_empty()
            && op.not_paths.iter().any(|p| glob_match(p, &ctx.path))
        {
            return false;
        }
        true
    }

    fn cond_matches(&self, cond: &Condition, ctx: &RequestContext) -> bool {
        let value = self.resolve_condition_value(&cond.key, ctx);
        let val_str = value.as_deref().unwrap_or("");

        // Positive: value must be in cond.values (OR)
        if !cond.values.is_empty() && !cond.values.iter().any(|v| v == val_str || v == "*") {
            return false;
        }
        // Negative: value must NOT be in cond.not_values
        if !cond.not_values.is_empty()
            && cond.not_values.iter().any(|v| v == val_str || v == "*")
        {
            return false;
        }
        true
    }

    /// Resolve a condition key to a value from the request context.
    fn resolve_condition_value(&self, key: &str, ctx: &RequestContext) -> Option<String> {
        match key {
            "source.ip" => ctx.source_ip.clone(),
            "source.namespace" => ctx.source_namespace.clone(),
            "source.principal" => ctx.source_principal.clone(),
            "request.method" => Some(ctx.method.clone()),
            "request.path" => Some(ctx.path.clone()),
            _ if key.starts_with("request.auth.claims[") => {
                // e.g. "request.auth.claims[group]"
                let claim_name = key
                    .trim_start_matches("request.auth.claims[")
                    .trim_end_matches(']');
                ctx.jwt_claims
                    .as_ref()
                    .and_then(|c| c.get(claim_name))
                    .and_then(|v| match v {
                        Value::String(s) => Some(s.clone()),
                        other => Some(other.to_string()),
                    })
            }
            _ => None,
        }
    }
}

// ─────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────

/// Simple glob matching: `*` matches anything, `?` matches one char.
fn glob_match(pattern: &str, value: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if pattern == value {
        return true;
    }
    // Wildcard prefix/suffix: "cluster.local/*/sa/*"
    if pattern.contains('*') {
        let parts: Vec<&str> = pattern.split('*').collect();
        let mut pos = 0usize;
        for (i, part) in parts.iter().enumerate() {
            if part.is_empty() {
                continue;
            }
            if i == 0 {
                if !value.starts_with(part) {
                    return false;
                }
                pos = part.len();
            } else if i == parts.len() - 1 {
                if !value[pos..].ends_with(part) {
                    return false;
                }
            } else if let Some(idx) = value[pos..].find(part) {
                pos += idx + part.len();
            } else {
                return false;
            }
        }
        return true;
    }
    false
}

/// Minimal IP block check — handles exact match and /prefix notation.
fn ip_in_block(ip: &str, block: &str) -> bool {
    if block == "*" || block == ip {
        return true;
    }
    // Very basic CIDR: if block has no /, treat as prefix string match
    if let Some(prefix) = block.strip_suffix("/32") {
        return ip == prefix;
    }
    ip.starts_with(block.trim_end_matches('/'))
}
