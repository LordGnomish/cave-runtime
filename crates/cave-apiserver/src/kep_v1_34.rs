// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! KEPs landing in Kubernetes 1.31–1.34 — bundled module so we don't
//! sprout a top-level mod for each individual KEP.
//!
//! - KEP-4008 (CRD validation ratcheting, GA in 1.30): on update, errors
//!   that pre-existed in the *unchanged* fields are tolerated.
//! - KEP-1287 (in-place pod resize, beta in 1.33, GA target 1.34):
//!   PodResizePolicy + container resource update without restart.
//! - KEP-3331 (structured authn config, GA 1.30): JWT issuer + claim
//!   mappings + claim validation.
//! - KEP-3962 (mutating admission policy, alpha 1.32): CEL-based mutation
//!   without webhooks. Type surface only here; CEL evaluator is M1.
//! - KEP-5073 (declarative validation, alpha 1.31): types only.
//!
//! ## Tenant invariant
//!
//! Each subfeature carries its own; tests guard at the boundary that
//! makes sense for the feature.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

// ─────────────────────────────────────────────────────────────────────────────
// KEP-4008 — Ratcheting validation. Compute the JSON-pointer paths that
// changed between two versions; let validators decide whether a violation
// is "ratcheted in" (already present in the unchanged part) or "newly
// introduced" (pre-existed in changed part is fine; new in unchanged is
// fine too — only NEW failures in CHANGED paths block).
// ─────────────────────────────────────────────────────────────────────────────

/// Compute the set of JSON-pointer paths that differ between `old` and
/// `new`. Paths point to the *deepest* divergence. Order is undefined.
pub fn changed_paths(old: &serde_json::Value, new: &serde_json::Value) -> HashSet<String> {
    let mut out = HashSet::new();
    diff_walk(old, new, "", &mut out);
    out
}

fn diff_walk(
    o: &serde_json::Value,
    n: &serde_json::Value,
    prefix: &str,
    out: &mut HashSet<String>,
) {
    match (o, n) {
        (serde_json::Value::Object(ao), serde_json::Value::Object(an)) => {
            let keys: HashSet<&String> = ao.keys().chain(an.keys()).collect();
            for k in keys {
                let path = format!("{prefix}/{}", escape_json_pointer(k));
                match (ao.get(k), an.get(k)) {
                    (Some(a), Some(b)) => diff_walk(a, b, &path, out),
                    _ => {
                        out.insert(path);
                    }
                }
            }
        }
        (serde_json::Value::Array(ao), serde_json::Value::Array(an)) => {
            let max = ao.len().max(an.len());
            for i in 0..max {
                let path = format!("{prefix}/{i}");
                match (ao.get(i), an.get(i)) {
                    (Some(a), Some(b)) => diff_walk(a, b, &path, out),
                    _ => {
                        out.insert(path);
                    }
                }
            }
        }
        (a, b) => {
            if a != b {
                out.insert(prefix.to_string());
            }
        }
    }
}

