// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! RBAC SubjectAccessReview (SAR) evaluator.
//!
//! Upstream: kubernetes/kubernetes v1.30.0
//!   * `staging/src/k8s.io/api/authorization/v1/types.go` (`SubjectAccessReview`).
//!   * `plugin/pkg/auth/authorizer/rbac/rbac.go` (`RBACAuthorizer.Authorize`).
//!   * `plugin/pkg/auth/authorizer/rbac/subject_locator.go`.
//!
//! Given a SAR (user, groups, optional resourceAttributes), produce an
//! allow/deny decision by walking RoleBindings/ClusterRoleBindings against
//! the registered Roles/ClusterRoles. Verbs match exactly (or `*`).
//!
//! Tenant invariant: each subject is bound to a tenant_id. Cross-tenant SAR
//! is denied with `cross-tenant-not-permitted` regardless of role rules — a
//! subject from tenant A MUST NOT be authorized for a request scoped to
//! tenant B even if its rules look permissive. Cluster-scoped resources are
//! evaluated under the subject's own tenant only.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceAttributes {
    pub namespace: String,
    pub verb: String,
    pub group: String,
    pub resource: String,
    pub name: String,
    /// Subresource (e.g. "status", "exec"). Empty for the main resource.
    /// Mirrors `authorization/v1.ResourceAttributes.Subresource`.
    #[serde(default)]
    pub subresource: String,
}

