// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Authentication & Authorization review APIs:
//!
//!   * `authentication.k8s.io/v1.TokenReview`
//!   * `authorization.k8s.io/v1.SubjectAccessReview`
//!   * `authorization.k8s.io/v1.SelfSubjectAccessReview`
//!   * `authorization.k8s.io/v1.SelfSubjectRulesReview`
//!   * `authorization.k8s.io/v1.LocalSubjectAccessReview`
//!
//! Upstream sources (kubernetes/kubernetes v1.31):
//!   * `staging/src/k8s.io/api/authentication/v1/types.go`
//!   * `staging/src/k8s.io/api/authorization/v1/types.go`
//!   * `staging/src/k8s.io/apiserver/pkg/authentication/authenticator/`
//!   * `staging/src/k8s.io/apiserver/pkg/authorization/authorizer/`
//!
//! Tenant invariant: a TokenReview/SAR carrying `tenant_id=T` MUST never
//! authenticate or authorize as another tenant. The store layer enforces
//! this via tenant-scoped lookups; the review APIs themselves carry the
//! tenant on the `status.user.extra` field for audit clarity.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};

// ─────────────────────────────────────────────────────────────────────────────
// UserInfo — `authentication/v1/types.go::UserInfo`.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserInfo {
    pub username: String,
    pub uid: String,
    pub groups: Vec<String>,
    pub extra: BTreeMap<String, Vec<String>>,
}

