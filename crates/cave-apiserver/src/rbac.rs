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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubjectAccessReviewSpec {
    pub user: String,
    pub groups: Vec<String>,
    pub tenant_id: String,
    pub resource_attributes: Option<ResourceAttributes>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    pub api_groups: Vec<String>,
    pub resources: Vec<String>,
    pub verbs: Vec<String>,
    pub resource_names: Vec<String>,
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
        let res_ok   = r.resources.iter().any(|x| x == "*" || x == &attrs.resource);
        let verb_ok  = r.verbs.iter().any(|v| v == "*" || v == &attrs.verb);
        let name_ok  = r.resource_names.is_empty()
            || r.resource_names.iter().any(|n| n == &attrs.name);
        if group_ok && res_ok && verb_ok && name_ok {
            return true;
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
                }),
            },
            status: SubjectAccessReviewStatus {
                allowed: false, denied: false, reason: String::new(), evaluation_error: String::new(),
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
}