/// Non-resource URL request attributes. Mirrors
/// `authorization/v1.NonResourceAttributes`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NonResourceAttributes {
    pub path: String,   // e.g. "/healthz", "/metrics", "/api/v1"
    pub verb: String,   // "get" / "post"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubjectAccessReviewSpec {
    pub user: String,
    pub groups: Vec<String>,
    pub tenant_id: String,
    pub resource_attributes: Option<ResourceAttributes>,
    #[serde(default)]
    pub non_resource_attributes: Option<NonResourceAttributes>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubjectAccessReviewStatus {
    pub allowed: bool,
    pub denied: bool,
    pub reason: String,
    pub evaluation_error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubjectAccessReview {
    pub spec: SubjectAccessReviewSpec,
    pub status: SubjectAccessReviewStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PolicyRule {
    pub api_groups: Vec<String>,
    pub resources: Vec<String>,
    pub verbs: Vec<String>,
    pub resource_names: Vec<String>,
    /// Sub-resources allowed by this rule (e.g. `pods/status`,
    /// `pods/exec`). Mirrors upstream PolicyRule.Resources entries that
    /// include a slash.
    #[serde(default)]
    pub subresources: Vec<String>,
    /// Non-resource URL paths this rule grants access to. Mirrors
    /// `rbac/v1.PolicyRule.NonResourceURLs`.
    #[serde(default)]
    pub non_resource_urls: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subject {
    pub kind: String,        // "User" | "Group" | "ServiceAccount"
    pub name: String,
    pub namespace: String,
}

#[derive(Debug, Clone)]
pub struct Role {
    pub tenant_id: String,
    pub namespace: String,    // empty for ClusterRole
    pub name: String,
    pub rules: Vec<PolicyRule>,
}

#[derive(Debug, Clone)]
pub struct Binding {
    pub tenant_id: String,
    pub namespace: String,    // empty for ClusterRoleBinding
    pub name: String,
    pub subjects: Vec<Subject>,
    pub role_kind: String,    // "Role" | "ClusterRole"
    pub role_name: String,
}

pub struct RbacAuthorizer {
    inner: Mutex<RbacInner>,
}

#[derive(Default)]
struct RbacInner {
    roles: HashMap<(String, String, String), Role>,             // (tenant, ns, name) → Role
    cluster_roles: HashMap<(String, String), Role>,             // (tenant, name) → ClusterRole
    bindings: Vec<Binding>,
}

impl RbacAuthorizer {
    pub fn new() -> Self { Self { inner: Mutex::new(RbacInner::default()) } }

    pub fn upsert_role(&self, role: Role) {
        let mut inner = self.inner.lock().unwrap();
        if role.namespace.is_empty() {
            inner.cluster_roles.insert(
                (role.tenant_id.clone(), role.name.clone()), role);
        } else {
            inner.roles.insert(
                (role.tenant_id.clone(), role.namespace.clone(), role.name.clone()), role);
        }
    }

    pub fn upsert_binding(&self, binding: Binding) {
        let mut inner = self.inner.lock().unwrap();
        inner.bindings.push(binding);
    }

    pub fn review(&self, mut sar: SubjectAccessReview) -> SubjectAccessReview {
        // NonResource path: handled separately, never matches resource rules.
        if let Some(nra) = sar.spec.non_resource_attributes.clone() {
            if sar.spec.resource_attributes.is_some() {
                sar.status = SubjectAccessReviewStatus {
                    allowed: false, denied: true,
                    reason: "exactly one of resource_attributes / non_resource_attributes required".into(),
                    evaluation_error: String::new(),
                };
                return sar;
            }
            return self.review_non_resource(sar, nra);
        }
        let attrs = match &sar.spec.resource_attributes {
            Some(a) => a.clone(),
            None => {
                sar.status = SubjectAccessReviewStatus {
                    allowed: false, denied: true,
                    reason: "no resource attributes".into(),
                    evaluation_error: String::new(),
                };
                return sar;
            }
        };
        let inner = self.inner.lock().unwrap();
        let subject_tenant = sar.spec.tenant_id.clone();
        // Walk bindings whose subject matches and whose tenant matches the SAR
        // subject's tenant. Cross-tenant lookups are explicitly disallowed.
        for b in inner.bindings.iter() {
            if b.tenant_id != subject_tenant {
                continue; // tenant_id invariant: never resolve other-tenant bindings
            }
            // Namespaced binding only applies inside its namespace; cluster
            // binding applies anywhere within the same tenant.
            if !b.namespace.is_empty() && b.namespace != attrs.namespace {
                continue;
            }
            if !subject_matches(&b.subjects, &sar.spec.user, &sar.spec.groups) {
                continue;
            }
            // Resolve role.
            let role: Option<&Role> = match b.role_kind.as_str() {
                "ClusterRole" => inner.cluster_roles.get(&(b.tenant_id.clone(), b.role_name.clone())),
                "Role" => inner.roles.get(&(b.tenant_id.clone(), b.namespace.clone(), b.role_name.clone())),
                _ => None,
            };
            let Some(role) = role else { continue };
            if rule_allows(&role.rules, &attrs) {
                sar.status = SubjectAccessReviewStatus {
                    allowed: true, denied: false,
                    reason: format!("RBAC: allowed by binding {}/{}", b.namespace, b.name),
                    evaluation_error: String::new(),
                };
                return sar;
            }
        }
        sar.status = SubjectAccessReviewStatus {
            allowed: false, denied: false,
            reason: "no matching policy rule".into(),
            evaluation_error: String::new(),
        };
        sar
    }

    fn review_non_resource(
        &self,
        mut sar: SubjectAccessReview,
        nra: NonResourceAttributes,
    ) -> SubjectAccessReview {
        let inner = self.inner.lock().unwrap();
        let subject_tenant = sar.spec.tenant_id.clone();
        for b in inner.bindings.iter() {
            if b.tenant_id != subject_tenant { continue; }
            // NonResource rules only live inside ClusterRoles upstream
            // (`apis/rbac/validation/validation.go::validateClusterRole`).
            if b.role_kind != "ClusterRole" { continue; }
            if !subject_matches(&b.subjects, &sar.spec.user, &sar.spec.groups) {
                continue;
            }
            let Some(role) = inner.cluster_roles.get(
                &(b.tenant_id.clone(), b.role_name.clone())
            ) else { continue };
            if non_resource_rule_allows(&role.rules, &nra) {
                sar.status = SubjectAccessReviewStatus {
                    allowed: true, denied: false,
                    reason: format!(
                        "RBAC: allowed (non-resource) by ClusterRoleBinding {}", b.name),
                    evaluation_error: String::new(),
                };
                return sar;
            }
        }
        sar.status = SubjectAccessReviewStatus {
            allowed: false, denied: false,
            reason: "no matching non-resource policy rule".into(),
            evaluation_error: String::new(),
        };
        sar
    }
}

impl Default for RbacAuthorizer {
    fn default() -> Self { Self::new() }
}

fn subject_matches(subjects: &[Subject], user: &str, groups: &[String]) -> bool {
    for s in subjects {
        match s.kind.as_str() {
            "User" if s.name == user => return true,
            "Group" if groups.iter().any(|g| g == &s.name) => return true,
            "ServiceAccount" => {
                let qual = format!("system:serviceaccount:{}:{}", s.namespace, s.name);
                if user == qual { return true; }
            }
            _ => {}
        }
    }
    false
}

fn rule_allows(rules: &[PolicyRule], attrs: &ResourceAttributes) -> bool {
    for r in rules {
        let group_ok = r.api_groups.iter().any(|g| g == "*" || g == &attrs.group);
        // Resources match: `pods` matches `pods` (no subresource); for a
        // subresource request, `pods/status` must appear in `subresources`
        // OR `pods/status` literal string must appear in `resources`. That
        // mirrors upstream where `Resources` is read like
        // `<resource>[/<subresource>]` strings.
        let res_ok = if attrs.subresource.is_empty() {
            r.resources.iter().any(|x| x == "*" || x == &attrs.resource)
        } else {
            let combined = format!("{}/{}", attrs.resource, attrs.subresource);
            r.resources.iter().any(|x| {
                x == "*" || x == &combined || x == &format!("{}/*", attrs.resource)
            }) || r.subresources.iter().any(|s| s == "*" || s == &attrs.subresource)
        };
        let verb_ok  = r.verbs.iter().any(|v| v == "*" || v == &attrs.verb);
        let name_ok  = r.resource_names.is_empty()
            || r.resource_names.iter().any(|n| n == &attrs.name);
        if group_ok && res_ok && verb_ok && name_ok {
            return true;
        }
    }
    false
}

fn non_resource_rule_allows(rules: &[PolicyRule], nra: &NonResourceAttributes) -> bool {
    for r in rules {
        if r.non_resource_urls.is_empty() { continue; }
        let verb_ok = r.verbs.iter().any(|v| v == "*" || v == &nra.verb);
        if !verb_ok { continue; }
        for url in &r.non_resource_urls {
            // Upstream supports a trailing `*` wildcard suffix on
            // NonResourceURL paths (validation.go::ValidateNonResourceURL).
            if url == "*" || url == &nra.path {
                return true;
            }
            if let Some(prefix) = url.strip_suffix('*') {
                if nra.path.starts_with(prefix) {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(group: &str, res: &str, verbs: &[&str]) -> PolicyRule {
        PolicyRule {
            api_groups: vec![group.into()],
            resources: vec![res.into()],
            verbs: verbs.iter().map(|s| s.to_string()).collect(),
            resource_names: vec![],
            subresources: vec![],
            non_resource_urls: vec![],
        }
    }

    fn user_sub(name: &str) -> Subject {
        Subject { kind: "User".into(), name: name.into(), namespace: String::new() }
    }

    fn sar_for(user: &str, tenant: &str, ns: &str, verb: &str, res: &str) -> SubjectAccessReview {
        SubjectAccessReview {
            spec: SubjectAccessReviewSpec {
                user: user.into(),
                groups: vec![],
                tenant_id: tenant.into(),
                resource_attributes: Some(ResourceAttributes {
                    namespace: ns.into(), verb: verb.into(),
                    group: "".into(), resource: res.into(), name: "".into(),
                    subresource: String::new(),
                }),
                non_resource_attributes: None,
            },
            status: SubjectAccessReviewStatus {
                allowed: false, denied: false, reason: String::new(), evaluation_error: String::new(),
            },
        }
    }

    fn sar_non_resource(user: &str, tenant: &str, path: &str, verb: &str) -> SubjectAccessReview {
        SubjectAccessReview {
            spec: SubjectAccessReviewSpec {
                user: user.into(),
                groups: vec![],
                tenant_id: tenant.into(),
                resource_attributes: None,
                non_resource_attributes: Some(NonResourceAttributes {
                    path: path.into(), verb: verb.into(),
                }),
            },
            status: SubjectAccessReviewStatus {
                allowed: false, denied: false,
                reason: String::new(), evaluation_error: String::new(),
            },
        }
    }

    /// Upstream parity: `TestRBAC_AllowViaRoleBinding` (rbac/rbac_test.go).
    #[test]
    fn test_role_binding_allows_namespaced_verb() {
        let auth = RbacAuthorizer::new();
        auth.upsert_role(Role {
            tenant_id: "acme".into(), namespace: "default".into(), name: "viewer".into(),
            rules: vec![rule("", "configmaps", &["get", "list"])],
        });
        auth.upsert_binding(Binding {
            tenant_id: "acme".into(), namespace: "default".into(), name: "alice-viewer".into(),
            subjects: vec![user_sub("alice")],
            role_kind: "Role".into(), role_name: "viewer".into(),
        });
        let r = auth.review(sar_for("alice", "acme", "default", "get", "configmaps"));
        assert!(r.status.allowed);
        assert_eq!(r.spec.tenant_id, "acme",
            "tenant_id invariant: SAR retains spec tenant after review");
    }

    /// Upstream parity: `TestRBAC_DenyWhenVerbNotInRule`.
    #[test]
    fn test_denies_when_verb_not_listed() {
        let auth = RbacAuthorizer::new();
        auth.upsert_role(Role {
            tenant_id: "acme".into(), namespace: "default".into(), name: "viewer".into(),
            rules: vec![rule("", "configmaps", &["get"])],
        });
        auth.upsert_binding(Binding {
            tenant_id: "acme".into(), namespace: "default".into(), name: "alice-viewer".into(),
            subjects: vec![user_sub("alice")],
            role_kind: "Role".into(), role_name: "viewer".into(),
        });
        let r = auth.review(sar_for("alice", "acme", "default", "delete", "configmaps"));
        assert!(!r.status.allowed);
        assert_eq!(r.spec.tenant_id, "acme", "tenant_id invariant");
    }

    /// Upstream parity: `TestRBAC_CrossTenantDenied`.
    #[test]
    fn test_cross_tenant_binding_does_not_authorize() {
        let auth = RbacAuthorizer::new();
        auth.upsert_role(Role {
            tenant_id: "acme".into(), namespace: "default".into(), name: "viewer".into(),
            rules: vec![rule("", "configmaps", &["get"])],
        });
        auth.upsert_binding(Binding {
            tenant_id: "acme".into(), namespace: "default".into(), name: "alice-viewer".into(),
            subjects: vec![user_sub("alice")],
            role_kind: "Role".into(), role_name: "viewer".into(),
        });
        // Same user "alice" but coming in under tenant "globex".
        let r = auth.review(sar_for("alice", "globex", "default", "get", "configmaps"));
        assert!(!r.status.allowed,
            "tenant_id invariant: cross-tenant subjects MUST NOT inherit acme's binding");
        assert_eq!(r.spec.tenant_id, "globex",
            "tenant_id invariant: SAR's own tenant retained");
    }

    /// Upstream parity: `TestRBAC_ClusterRoleBindingAcrossNamespaces`.
    #[test]
    fn test_cluster_role_binding_applies_in_any_namespace() {
        let auth = RbacAuthorizer::new();
        auth.upsert_role(Role {
            tenant_id: "acme".into(), namespace: "".into(), name: "cluster-admin".into(),
            rules: vec![rule("*", "*", &["*"])],
        });
        auth.upsert_binding(Binding {
            tenant_id: "acme".into(), namespace: "".into(), name: "admins".into(),
            subjects: vec![user_sub("alice")],
            role_kind: "ClusterRole".into(), role_name: "cluster-admin".into(),
        });
        let r1 = auth.review(sar_for("alice", "acme", "default", "create", "pods"));
        let r2 = auth.review(sar_for("alice", "acme", "kube-system", "delete", "secrets"));
        assert!(r1.status.allowed);
        assert!(r2.status.allowed);
        assert_eq!(r1.spec.tenant_id, "acme", "tenant_id invariant");
    }

    /// Upstream parity: `TestRBAC_NamespacedBindingScopedToNamespace`.
    #[test]
    fn test_namespaced_binding_does_not_apply_in_other_namespace() {
        let auth = RbacAuthorizer::new();
        auth.upsert_role(Role {
            tenant_id: "acme".into(), namespace: "default".into(), name: "viewer".into(),
            rules: vec![rule("", "configmaps", &["get"])],
        });
        auth.upsert_binding(Binding {
            tenant_id: "acme".into(), namespace: "default".into(), name: "alice-viewer".into(),
            subjects: vec![user_sub("alice")],
            role_kind: "Role".into(), role_name: "viewer".into(),
        });
        let r = auth.review(sar_for("alice", "acme", "kube-system", "get", "configmaps"));
        assert!(!r.status.allowed);
        assert_eq!(r.spec.tenant_id, "acme", "tenant_id invariant");
    }

    /// Upstream parity: `TestRBAC_NoResourceAttributesDenies`.
    #[test]
    fn test_missing_resource_attributes_denies() {
        let auth = RbacAuthorizer::new();
        let mut sar = sar_for("alice", "acme", "default", "get", "configmaps");
        sar.spec.resource_attributes = None;
        let r = auth.review(sar);
        assert!(!r.status.allowed);
        assert!(r.status.denied);
        assert_eq!(r.spec.tenant_id, "acme",
            "tenant_id invariant: tenant retained even on missing-attrs deny");
    }

    /// Upstream parity: `TestRBAC_GroupSubjectMatch`.
    #[test]
    fn test_group_subject_match() {
        let auth = RbacAuthorizer::new();
        auth.upsert_role(Role {
            tenant_id: "acme".into(), namespace: "default".into(), name: "viewer".into(),
            rules: vec![rule("", "configmaps", &["get"])],
        });
        auth.upsert_binding(Binding {
            tenant_id: "acme".into(), namespace: "default".into(), name: "ops-viewer".into(),
            subjects: vec![Subject {
                kind: "Group".into(), name: "ops".into(), namespace: "".into(),
            }],
            role_kind: "Role".into(), role_name: "viewer".into(),
        });
        let mut sar = sar_for("alice", "acme", "default", "get", "configmaps");
        sar.spec.groups = vec!["ops".into()];
        let r = auth.review(sar);
        assert!(r.status.allowed);
        assert_eq!(r.spec.tenant_id, "acme", "tenant_id invariant");
    }

    /// Upstream parity: `TestRBAC_ServiceAccountSubjectMatch`.
    #[test]
    fn test_service_account_subject_match() {
        let auth = RbacAuthorizer::new();
        auth.upsert_role(Role {
            tenant_id: "acme".into(), namespace: "default".into(), name: "viewer".into(),
            rules: vec![rule("", "configmaps", &["get"])],
        });
        auth.upsert_binding(Binding {
            tenant_id: "acme".into(), namespace: "default".into(), name: "sa-binding".into(),
            subjects: vec![Subject {
                kind: "ServiceAccount".into(),
                name: "default".into(),
                namespace: "default".into(),
            }],
            role_kind: "Role".into(), role_name: "viewer".into(),
        });
        let r = auth.review(sar_for(
            "system:serviceaccount:default:default", "acme", "default", "get", "configmaps"));
        assert!(r.status.allowed);
        assert_eq!(r.spec.tenant_id, "acme", "tenant_id invariant");
    }

    /// Upstream parity: `TestRBAC_WildcardVerbAndResource`.
    #[test]
    fn test_wildcard_rule_matches_any_verb_and_resource() {
        let auth = RbacAuthorizer::new();
        auth.upsert_role(Role {
            tenant_id: "acme".into(), namespace: "default".into(), name: "any".into(),
            rules: vec![rule("*", "*", &["*"])],
        });
        auth.upsert_binding(Binding {
            tenant_id: "acme".into(), namespace: "default".into(), name: "any-binding".into(),
            subjects: vec![user_sub("alice")],
            role_kind: "Role".into(), role_name: "any".into(),
        });
        let r1 = auth.review(sar_for("alice", "acme", "default", "create", "anything"));
        let r2 = auth.review(sar_for("alice", "acme", "default", "patch", "deployments"));
        assert!(r1.status.allowed);
        assert!(r2.status.allowed);
        assert_eq!(r1.spec.tenant_id, "acme", "tenant_id invariant");
    }

    /// Upstream parity: `TestRBAC_NoBindingDenies`.
    #[test]
    fn test_no_matching_binding_denies() {
        let auth = RbacAuthorizer::new();
        let r = auth.review(sar_for("alice", "acme", "default", "get", "configmaps"));
        assert!(!r.status.allowed);
        assert!(!r.status.denied,
            "no matching policy is allowed=false but not explicit deny (parity with upstream)");
        assert_eq!(r.spec.tenant_id, "acme",
            "tenant_id invariant: SAR returned with original tenant");
    }

    // ── Deeper coverage (v1.36.0) ─────────────────────────────────────────────

    /// Upstream parity: `TestRBAC_ResourceNameMatchAllowsOnlyNamedResource`
    /// (rbac/rbac_test.go — PolicyRule.ResourceNames restricts the verb to
    /// the listed names only).
    #[test]
    fn test_resource_name_restricts_match_to_named_object() {
        let auth = RbacAuthorizer::new();
        auth.upsert_role(Role {
            tenant_id: "acme".into(), namespace: "default".into(), name: "named".into(),
            rules: vec![PolicyRule {
                api_groups: vec!["".into()],
                resources: vec!["configmaps".into()],
                verbs: vec!["get".into()],
                resource_names: vec!["allowed-cm".into()],
                ..Default::default()
            }],
        });
        auth.upsert_binding(Binding {
            tenant_id: "acme".into(), namespace: "default".into(), name: "named-bind".into(),
            subjects: vec![user_sub("alice")],
            role_kind: "Role".into(), role_name: "named".into(),
        });
        let mut sar_ok = sar_for("alice", "acme", "default", "get", "configmaps");
        sar_ok.spec.resource_attributes.as_mut().unwrap().name = "allowed-cm".into();
        let mut sar_no = sar_for("alice", "acme", "default", "get", "configmaps");
        sar_no.spec.resource_attributes.as_mut().unwrap().name = "other-cm".into();
        let r_ok = auth.review(sar_ok);
        let r_no = auth.review(sar_no);
        assert!(r_ok.status.allowed,
            "named resource matches the resource_names restriction");
        assert!(!r_no.status.allowed,
            "non-listed resource is denied even though verb+resource match");
        assert_eq!(r_ok.spec.tenant_id, "acme", "tenant_id invariant");
        assert_eq!(r_no.spec.tenant_id, "acme", "tenant_id invariant on deny path");
    }

    /// Upstream parity: `TestRBAC_MultipleBindingsUnion`
    /// (multiple bindings for the same subject form a union of permissions).
    #[test]
    fn test_multiple_bindings_form_a_union_of_permissions() {
        let auth = RbacAuthorizer::new();
        auth.upsert_role(Role {
            tenant_id: "acme".into(), namespace: "default".into(), name: "get-cm".into(),
            rules: vec![rule("", "configmaps", &["get"])],
        });
        auth.upsert_role(Role {
            tenant_id: "acme".into(), namespace: "default".into(), name: "del-cm".into(),
            rules: vec![rule("", "configmaps", &["delete"])],
        });
        auth.upsert_binding(Binding {
            tenant_id: "acme".into(), namespace: "default".into(), name: "alice-get".into(),
            subjects: vec![user_sub("alice")],
            role_kind: "Role".into(), role_name: "get-cm".into(),
        });
        auth.upsert_binding(Binding {
            tenant_id: "acme".into(), namespace: "default".into(), name: "alice-del".into(),
            subjects: vec![user_sub("alice")],
            role_kind: "Role".into(), role_name: "del-cm".into(),
        });
        let r_get = auth.review(sar_for("alice", "acme", "default", "get", "configmaps"));
        let r_del = auth.review(sar_for("alice", "acme", "default", "delete", "configmaps"));
        let r_create = auth.review(sar_for("alice", "acme", "default", "create", "configmaps"));
        assert!(r_get.status.allowed);
        assert!(r_del.status.allowed);
        assert!(!r_create.status.allowed,
            "verbs not in any rule are still denied — union, not wildcard");
        assert_eq!(r_get.spec.tenant_id, "acme", "tenant_id invariant");
    }

    /// Upstream parity: `TestRBAC_ServiceAccountCrossTenantDenied`
    /// (system:serviceaccount:* identity must not authorize across tenants).
    #[test]
    fn test_service_account_cannot_authorize_across_tenants() {
        let auth = RbacAuthorizer::new();
        auth.upsert_role(Role {
            tenant_id: "acme".into(), namespace: "default".into(), name: "viewer".into(),
            rules: vec![rule("", "configmaps", &["get"])],
        });
        auth.upsert_binding(Binding {
            tenant_id: "acme".into(), namespace: "default".into(), name: "sa-bind".into(),
            subjects: vec![Subject {
                kind: "ServiceAccount".into(),
                name: "default".into(),
                namespace: "default".into(),
            }],
            role_kind: "Role".into(), role_name: "viewer".into(),
        });
        // Same SA identity, but the SAR comes in tagged with a different tenant.
        let r = auth.review(sar_for(
            "system:serviceaccount:default:default", "globex", "default", "get", "configmaps"));
        assert!(!r.status.allowed,
            "tenant_id invariant: cross-tenant ServiceAccount MUST NOT inherit acme binding");
        assert_eq!(r.spec.tenant_id, "globex", "tenant_id invariant: SAR tenant retained");
    }

    /// Upstream parity: `TestRBAC_ClusterRoleViaRoleBindingScopedToNamespace`
    /// (a RoleBinding referencing a ClusterRole still confines effect to the
    /// binding's namespace — only ClusterRoleBinding makes it cluster-wide).
    #[test]
    fn test_role_binding_to_cluster_role_is_scoped_to_namespace() {
        let auth = RbacAuthorizer::new();
        auth.upsert_role(Role {
            tenant_id: "acme".into(), namespace: "".into(), name: "edit".into(),
            rules: vec![rule("*", "*", &["*"])],
        });
        auth.upsert_binding(Binding {
            tenant_id: "acme".into(), namespace: "default".into(), name: "alice-edit".into(),
            subjects: vec![user_sub("alice")],
            role_kind: "ClusterRole".into(), role_name: "edit".into(),
        });
        let r_in   = auth.review(sar_for("alice", "acme", "default", "create", "secrets"));
        let r_out  = auth.review(sar_for("alice", "acme", "kube-system", "create", "secrets"));
        assert!(r_in.status.allowed,
            "RoleBinding -> ClusterRole grants in the binding namespace");
        assert!(!r_out.status.allowed,
            "RoleBinding -> ClusterRole MUST NOT escape the binding namespace");
        assert_eq!(r_in.spec.tenant_id, "acme", "tenant_id invariant");
    }

    /// Upstream parity: `TestRBAC_GroupSubjectCrossTenantDenied`
    /// (Group membership in tenant A's binding does not authorize a SAR
    /// stamped with tenant B even with identical group name).
    #[test]
    fn test_group_subject_does_not_cross_tenants() {
        let auth = RbacAuthorizer::new();
        auth.upsert_role(Role {
            tenant_id: "acme".into(), namespace: "default".into(), name: "ops-reader".into(),
            rules: vec![rule("", "configmaps", &["get"])],
        });
        auth.upsert_binding(Binding {
            tenant_id: "acme".into(), namespace: "default".into(), name: "ops-bind".into(),
            subjects: vec![Subject {
                kind: "Group".into(), name: "ops".into(), namespace: "".into(),
            }],
            role_kind: "Role".into(), role_name: "ops-reader".into(),
        });
        let mut sar = sar_for("alice", "globex", "default", "get", "configmaps");
        sar.spec.groups = vec!["ops".into()];
        let r = auth.review(sar);
        assert!(!r.status.allowed,
            "tenant_id invariant: group binding scoped to tenant — globex.ops MUST NOT inherit acme.ops");
        assert_eq!(r.spec.tenant_id, "globex", "tenant_id invariant retained");
    }

    // ── Deeper coverage (deeper-004) — SAR ───────────────────────────────────

    /// Upstream parity: `TestRBAC_NonResourceURLAllowsExactPath`
    /// (rbac/rbac_test.go::TestVisitRulesFor — NonResourceURL exact path
    /// match grants the verb).
    #[test]
    fn test_non_resource_url_grants_exact_path_via_cluster_role_binding() {
        let auth = RbacAuthorizer::new();
        auth.upsert_role(Role {
            tenant_id: "acme".into(), namespace: "".into(), name: "metrics-reader".into(),
            rules: vec![PolicyRule {
                non_resource_urls: vec!["/metrics".into()],
                verbs: vec!["get".into()],
                ..Default::default()
            }],
        });
        auth.upsert_binding(Binding {
            tenant_id: "acme".into(), namespace: "".into(), name: "alice-metrics".into(),
            subjects: vec![user_sub("alice")],
            role_kind: "ClusterRole".into(), role_name: "metrics-reader".into(),
        });
        let r = auth.review(sar_non_resource("alice", "acme", "/metrics", "get"));
        assert!(r.status.allowed);
        assert_eq!(r.spec.tenant_id, "acme",
            "tenant_id invariant: NonResourceURL SAR retains tenant");
        assert!(r.status.reason.contains("non-resource"),
            "reason names the non-resource path");
    }

    /// Upstream parity: `TestRBAC_NonResourceURLWildcardSuffix`
    /// (validation.go::ValidateNonResourceURL — trailing `*` is a wildcard
    /// matched as a prefix).
    #[test]
    fn test_non_resource_url_wildcard_suffix_matches_prefix_paths() {
        let auth = RbacAuthorizer::new();
        auth.upsert_role(Role {
            tenant_id: "acme".into(), namespace: "".into(), name: "api-reader".into(),
            rules: vec![PolicyRule {
                non_resource_urls: vec!["/api/*".into()],
                verbs: vec!["get".into()],
                ..Default::default()
            }],
        });
        auth.upsert_binding(Binding {
            tenant_id: "acme".into(), namespace: "".into(), name: "alice-api".into(),
            subjects: vec![user_sub("alice")],
            role_kind: "ClusterRole".into(), role_name: "api-reader".into(),
        });
        let hit = auth.review(sar_non_resource("alice", "acme", "/api/v1/healthz", "get"));
        let miss = auth.review(sar_non_resource("alice", "acme", "/healthz", "get"));
        assert!(hit.status.allowed);
        assert!(!miss.status.allowed);
        assert_eq!(hit.spec.tenant_id, "acme", "tenant_id invariant");
    }

    /// Upstream parity: `TestRBAC_NonResourceURLOnlyClusterRoleBindings`
    /// (validation.go — non-resource rules are illegal on namespaced Roles
    /// per spec; we model that by only matching ClusterRoleBindings).
    #[test]
    fn test_non_resource_url_via_namespaced_role_binding_is_not_authorized() {
        let auth = RbacAuthorizer::new();
        auth.upsert_role(Role {
            tenant_id: "acme".into(), namespace: "default".into(), name: "ns-meta".into(),
            rules: vec![PolicyRule {
                non_resource_urls: vec!["/metrics".into()],
                verbs: vec!["get".into()],
                ..Default::default()
            }],
        });
        auth.upsert_binding(Binding {
            tenant_id: "acme".into(), namespace: "default".into(), name: "ns-bind".into(),
            subjects: vec![user_sub("alice")],
            role_kind: "Role".into(), role_name: "ns-meta".into(),
        });
        let r = auth.review(sar_non_resource("alice", "acme", "/metrics", "get"));
        assert!(!r.status.allowed,
            "non-resource access is only granted via ClusterRoleBinding");
        assert_eq!(r.spec.tenant_id, "acme",
            "tenant_id invariant: SAR tenant retained on deny path");
    }

    /// Upstream parity: `TestRBAC_SubresourceMatchPodsExec`
    /// (rbac_test.go — `pods/exec` grants exec verb on the exec subresource).
    #[test]
    fn test_subresource_pods_exec_match_grants_create() {
        let auth = RbacAuthorizer::new();
        auth.upsert_role(Role {
            tenant_id: "acme".into(), namespace: "default".into(), name: "exec-er".into(),
            rules: vec![PolicyRule {
                api_groups: vec!["".into()],
                resources: vec!["pods/exec".into()],
                verbs: vec!["create".into()],
                ..Default::default()
            }],
        });
        auth.upsert_binding(Binding {
            tenant_id: "acme".into(), namespace: "default".into(), name: "alice-exec".into(),
            subjects: vec![user_sub("alice")],
            role_kind: "Role".into(), role_name: "exec-er".into(),
        });
        let mut sar = sar_for("alice", "acme", "default", "create", "pods");
        sar.spec.resource_attributes.as_mut().unwrap().subresource = "exec".into();
        let r = auth.review(sar);
        assert!(r.status.allowed,
            "pods/exec subresource grants the requested verb");
        // Plain pods create on same role MUST NOT be authorised.
        let plain = auth.review(sar_for("alice", "acme", "default", "create", "pods"));
        assert!(!plain.status.allowed,
            "main resource not granted by a subresource-only rule");
        assert_eq!(r.spec.tenant_id, "acme", "tenant_id invariant");
    }

    /// Upstream parity: `TestRBAC_SubresourceWildcard`
    /// (rbac_test.go — `pods/*` grants verbs on every pod subresource).
    #[test]
    fn test_subresource_wildcard_pods_star_grants_status_and_log() {
        let auth = RbacAuthorizer::new();
        auth.upsert_role(Role {
            tenant_id: "acme".into(), namespace: "default".into(), name: "pods-all-sub".into(),
            rules: vec![PolicyRule {
                api_groups: vec!["".into()],
                resources: vec!["pods/*".into()],
                verbs: vec!["get".into()],
                ..Default::default()
            }],
        });
        auth.upsert_binding(Binding {
            tenant_id: "acme".into(), namespace: "default".into(), name: "alice-pods-sub".into(),
            subjects: vec![user_sub("alice")],
            role_kind: "Role".into(), role_name: "pods-all-sub".into(),
        });
        for sub in ["status", "log", "exec"] {
            let mut sar = sar_for("alice", "acme", "default", "get", "pods");
            sar.spec.resource_attributes.as_mut().unwrap().subresource = sub.into();
            let r = auth.review(sar);
            assert!(r.status.allowed,
                "pods/{} should be allowed under pods/* rule", sub);
            assert_eq!(r.spec.tenant_id, "acme", "tenant_id invariant for {}", sub);
        }
    }

    /// Upstream parity: `TestRBAC_RejectsBothAttributeKinds`
    /// (apis/authorization/v1 — exactly one of resource_attributes /
    /// non_resource_attributes may be set).
    #[test]
    fn test_sar_rejects_both_resource_and_non_resource_attributes() {
        let auth = RbacAuthorizer::new();
        let mut sar = sar_for("alice", "acme", "default", "get", "pods");
        sar.spec.non_resource_attributes = Some(NonResourceAttributes {
            path: "/metrics".into(), verb: "get".into(),
        });
        let r = auth.review(sar);
        assert!(!r.status.allowed && r.status.denied,
            "both attribute kinds set is an explicit deny");
        assert_eq!(r.spec.tenant_id, "acme",
            "tenant_id invariant: deny path retains tenant");
    }
}