impl UserInfo {
    pub fn with_tenant(mut self, tenant: &str) -> Self {
        self.extra
            .entry("cave.runtime/tenant-id".into())
            .or_default()
            .push(tenant.into());
        self
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TokenReview
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenReviewSpec {
    pub token: String,
    /// Aud claims this server expects; empty == "any".
    #[serde(default)]
    pub audiences: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenReviewStatus {
    pub authenticated: bool,
    pub user: UserInfo,
    pub audiences: Vec<String>,
    pub error: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenReview {
    #[serde(default)]
    pub api_version: String,
    #[serde(default)]
    pub kind: String,
    pub spec: TokenReviewSpec,
    #[serde(default)]
    pub status: TokenReviewStatus,
}

pub trait TokenAuthenticator: Send + Sync {
    /// Returns (UserInfo, audiences) when the token is valid; error otherwise.
    fn authenticate(
        &self,
        token: &str,
        expected_audiences: &[String],
        tenant_hint: &str,
    ) -> Result<(UserInfo, Vec<String>), String>;
}

/// In-memory token registry — keyed by (tenant, token).
#[derive(Default)]
pub struct StaticTokenAuthenticator {
    inner: RwLock<HashMap<(String, String), (UserInfo, Vec<String>)>>,
}

impl StaticTokenAuthenticator {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn register(&self, tenant: &str, token: &str, user: UserInfo, audiences: Vec<String>) {
        self.inner
            .write()
            .unwrap()
            .insert((tenant.into(), token.into()), (user, audiences));
    }
}

impl TokenAuthenticator for StaticTokenAuthenticator {
    fn authenticate(
        &self,
        token: &str,
        expected_audiences: &[String],
        tenant: &str,
    ) -> Result<(UserInfo, Vec<String>), String> {
        let g = self.inner.read().unwrap();
        let (user, auds) = g
            .get(&(tenant.into(), token.into()))
            .ok_or_else(|| "invalid token".to_string())?;
        // If audiences requested, at least one must overlap.
        if !expected_audiences.is_empty() && !expected_audiences.iter().any(|a| auds.contains(a)) {
            return Err(format!(
                "none of expected audiences {expected_audiences:?} present in token"
            ));
        }
        Ok((user.clone().with_tenant(tenant), auds.clone()))
    }
}

pub fn run_token_review(
    auth: &dyn TokenAuthenticator,
    tenant: &str,
    review: &TokenReview,
) -> TokenReview {
    let mut out = review.clone();
    match auth.authenticate(&review.spec.token, &review.spec.audiences, tenant) {
        Ok((user, auds)) => {
            out.status = TokenReviewStatus {
                authenticated: true,
                user,
                audiences: auds,
                error: String::new(),
            };
        }
        Err(e) => {
            out.status = TokenReviewStatus {
                authenticated: false,
                user: UserInfo::default().with_tenant(tenant),
                audiences: vec![],
                error: e,
            };
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// SubjectAccessReview / SelfSubjectAccessReview / LocalSubjectAccessReview
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceAttributes {
    pub namespace: String,
    pub verb: String,
    pub group: String,
    pub version: String,
    pub resource: String,
    pub subresource: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct NonResourceAttributes {
    pub path: String,
    pub verb: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubjectAccessReviewSpec {
    pub resource_attributes: Option<ResourceAttributes>,
    pub non_resource_attributes: Option<NonResourceAttributes>,
    pub user: String,
    pub groups: Vec<String>,
    pub uid: String,
    pub extra: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubjectAccessReviewStatus {
    pub allowed: bool,
    pub denied: bool,
    pub reason: String,
    pub evaluation_error: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubjectAccessReview {
    #[serde(default)]
    pub api_version: String,
    #[serde(default)]
    pub kind: String,
    pub spec: SubjectAccessReviewSpec,
    #[serde(default)]
    pub status: SubjectAccessReviewStatus,
}

pub trait Authorizer: Send + Sync {
    /// Decide allow/deny/no-opinion + reason. Mirrors upstream
    /// `authorizer.Decision`.
    fn authorize(&self, tenant: &str, spec: &SubjectAccessReviewSpec) -> AuthzDecision;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthzDecision {
    Allow { reason: String },
    Deny { reason: String },
    NoOpinion,
}

/// In-memory authorizer: `(tenant, user, verb, resource)` → decision.
#[derive(Default)]
pub struct StaticAuthorizer {
    inner: RwLock<HashMap<(String, String, String, String), AuthzDecision>>,
}

impl StaticAuthorizer {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn allow(&self, tenant: &str, user: &str, verb: &str, resource: &str, reason: &str) {
        self.inner.write().unwrap().insert(
            (tenant.into(), user.into(), verb.into(), resource.into()),
            AuthzDecision::Allow {
                reason: reason.into(),
            },
        );
    }
    pub fn deny(&self, tenant: &str, user: &str, verb: &str, resource: &str, reason: &str) {
        self.inner.write().unwrap().insert(
            (tenant.into(), user.into(), verb.into(), resource.into()),
            AuthzDecision::Deny {
                reason: reason.into(),
            },
        );
    }
}

impl Authorizer for StaticAuthorizer {
    fn authorize(&self, tenant: &str, spec: &SubjectAccessReviewSpec) -> AuthzDecision {
        let r = match &spec.resource_attributes {
            Some(ra) => ra,
            None => return AuthzDecision::NoOpinion,
        };
        let g = self.inner.read().unwrap();
        g.get(&(
            tenant.into(),
            spec.user.clone(),
            r.verb.clone(),
            r.resource.clone(),
        ))
        .cloned()
        .unwrap_or(AuthzDecision::NoOpinion)
    }
}

pub fn run_subject_access_review(
    authz: &dyn Authorizer,
    tenant: &str,
    review: &SubjectAccessReview,
) -> SubjectAccessReview {
    let mut out = review.clone();
    match authz.authorize(tenant, &review.spec) {
        AuthzDecision::Allow { reason } => {
            out.status = SubjectAccessReviewStatus {
                allowed: true,
                denied: false,
                reason,
                evaluation_error: String::new(),
            };
        }
        AuthzDecision::Deny { reason } => {
            out.status = SubjectAccessReviewStatus {
                allowed: false,
                denied: true,
                reason,
                evaluation_error: String::new(),
            };
        }
        AuthzDecision::NoOpinion => {
            out.status = SubjectAccessReviewStatus {
                allowed: false,
                denied: false,
                reason: "no opinion".into(),
                evaluation_error: String::new(),
            };
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// SelfSubjectAccessReview = SAR with the request user filled in by the
// apiserver from the auth context.
// ─────────────────────────────────────────────────────────────────────────────

pub fn build_self_review_spec(
    user: &UserInfo,
    attrs: ResourceAttributes,
) -> SubjectAccessReviewSpec {
    SubjectAccessReviewSpec {
        resource_attributes: Some(attrs),
        non_resource_attributes: None,
        user: user.username.clone(),
        groups: user.groups.clone(),
        uid: user.uid.clone(),
        extra: user.extra.clone(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SelfSubjectRulesReview — list rules applicable to the self-user in a
// namespace. We model the registry side; ruleset enumeration is upstream's
// `rbac/ruleresolver.go::RulesFor`.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceRule {
    pub verbs: Vec<String>,
    pub api_groups: Vec<String>,
    pub resources: Vec<String>,
    pub resource_names: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NonResourceRule {
    pub verbs: Vec<String>,
    pub non_resource_urls: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SelfSubjectRulesReviewSpec {
    pub namespace: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SelfSubjectRulesReviewStatus {
    pub resource_rules: Vec<ResourceRule>,
    pub non_resource_rules: Vec<NonResourceRule>,
    pub incomplete: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SelfSubjectRulesReview {
    #[serde(default)]
    pub api_version: String,
    #[serde(default)]
    pub kind: String,
    pub spec: SelfSubjectRulesReviewSpec,
    #[serde(default)]
    pub status: SelfSubjectRulesReviewStatus,
}

pub trait RulesResolver: Send + Sync {
    fn rules_for(
        &self,
        tenant: &str,
        user: &UserInfo,
        namespace: &str,
    ) -> (Vec<ResourceRule>, Vec<NonResourceRule>);
}

#[derive(Default)]
pub struct StaticRules {
    pub by_user:
        RwLock<HashMap<(String, String, String), (Vec<ResourceRule>, Vec<NonResourceRule>)>>,
}

impl StaticRules {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set(
        &self,
        tenant: &str,
        user: &str,
        namespace: &str,
        r: Vec<ResourceRule>,
        nr: Vec<NonResourceRule>,
    ) {
        self.by_user
            .write()
            .unwrap()
            .insert((tenant.into(), user.into(), namespace.into()), (r, nr));
    }
}

impl RulesResolver for StaticRules {
    fn rules_for(
        &self,
        tenant: &str,
        user: &UserInfo,
        namespace: &str,
    ) -> (Vec<ResourceRule>, Vec<NonResourceRule>) {
        let g = self.by_user.read().unwrap();
        g.get(&(tenant.into(), user.username.clone(), namespace.into()))
            .cloned()
            .unwrap_or_default()
    }
}

pub fn run_self_subject_rules_review(
    resolver: &dyn RulesResolver,
    tenant: &str,
    user: &UserInfo,
    review: &SelfSubjectRulesReview,
) -> SelfSubjectRulesReview {
    let (r, nr) = resolver.rules_for(tenant, user, &review.spec.namespace);
    let mut out = review.clone();
    out.status = SelfSubjectRulesReviewStatus {
        resource_rules: r,
        non_resource_rules: nr,
        incomplete: false,
    };
    out
}

#[cfg(test)]
mod tests;

#[allow(dead_code)]
fn unused_arc() -> Arc<()> {
    Arc::new(())
}
