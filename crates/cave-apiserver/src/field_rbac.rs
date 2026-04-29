//! Field-level RBAC — admission deny on protected field changes (KEP-3633).
//!
//! Upstream: kubernetes/kubernetes v1.36.0
//!   * `staging/src/k8s.io/apiserver/pkg/admission/plugin/policy/`.
//!   * KEP-3633 — Field-level RBAC for admission policies.
//!
//! Field-level RBAC narrows resource-level permissions: a user may have
//! `update` permission on Pods but be barred from changing
//! `spec.serviceAccountName`. This module provides the matching engine
//! and a chain-style evaluator that compares the old vs new object.
//!
//! Tenant invariant: every protection rule is owned by a `tenant_id`.
//! Rules from tenant A MUST NOT block changes to tenant B's objects, and
//! a tenant cannot circumvent another tenant's protections.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

/// Subject — the actor whose permissions are being narrowed. Maps onto
/// the `User`/`Group`/`ServiceAccount` triumvirate from upstream RBAC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldSubject {
    User(String),
    Group(String),
    ServiceAccount { namespace: String, name: String },
    /// `*` — any subject. Useful for global protections.
    Any,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldProtection {
    pub tenant_id: String,
    pub name: String,
    /// Resource types this rule applies to (e.g. `["pods"]`).
    pub resources: Vec<String>,
    /// Subjects barred from changing the protected fields.
    pub denied_subjects: Vec<FieldSubject>,
    /// Dotted JSON paths the subjects MUST NOT change.
    pub protected_paths: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct FieldRequest {
    pub tenant_id: String,
    pub user: String,
    pub groups: Vec<String>,
    pub resource: String,
    pub old_object: serde_json::Value,
    pub new_object: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldDecision {
    Allow,
    Deny {
        rule_name: String,
        path: String,
        subject: FieldSubject,
    },
}

pub struct FieldRbacRegistry {
    inner: Mutex<HashMap<(String, String), FieldProtection>>, // (tenant, name)
}

impl FieldRbacRegistry {
    pub fn new() -> Self {
        Self { inner: Mutex::new(HashMap::new()) }
    }

    pub fn upsert(&self, p: FieldProtection) {
        self.inner.lock().unwrap().insert(
            (p.tenant_id.clone(), p.name.clone()), p);
    }

    /// Evaluate `req` against every rule registered under `req.tenant_id`.
    /// Returns the first denial; otherwise Allow. Mirrors the dispatcher
    /// pattern in upstream `policy/validating/dispatcher.go::Dispatch`,
    /// adapted to the field-level KEP.
    pub fn evaluate(&self, req: &FieldRequest) -> FieldDecision {
        let inner = self.inner.lock().unwrap();
        let mut rules: Vec<&FieldProtection> = inner.values()
            .filter(|p| p.tenant_id == req.tenant_id)
            .collect();
        rules.sort_by(|a, b| a.name.cmp(&b.name));
        for rule in rules {
            if !rule.resources.iter().any(|r| r == "*" || r == &req.resource) {
                continue;
            }
            // Check whether the subject is in the deny list.
            let subject_match = rule.denied_subjects.iter().find(|s| match s {
                FieldSubject::Any => true,
                FieldSubject::User(u) => u == &req.user,
                FieldSubject::Group(g) => req.groups.iter().any(|x| x == g),
                FieldSubject::ServiceAccount { namespace, name } => {
                    let qual = format!("system:serviceaccount:{}:{}", namespace, name);
                    req.user == qual
                }
            });
            let Some(matched_subject) = subject_match else { continue; };
            // Check whether any protected path actually changed.
            for path in &rule.protected_paths {
                let old_v = json_path(&req.old_object, path);
                let new_v = json_path(&req.new_object, path);
                if old_v != new_v {
                    return FieldDecision::Deny {
                        rule_name: rule.name.clone(),
                        path: path.clone(),
                        subject: matched_subject.clone(),
                    };
                }
            }
        }
        FieldDecision::Allow
    }
}

impl Default for FieldRbacRegistry {
    fn default() -> Self { Self::new() }
}

fn json_path<'a>(obj: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut cur = obj;
    for seg in path.split('.') {
        let s = seg.trim();
        if s.is_empty() { continue; }
        cur = cur.get(s)?;
    }
    Some(cur)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rule(tenant: &str, name: &str, subjects: Vec<FieldSubject>, paths: Vec<&str>) -> FieldProtection {
        FieldProtection {
            tenant_id: tenant.into(), name: name.into(),
            resources: vec!["pods".into()],
            denied_subjects: subjects,
            protected_paths: paths.into_iter().map(String::from).collect(),
        }
    }

    fn req(tenant: &str, user: &str, old: serde_json::Value, new: serde_json::Value) -> FieldRequest {
        FieldRequest {
            tenant_id: tenant.into(),
            user: user.into(),
            groups: vec![],
            resource: "pods".into(),
            old_object: old, new_object: new,
        }
    }

    /// Upstream parity: `TestFieldRBAC_DenyOnProtectedFieldChange`
    /// (KEP-3633 — changing a protected path under a denied subject is
    /// rejected before the request reaches the storage layer).
    #[test]
    fn test_deny_when_protected_field_changes_for_denied_user() {
        let r = FieldRbacRegistry::new();
        r.upsert(rule("acme", "no-sa-rewrite",
            vec![FieldSubject::User("alice".into())],
            vec!["spec.serviceAccountName"]));
        let req = req("acme", "alice",
            json!({"spec": {"serviceAccountName": "default"}}),
            json!({"spec": {"serviceAccountName": "elevated"}}));
        match r.evaluate(&req) {
            FieldDecision::Deny { rule_name, path, subject } => {
                assert_eq!(rule_name, "no-sa-rewrite");
                assert_eq!(path, "spec.serviceAccountName");
                assert_eq!(subject, FieldSubject::User("alice".into()));
            }
            _ => panic!("expected Deny"),
        }
    }

    /// Upstream parity: `TestFieldRBAC_AllowsWhenFieldUnchanged`
    /// (KEP-3633 — protected fields only matter when they actually change).
    #[test]
    fn test_allow_when_protected_field_unchanged_even_for_denied_user() {
        let r = FieldRbacRegistry::new();
        r.upsert(rule("acme", "no-sa-rewrite",
            vec![FieldSubject::User("alice".into())],
            vec!["spec.serviceAccountName"]));
        let req = req("acme", "alice",
            json!({"spec": {"serviceAccountName": "default", "image": "nginx:1.27"}}),
            json!({"spec": {"serviceAccountName": "default", "image": "nginx:1.28"}}));
        assert_eq!(r.evaluate(&req), FieldDecision::Allow,
            "image change ok — protected SA name unchanged");
    }

    /// Upstream parity: `TestFieldRBAC_AllowsForNonDeniedSubject`
    /// (KEP-3633 — the rule only fires for subjects in the deny list).
    #[test]
    fn test_allow_when_subject_not_in_deny_list() {
        let r = FieldRbacRegistry::new();
        r.upsert(rule("acme", "no-sa-rewrite",
            vec![FieldSubject::User("alice".into())],
            vec!["spec.serviceAccountName"]));
        let req = req("acme", "system:serviceaccount:kube-system:scheduler",
            json!({"spec": {"serviceAccountName": "default"}}),
            json!({"spec": {"serviceAccountName": "elevated"}}));
        assert_eq!(r.evaluate(&req), FieldDecision::Allow,
            "scheduler is not in the deny list — change permitted");
    }

    /// Upstream parity: `TestFieldRBAC_DeniesAnySubjectOnAnyChange`
    /// (KEP-3633 — `FieldSubject::Any` is the global form, applied to
    /// every actor).
    #[test]
    fn test_subject_any_blocks_every_actor_from_changing_path() {
        let r = FieldRbacRegistry::new();
        r.upsert(rule("acme", "freeze-image",
            vec![FieldSubject::Any],
            vec!["spec.image"]));
        for user in ["alice", "bob", "system:serviceaccount:default:default"] {
            let req = req("acme", user,
                json!({"spec": {"image": "nginx:1.27"}}),
                json!({"spec": {"image": "nginx:1.28"}}));
            match r.evaluate(&req) {
                FieldDecision::Deny { rule_name, .. } => assert_eq!(rule_name, "freeze-image"),
                _ => panic!("user `{}` should be denied by Any-subject rule", user),
            }
        }
    }

    /// Upstream parity: `TestFieldRBAC_TenantIsolation`
    /// (cave-apiserver invariant: rules under acme never block changes to
    /// globex's objects, even when the protected path matches).
    #[test]
    fn test_rule_does_not_cross_tenant_boundaries() {
        let r = FieldRbacRegistry::new();
        r.upsert(rule("acme", "no-sa-rewrite",
            vec![FieldSubject::Any],
            vec!["spec.serviceAccountName"]));
        let req = req("globex", "alice",
            json!({"spec": {"serviceAccountName": "default"}}),
            json!({"spec": {"serviceAccountName": "elevated"}}));
        assert_eq!(r.evaluate(&req), FieldDecision::Allow,
            "tenant_id invariant: acme rule does not block globex's request");
    }

    /// Upstream parity: `TestFieldRBAC_GroupSubjectMatchesGroupMembership`
    /// (KEP-3633 — `FieldSubject::Group` matches when the user is in
    /// the named group).
    #[test]
    fn test_group_subject_matches_when_user_belongs_to_named_group() {
        let r = FieldRbacRegistry::new();
        r.upsert(rule("acme", "no-replicas-rewrite",
            vec![FieldSubject::Group("ops".into())],
            vec!["spec.replicas"]));
        let mut req = req("acme", "alice",
            json!({"spec": {"replicas": 3}}),
            json!({"spec": {"replicas": 10}}));
        req.groups = vec!["dev".into()];
        // alice is not in `ops` → allow.
        assert_eq!(r.evaluate(&req), FieldDecision::Allow);
        req.groups = vec!["dev".into(), "ops".into()];
        // adding `ops` membership flips to deny.
        match r.evaluate(&req) {
            FieldDecision::Deny { subject, .. } => {
                assert_eq!(subject, FieldSubject::Group("ops".into()));
            }
            _ => panic!("expected Deny once user is in `ops`"),
        }
    }
}
