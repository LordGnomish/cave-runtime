// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! RBAC primitives — first-party role-based access control for CAVE modules.
//!
//! A small, dependency-free authorization core that other cave-* crates can
//! adopt directly. The model mirrors the familiar Kubernetes-style shape
//! (verbs × resources, roles, subject→role bindings) but is intentionally
//! minimal and self-contained:
//!
//! - [`Permission`] grants a `verb` on a `resource`, with `*` acting as a
//!   wildcard that matches any value on either field.
//! - [`Role`] is a named bundle of permissions.
//! - [`Policy`] holds the set of known roles plus subject→role bindings, and
//!   answers authorization questions via [`Policy::evaluate`].
//! - [`Decision`] is the deny-by-default verdict.
//!
//! Evaluation is **deny-by-default**: access is granted only when some role
//! bound to the subject contains a permission whose verb and resource both
//! match the request. Unknown subjects, unbound subjects, and bindings that
//! reference unknown roles all resolve to [`Decision::Deny`].

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// The wildcard token that matches any verb or any resource.
pub const WILDCARD: &str = "*";

/// A single grant: permission to perform `verb` on `resource`.
///
/// Either field may be the [`WILDCARD`] (`*`) token, in which case it matches
/// any concrete value for that field. A permission of `{ verb: "*", resource:
/// "*" }` therefore grants everything.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Permission {
    /// Action being authorized, e.g. `get`, `create`, or `*`.
    pub verb: String,
    /// Object the action applies to, e.g. `pods`, `secrets`, or `*`.
    pub resource: String,
}

impl Permission {
    /// Construct a permission from any string-like verb and resource.
    pub fn new(verb: impl Into<String>, resource: impl Into<String>) -> Self {
        Self {
            verb: verb.into(),
            resource: resource.into(),
        }
    }

    /// A permission that grants every verb on every resource (`* / *`).
    pub fn allow_all() -> Self {
        Self::new(WILDCARD, WILDCARD)
    }

    /// Whether this permission authorizes `verb` on `resource`.
    ///
    /// A field matches when it is exactly equal to the requested value or when
    /// it is the [`WILDCARD`] token. Both fields must match.
    pub fn matches(&self, verb: &str, resource: &str) -> bool {
        field_matches(&self.verb, verb) && field_matches(&self.resource, resource)
    }
}

/// `true` if `pattern` matches `value` exactly or `pattern` is the wildcard.
fn field_matches(pattern: &str, value: &str) -> bool {
    pattern == WILDCARD || pattern == value
}

/// A named bundle of [`Permission`]s.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Role {
    /// Unique role name, referenced by [`Policy`] bindings.
    pub name: String,
    /// Permissions granted by this role.
    pub permissions: Vec<Permission>,
}

impl Role {
    /// Construct a role with no permissions.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            permissions: Vec::new(),
        }
    }

    /// Builder: add a permission and return `self`.
    pub fn with_permission(mut self, verb: impl Into<String>, resource: impl Into<String>) -> Self {
        self.permissions.push(Permission::new(verb, resource));
        self
    }

    /// Whether any permission in this role authorizes `verb` on `resource`.
    pub fn grants(&self, verb: &str, resource: &str) -> bool {
        self.permissions.iter().any(|p| p.matches(verb, resource))
    }
}

/// The verdict of an authorization check. Deny-by-default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    /// Access is permitted.
    Allow,
    /// Access is refused (the default in the absence of a matching grant).
    Deny,
}

impl Decision {
    /// Convenience predicate: `true` only for [`Decision::Allow`].
    pub fn is_allowed(self) -> bool {
        matches!(self, Decision::Allow)
    }
}

/// A complete authorization policy: known roles plus subject→role bindings.
///
/// Subjects are opaque identifiers (e.g. a `cave_uid`, service account, or
/// group name). A subject may be bound to multiple roles; the effective set of
/// permissions is the union of all roles bound to it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Policy {
    /// All roles known to this policy.
    pub roles: Vec<Role>,
    /// Map from subject identifier to the names of roles bound to it.
    #[serde(default)]
    pub bindings: HashMap<String, Vec<String>>,
}

impl Policy {
    /// An empty policy that denies everything.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builder: register a role and return `self`.
    pub fn with_role(mut self, role: Role) -> Self {
        self.roles.push(role);
        self
    }

    /// Builder: bind `subject` to `role_name` and return `self`.
    ///
    /// Multiple calls accumulate; a subject may hold many roles. Binding a
    /// subject to a role name that is never defined is harmless — it simply
    /// contributes no permissions during evaluation.
    pub fn with_binding(
        mut self,
        subject: impl Into<String>,
        role_name: impl Into<String>,
    ) -> Self {
        self.bindings
            .entry(subject.into())
            .or_default()
            .push(role_name.into());
        self
    }

    /// Look up a role by name.
    fn role(&self, name: &str) -> Option<&Role> {
        self.roles.iter().find(|r| r.name == name)
    }

    /// Decide whether `subject` may perform `verb` on `resource`.
    ///
    /// Deny-by-default: returns [`Decision::Allow`] only when at least one role
    /// bound to `subject` contains a permission whose verb and resource both
    /// match (exactly or via the `*` wildcard). Unknown subjects, subjects with
    /// no bindings, and bindings to undefined roles all yield
    /// [`Decision::Deny`].
    pub fn evaluate(&self, subject: &str, verb: &str, resource: &str) -> Decision {
        let Some(role_names) = self.bindings.get(subject) else {
            return Decision::Deny;
        };

        let allowed = role_names
            .iter()
            .filter_map(|name| self.role(name))
            .any(|role| role.grants(verb, resource));

        if allowed {
            Decision::Allow
        } else {
            Decision::Deny
        }
    }