fn escape_json_pointer(seg: &str) -> String {
    seg.replace('~', "~0").replace('/', "~1")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationFailure {
    pub field_path: String,
    pub message: String,
}

/// Apply KEP-4008 ratcheting: a failure is *blocking* iff its `field_path`
/// is below at least one changed path. Failures rooted only in unchanged
/// subtrees are tolerated.
pub fn ratchet_failures(
    failures: Vec<ValidationFailure>,
    changed: &HashSet<String>,
) -> Vec<ValidationFailure> {
    failures
        .into_iter()
        .filter(|f| failure_in_changed_subtree(&f.field_path, changed))
        .collect()
}

fn failure_in_changed_subtree(field: &str, changed: &HashSet<String>) -> bool {
    changed.iter().any(|c| {
        field == c || field.starts_with(&format!("{c}/")) || c.starts_with(&format!("{field}/"))
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// KEP-1287 — In-place pod resize. The kubelet observes a CPU/memory bump
// in spec.containers[*].resources and applies it without restart, when the
// container's `resizePolicy` allows. Resize is gated on a `ResizeStatus`
// field on the Pod.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ResourceResizeRestartPolicy {
    NotRequired,
    RestartContainer,
}

impl Default for ResourceResizeRestartPolicy {
    fn default() -> Self {
        ResourceResizeRestartPolicy::NotRequired
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContainerResizePolicy {
    pub resource_name: String,
    pub restart_policy: ResourceResizeRestartPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum PodResizeStatus {
    /// Initial state.
    Empty,
    /// Resource update has been observed by kubelet.
    Proposed,
    /// Kubelet is actively reconciling the new resources.
    InProgress,
    /// Reconciliation complete and reflected in container.
    Deferred,
    /// Cannot satisfy on this node.
    Infeasible,
}

#[derive(Debug, Clone, Default)]
pub struct ContainerResources {
    pub requests: HashMap<String, i64>,
    pub limits: HashMap<String, i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResizeDecision {
    /// Container can be resized in place; restart not required.
    InPlaceNoRestart,
    /// Resize requires restart; mark Pod for container restart.
    RestartRequired { containers: Vec<String> },
    /// No diff — no-op.
    NoChange,
    /// Resize cannot be satisfied (over node allocatable).
    Infeasible(String),
}

pub fn evaluate_resize(
    container: &str,
    old_res: &ContainerResources,
    new_res: &ContainerResources,
    policies: &[ContainerResizePolicy],
    node_allocatable: &HashMap<String, i64>,
) -> ResizeDecision {
    let mut changed_resources: Vec<String> = vec![];
    for k in old_res.requests.keys().chain(new_res.requests.keys()) {
        if old_res.requests.get(k) != new_res.requests.get(k) {
            changed_resources.push(k.clone());
        }
    }
    for k in old_res.limits.keys().chain(new_res.limits.keys()) {
        if old_res.limits.get(k) != new_res.limits.get(k) {
            changed_resources.push(k.clone());
        }
    }
    changed_resources.sort();
    changed_resources.dedup();
    if changed_resources.is_empty() {
        return ResizeDecision::NoChange;
    }
    // Feasibility: each new request must be <= allocatable.
    for (k, v) in &new_res.requests {
        if let Some(alloc) = node_allocatable.get(k) {
            if v > alloc {
                return ResizeDecision::Infeasible(format!(
                    "requested {k} {v} exceeds node allocatable {alloc}"
                ));
            }
        }
    }
    let mut needs_restart = false;
    for r in &changed_resources {
        let policy = policies.iter().find(|p| p.resource_name == *r);
        let restart = policy.map(|p| &p.restart_policy)
            == Some(&ResourceResizeRestartPolicy::RestartContainer);
        if restart {
            needs_restart = true;
            break;
        }
    }
    if needs_restart {
        ResizeDecision::RestartRequired {
            containers: vec![container.into()],
        }
    } else {
        ResizeDecision::InPlaceNoRestart
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// KEP-3331 — AuthenticationConfiguration. Models the JWT path: issuer URL,
// claim mappings (username/groups/uid), claim validation rules.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Issuer {
    pub url: String,
    /// Allowed audiences (`aud` claim). At least one must match.
    pub audiences: Vec<String>,
    pub certificate_authority: Vec<u8>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClaimOrExpression {
    /// Direct claim name (e.g. "sub", "preferred_username").
    pub claim: String,
    /// CEL expression — when set, overrides `claim`. Stub here.
    #[serde(default)]
    pub expression: String,
    pub prefix: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClaimMappings {
    pub username: ClaimOrExpression,
    pub groups: ClaimOrExpression,
    pub uid: ClaimOrExpression,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClaimValidationRule {
    /// Direct claim equality check.
    pub claim: Option<String>,
    pub required_value: Option<String>,
    /// CEL expression — opaque here.
    pub expression: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JWTAuthenticator {
    pub issuer: Issuer,
    pub claim_mappings: ClaimMappings,
    pub claim_validation_rules: Vec<ClaimValidationRule>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuthenticationConfiguration {
    pub jwt: Vec<JWTAuthenticator>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuthnConfigError {
    #[error("at least one JWT authenticator is required")]
    NoJWT,
    #[error("issuer.url must be https://")]
    IssuerNotHttps,
    #[error("at least one audience must be configured")]
    NoAudiences,
    #[error("username claim or expression is required")]
    UsernameMissing,
    #[error("issuer URL `{0}` is duplicated")]
    DuplicateIssuer(String),
}

pub fn validate_authn_config(c: &AuthenticationConfiguration) -> Result<(), AuthnConfigError> {
    if c.jwt.is_empty() {
        return Err(AuthnConfigError::NoJWT);
    }
    let mut seen: HashSet<&String> = HashSet::new();
    for jwt in &c.jwt {
        if !jwt.issuer.url.starts_with("https://") {
            return Err(AuthnConfigError::IssuerNotHttps);
        }
        if jwt.issuer.audiences.is_empty() {
            return Err(AuthnConfigError::NoAudiences);
        }
        if jwt.claim_mappings.username.claim.is_empty()
            && jwt.claim_mappings.username.expression.is_empty()
        {
            return Err(AuthnConfigError::UsernameMissing);
        }
        if !seen.insert(&jwt.issuer.url) {
            return Err(AuthnConfigError::DuplicateIssuer(jwt.issuer.url.clone()));
        }
    }
    Ok(())
}

/// Apply claim mappings to extracted JWT claims. Returns the (username,
/// groups, uid) tuple. Missing groups / uid yields empty values.
pub fn apply_claim_mappings(
    mappings: &ClaimMappings,
    claims: &serde_json::Value,
) -> (String, Vec<String>, String) {
    let username = read_claim(&mappings.username, claims).unwrap_or_default();
    let groups = match claims.get(&mappings.groups.claim) {
        Some(serde_json::Value::Array(a)) => a
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .map(|g| match &mappings.groups.prefix {
                Some(p) => format!("{p}{g}"),
                None => g,
            })
            .collect(),
        _ => vec![],
    };
    let uid = read_claim(&mappings.uid, claims).unwrap_or_default();
    (username, groups, uid)
}

fn read_claim(m: &ClaimOrExpression, claims: &serde_json::Value) -> Option<String> {
    if !m.expression.is_empty() {
        // CEL expression evaluation is M1 territory; stub returns None so
        // tests can use `claim` directly.
        return None;
    }
    let raw = claims.get(&m.claim)?.as_str()?;
    let prefixed = match &m.prefix {
        Some(p) => format!("{p}{raw}"),
        None => raw.to_string(),
    };
    Some(prefixed)
}

#[cfg(test)]
mod tests;
