// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! JWT validation (RequestAuthentication) and authorization policy engine
//! (AuthorizationPolicy).
//!
//! Istio semantics:
//!   1. DENY policies evaluated first — any match → DENY.
//!   2. No ALLOW policies for the workload → default-allow.
//!   3. At least one ALLOW policy exists → request must match one.
//!   4. CUSTOM action → delegated to external provider (recorded but not blocked here).
//!   5. AUDIT action → always allowed, decision logged.

use crate::{
    error::{MeshError, MeshResult},
    models::{
        AuthorizationPolicy, AuthzAction, AuthzRule, Condition, Operation, RequestAuthentication,
        RequestContext, Source,
    },
};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};
use tracing::{debug, info};

// ─────────────────────────────────────────────────────────────
// AuthEngine
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AuthEngine {
    request_auth: Arc<RwLock<HashMap<String, RequestAuthentication>>>,
    authz_policies: Arc<RwLock<HashMap<String, AuthorizationPolicy>>>,
    /// HMAC secret for HS256 JWT validation (dev/test; prod uses per-issuer RSA keys).
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
        self.request_auth
            .read()
            .unwrap()
            .values()
            .cloned()
            .collect()
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

    /// Validate a JWT using the configured HMAC secret (HS256).
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
            return Ok(());
        }

        if let Some(raw_token) = token {
            let claims = self.validate_jwt(raw_token)?;
            // Populate request_principal from sub claim
            if let Some(Value::String(sub)) = claims.get("sub") {
                ctx.request_principal = Some(sub.clone());
            }
            ctx.jwt_claims = Some(claims);
            return Ok(());
        }

        // Token required but absent
        Err(MeshError::Jwt("JWT required but not provided".to_string()))
    }

    // ─── Authorization ───────────────────────────────────────

    /// Check authorization against all applicable policies.
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
            policies = policies.len(),
            "AuthzPolicy check"
        );

        // ── 1. DENY ──────────────────────────────────────────
        for policy in policies.iter().filter(|p| p.action == AuthzAction::Deny) {
            if policy.rules.is_empty() || self.rules_match(&policy.rules, ctx) {
                return Err(MeshError::AuthzDenied(format!(
                    "DENY policy '{}' matched",
                    policy.name
                )));
            }
        }

        // ── 2. AUDIT (allow, log) ────────────────────────────
        for policy in policies.iter().filter(|p| p.action == AuthzAction::Audit) {
            if policy.rules.is_empty() || self.rules_match(&policy.rules, ctx) {
                info!(policy = %policy.name, path = %ctx.path, "AUDIT policy matched");
                // Audit does not block
            }
        }

        // ── 3. CUSTOM (delegate to external authz) ───────────
        for policy in policies.iter().filter(|p| p.action == AuthzAction::Custom) {
            if policy.rules.is_empty() || self.rules_match(&policy.rules, ctx) {
                // In a real implementation this would call the external provider.
                // Here we allow but record the delegation.
                if let Some(provider) = &policy.provider {
                    debug!(provider = %provider.name, "CUSTOM authz delegation (pass-through in control plane)");
                }
            }
        }

        // ── 4. ALLOW ─────────────────────────────────────────
        let allow_policies: Vec<&AuthorizationPolicy> = policies
            .iter()
            .filter(|p| p.action == AuthzAction::Allow)
            .collect();

        if allow_policies.is_empty() {
            return Ok(()); // default-allow
        }

        for policy in &allow_policies {
            if policy.rules.is_empty() || self.rules_match(&policy.rules, ctx) {
                return Ok(());
            }
        }

        Err(MeshError::AuthzDenied(
            "No ALLOW policy matched".to_string(),
        ))
    }

    // ─── Internal helpers ────────────────────────────────────

    fn rules_match(&self, rules: &[AuthzRule], ctx: &RequestContext) -> bool {
        rules.iter().any(|rule| self.single_rule_matches(rule, ctx))
    }

    fn single_rule_matches(&self, rule: &AuthzRule, ctx: &RequestContext) -> bool {
        if !rule.from.is_empty() && !rule.from.iter().all(|src| self.source_matches(src, ctx)) {
            return false;
        }
        if !rule.to.is_empty() && !rule.to.iter().all(|op| self.op_matches(op, ctx)) {
            return false;
        }
        if !rule.when.is_empty() && !rule.when.iter().all(|cond| self.cond_matches(cond, ctx)) {
            return false;
        }
        true
    }

    fn source_matches(&self, src: &Source, ctx: &RequestContext) -> bool {
        if !src.principals.is_empty() {
            let principal = ctx.source_principal.as_deref().unwrap_or("");
            if !src.principals.iter().any(|p| glob_match(p, principal)) {
                return false;
            }
        }
        if !src.request_principals.is_empty() {
            let rp = ctx.request_principal.as_deref().unwrap_or("");
            if !src.request_principals.iter().any(|p| glob_match(p, rp)) {
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
        if !src.remote_ip_blocks.is_empty() {
            let ip = ctx.remote_ip.as_deref().unwrap_or("");
            if !src.remote_ip_blocks.iter().any(|b| ip_in_block(ip, b)) {
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
        if !src.not_request_principals.is_empty() {
            let rp = ctx.request_principal.as_deref().unwrap_or("");
            if src.not_request_principals.iter().any(|p| glob_match(p, rp)) {
                return false;
            }
        }
        if !src.not_namespaces.is_empty() {
            let ns = ctx.source_namespace.as_deref().unwrap_or("");
            if src.not_namespaces.iter().any(|n| n == ns || n == "*") {
                return false;
            }
        }
        if !src.not_ip_blocks.is_empty() {
            let ip = ctx.source_ip.as_deref().unwrap_or("");
            if src.not_ip_blocks.iter().any(|b| ip_in_block(ip, b)) {
                return false;
            }
        }
        true
    }

    fn op_matches(&self, op: &Operation, ctx: &RequestContext) -> bool {
        if !op.methods.is_empty() && !op.methods.iter().any(|m| m == &ctx.method || m == "*") {
            return false;
        }
        if !op.paths.is_empty() && !op.paths.iter().any(|p| glob_match(p, &ctx.path)) {
            return false;
        }
        if !op.hosts.is_empty() && !op.hosts.iter().any(|h| h == &ctx.host || h == "*") {
            return false;
        }
        if !op.ports.is_empty() {
            let port_str = ctx.port.map(|p| p.to_string()).unwrap_or_default();
            if !op.ports.iter().any(|p| p == &port_str || p == "*") {
                return false;
            }
        }
        if !op.not_methods.is_empty() && op.not_methods.iter().any(|m| m == &ctx.method || m == "*")
        {
            return false;
        }
        if !op.not_paths.is_empty() && op.not_paths.iter().any(|p| glob_match(p, &ctx.path)) {
            return false;
        }
        if !op.not_hosts.is_empty() && op.not_hosts.iter().any(|h| h == &ctx.host || h == "*") {
            return false;
        }
        if !op.not_ports.is_empty() {
            let port_str = ctx.port.map(|p| p.to_string()).unwrap_or_default();
            if op.not_ports.iter().any(|p| p == &port_str || p == "*") {
                return false;
            }
        }
        true
    }

    fn cond_matches(&self, cond: &Condition, ctx: &RequestContext) -> bool {
        let value = self.resolve_condition_value(&cond.key, ctx);
        let val_str = value.as_deref().unwrap_or("");

        if !cond.values.is_empty() && !cond.values.iter().any(|v| v == val_str || v == "*") {
            return false;
        }
        if !cond.not_values.is_empty() && cond.not_values.iter().any(|v| v == val_str || v == "*") {
            return false;
        }
        true
    }

    fn resolve_condition_value(&self, key: &str, ctx: &RequestContext) -> Option<String> {
        match key {
            "source.ip" => ctx.source_ip.clone(),
            "source.namespace" => ctx.source_namespace.clone(),
            "source.principal" => ctx.source_principal.clone(),
            "request.auth.principal" => ctx.request_principal.clone(),
            "request.method" => Some(ctx.method.clone()),
            "request.path" => Some(ctx.path.clone()),
            _ if key.starts_with("request.auth.claims[") => {
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

fn glob_match(pattern: &str, value: &str) -> bool {
    if pattern == "*" || pattern == value {
        return true;
    }
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
                return value[pos..].ends_with(part);
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

fn ip_in_block(ip: &str, block: &str) -> bool {
    if block == "*" || block == ip {
        return true;
    }
    if let Some(prefix) = block.strip_suffix("/32") {
        return ip == prefix;
    }
    ip.starts_with(block.trim_end_matches('/'))
}