    /// Ergonomic boolean form of [`Policy::evaluate`].
    pub fn is_allowed(&self, subject: &str, verb: &str, resource: &str) -> bool {
        self.evaluate(subject, verb, resource).is_allowed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reader role granting `get` on `pods`, bound to `alice`.
    fn reader_policy() -> Policy {
        Policy::new()
            .with_role(Role::new("reader").with_permission("get", "pods"))
            .with_binding("alice", "reader")
    }

    #[test]
    fn test_exact_allow() {
        let policy = reader_policy();
        assert_eq!(policy.evaluate("alice", "get", "pods"), Decision::Allow);
    }

    #[test]
    fn test_wildcard_verb() {
        let policy = Policy::new()
            .with_role(Role::new("pod-admin").with_permission("*", "pods"))
            .with_binding("alice", "pod-admin");
        // Any verb on pods is allowed...
        assert_eq!(policy.evaluate("alice", "get", "pods"), Decision::Allow);
        assert_eq!(policy.evaluate("alice", "delete", "pods"), Decision::Allow);
        // ...but the wildcard is scoped to the resource.
        assert_eq!(policy.evaluate("alice", "get", "secrets"), Decision::Deny);
    }

    #[test]
    fn test_wildcard_resource() {
        let policy = Policy::new()
            .with_role(Role::new("getter").with_permission("get", "*"))
            .with_binding("alice", "getter");
        // `get` on any resource is allowed...
        assert_eq!(policy.evaluate("alice", "get", "pods"), Decision::Allow);
        assert_eq!(policy.evaluate("alice", "get", "secrets"), Decision::Allow);
        // ...but the wildcard is scoped to the verb.
        assert_eq!(policy.evaluate("alice", "delete", "pods"), Decision::Deny);
    }

    #[test]
    fn test_double_wildcard_allows_all() {
        let policy = Policy::new()
            .with_role(Role::new("super").with_permission("*", "*"))
            .with_binding("root", "super");
        assert_eq!(policy.evaluate("root", "delete", "secrets"), Decision::Allow);
        assert_eq!(policy.evaluate("root", "anything", "everything"), Decision::Allow);
    }

    #[test]
    fn test_deny_by_default_unmatched_verb() {
        let policy = reader_policy();
        // `reader` only grants `get`; `delete` must be denied.
        assert_eq!(policy.evaluate("alice", "delete", "pods"), Decision::Deny);
    }

    #[test]
    fn test_deny_by_default_unmatched_resource() {
        let policy = reader_policy();
        // `reader` only grants on `pods`; `secrets` must be denied.
        assert_eq!(policy.evaluate("alice", "get", "secrets"), Decision::Deny);
    }

    #[test]
    fn test_multi_role_union() {
        // alice holds two roles; the effective grant is their union.
        let policy = Policy::new()
            .with_role(Role::new("pod-reader").with_permission("get", "pods"))
            .with_role(Role::new("secret-writer").with_permission("create", "secrets"))
            .with_binding("alice", "pod-reader")
            .with_binding("alice", "secret-writer");

        assert_eq!(policy.evaluate("alice", "get", "pods"), Decision::Allow);
        assert_eq!(policy.evaluate("alice", "create", "secrets"), Decision::Allow);
        // Neither role grants this combination.
        assert_eq!(policy.evaluate("alice", "delete", "pods"), Decision::Deny);
        assert_eq!(policy.evaluate("alice", "get", "secrets"), Decision::Deny);
    }

    #[test]
    fn test_unknown_subject_denied() {
        let policy = reader_policy();
        // `mallory` has no binding at all.
        assert_eq!(policy.evaluate("mallory", "get", "pods"), Decision::Deny);
    }

    #[test]
    fn test_permission_matches_exact() {
        let p = Permission::new("get", "pods");
        assert!(p.matches("get", "pods"));
        assert!(!p.matches("get", "secrets"));
        assert!(!p.matches("delete", "pods"));
    }

    #[test]
    fn test_permission_matches_wildcards() {
        assert!(Permission::new("*", "pods").matches("delete", "pods"));
        assert!(Permission::new("get", "*").matches("get", "configmaps"));
        assert!(Permission::allow_all().matches("whatever", "whichever"));
        // A concrete verb does not match a different verb even with wildcard resource.
        assert!(!Permission::new("get", "*").matches("list", "pods"));
    }

    #[test]
    fn test_decision_is_allowed() {
        assert!(Decision::Allow.is_allowed());
        assert!(!Decision::Deny.is_allowed());
    }

    #[test]
    fn test_binding_to_unknown_role_is_ignored() {
        // The binding references "ghost", which is never defined as a role.
        let policy = Policy::new()
            .with_role(Role::new("reader").with_permission("get", "pods"))
            .with_binding("alice", "ghost");
        assert_eq!(policy.evaluate("alice", "get", "pods"), Decision::Deny);
        // The convenience boolean form agrees.
        assert!(!policy.is_allowed("alice", "get", "pods"));
    }

    #[test]
    fn test_empty_policy_denies() {
        let policy = Policy::new();
        assert_eq!(policy.evaluate("anyone", "get", "anything"), Decision::Deny);
    }

    #[test]
    fn test_builder_roundtrip_serde() {
        let policy = reader_policy();
        let json = serde_json::to_string(&policy).expect("serialize");
        let restored: Policy = serde_json::from_str(&json).expect("deserialize");
        // Behavior survives a serde round-trip.
        assert_eq!(restored.evaluate("alice", "get", "pods"), Decision::Allow);
        assert_eq!(restored.evaluate("alice", "delete", "pods"), Decision::Deny);
        // Decision serializes lowercase.
        assert_eq!(
            serde_json::to_string(&Decision::Allow).unwrap(),
            "\"allow\""
        );
    }
}
