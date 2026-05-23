// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Authorization — RBAC + Node + Webhook authorizers.
//!
//! Mirrors `plugin/pkg/auth/authorizer` of upstream Kubernetes.  The
//! chain is consulted in order; each authorizer returns one of
//! `Allow`, `Deny(reason)`, or `NoOpinion`.  The first explicit
//! `Allow`/`Deny` wins; if every authorizer returns `NoOpinion` the
//! chain default is `Deny`.

use crate::authn::Identity;
use crate::error::Error;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verb {
    Get,
    List,
    Watch,
    Create,
    Update,
    Patch,
    Delete,
    DeleteCollection,
    Connect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attributes {
    pub user: Identity,
    pub verb: Verb,
    pub api_group: String,
    pub resource: String,
    pub namespace: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthzDecision {
    Allow,
    Deny(String),
    NoOpinion,
}

pub trait Authorizer: Send + Sync {
    fn name(&self) -> &'static str;
    fn authorize(&self, attrs: &Attributes) -> AuthzDecision;
}

pub struct ChainAuthorizer {
    authorizers: Vec<Box<dyn Authorizer>>,
}

impl Default for ChainAuthorizer {
    fn default() -> Self {
        Self::new()
    }
}

impl ChainAuthorizer {
    pub fn new() -> Self {
        Self {
            authorizers: Vec::new(),
        }
    }
    pub fn add(mut self, a: Box<dyn Authorizer>) -> Self {
        self.authorizers.push(a);
        self
    }
    pub fn authorize(&self, attrs: &Attributes) -> Result<(), Error> {
        for a in &self.authorizers {
            match a.authorize(attrs) {
                AuthzDecision::Allow => return Ok(()),
                AuthzDecision::Deny(r) => {
                    return Err(Error::Forbidden(format!("{}: {}", a.name(), r)));
                }
                AuthzDecision::NoOpinion => continue,
            }
        }
        Err(Error::Forbidden("no opinion in chain".into()))
    }
}

// ─── RBAC ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyRule {
    pub api_groups: Vec<String>,
    pub resources: Vec<String>,
    pub resource_names: Vec<String>,
    pub verbs: Vec<Verb>,
}

impl PolicyRule {
    pub fn matches(&self, attrs: &Attributes) -> bool {
        let group_ok = self.api_groups.iter().any(|g| g == "*" || g == &attrs.api_group);
        let res_ok = self.resources.iter().any(|r| r == "*" || r == &attrs.resource);
        let name_ok = self.resource_names.is_empty()
            || self
                .resource_names
                .iter()
                .any(|n| Some(n) == attrs.name.as_ref());
        let verb_ok = self.verbs.iter().any(|v| *v == attrs.verb);
        group_ok && res_ok && name_ok && verb_ok
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Role {
    pub name: String,
    pub namespace: Option<String>,
    pub rules: Vec<PolicyRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub name: String,
    /// `Some(ns)` for RoleBinding, `None` for ClusterRoleBinding.
    pub namespace: Option<String>,
    pub role_name: String,
    /// True when the bound role is a ClusterRole.
    pub cluster_role: bool,
    pub subjects: Vec<Subject>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subject {
    pub kind: SubjectKind,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubjectKind {
    User,
    Group,
    ServiceAccount,
}

pub struct RbacAuthorizer {
    pub roles: std::sync::Arc<std::sync::RwLock<Vec<Role>>>,
    pub cluster_roles: std::sync::Arc<std::sync::RwLock<Vec<Role>>>,
    pub bindings: std::sync::Arc<std::sync::RwLock<Vec<Binding>>>,
}

impl Default for RbacAuthorizer {
    fn default() -> Self {
        Self {
            roles: std::sync::Arc::new(std::sync::RwLock::new(Vec::new())),
            cluster_roles: std::sync::Arc::new(std::sync::RwLock::new(Vec::new())),
            bindings: std::sync::Arc::new(std::sync::RwLock::new(Vec::new())),
        }
    }
}

impl RbacAuthorizer {
    pub fn add_role(&self, r: Role) {
        if r.namespace.is_some() {
            self.roles.write().expect("rbac").push(r);
        } else {
            self.cluster_roles.write().expect("rbac").push(r);
        }
    }

    pub fn bind(&self, b: Binding) {
        self.bindings.write().expect("rbac").push(b);
    }

    fn matches_subject(s: &Subject, id: &Identity) -> bool {
        match s.kind {
            SubjectKind::User => s.name == id.user,
            SubjectKind::Group => id.groups.iter().any(|g| g == &s.name),
            SubjectKind::ServiceAccount => id.user.starts_with("system:serviceaccount:") && id.user.ends_with(&format!(":{}", s.name)),
        }
    }
}

impl Authorizer for RbacAuthorizer {
    fn name(&self) -> &'static str {
        "RBAC"
    }
    fn authorize(&self, attrs: &Attributes) -> AuthzDecision {
        let bindings = self.bindings.read().expect("rbac");
        for b in bindings.iter() {
            // Namespace scope filter — RoleBinding restricts to its own namespace.
            if let Some(bns) = &b.namespace {
                if Some(bns) != attrs.namespace.as_ref() {
                    continue;
                }
            }
            if !b.subjects.iter().any(|s| Self::matches_subject(s, &attrs.user)) {
                continue;
            }
            let role_set = if b.cluster_role {
                self.cluster_roles.read().expect("rbac").clone()
            } else {
                self.roles.read().expect("rbac").clone()
            };
            for role in role_set.iter().filter(|r| r.name == b.role_name) {
                for rule in &role.rules {
                    if rule.matches(attrs) {
                        return AuthzDecision::Allow;
                    }
                }
            }
        }
        AuthzDecision::NoOpinion
    }
}

// ─── Node restriction ───────────────────────────────────────────────────────

/// Mirrors `plugin/pkg/admission/noderestriction`. Restricts kubelet
/// (group `system:nodes`) to mutating only its own Node/Pod records.
pub struct NodeAuthorizer;

impl Authorizer for NodeAuthorizer {
    fn name(&self) -> &'static str {
        "Node"
    }
    fn authorize(&self, attrs: &Attributes) -> AuthzDecision {
        if !attrs.user.user.starts_with("system:node:") {
            return AuthzDecision::NoOpinion;
        }
        let node = attrs.user.user.trim_start_matches("system:node:");
        match (attrs.resource.as_str(), &attrs.name) {
            ("nodes", Some(n)) if n == node => AuthzDecision::Allow,
            ("pods", _) => AuthzDecision::Allow,
            ("configmaps", _) | ("secrets", _) => AuthzDecision::Allow,
            ("nodes", Some(other)) => AuthzDecision::Deny(format!(
                "kubelet {} may not act on node {}",
                node, other
            )),
            _ => AuthzDecision::NoOpinion,
        }
    }
}

// ─── Webhook ────────────────────────────────────────────────────────────────

pub struct WebhookAuthorizer {
    pub allow_users: std::sync::Arc<std::sync::RwLock<std::collections::HashSet<String>>>,
    pub deny_users: std::sync::Arc<std::sync::RwLock<std::collections::HashSet<String>>>,
}

impl Default for WebhookAuthorizer {
    fn default() -> Self {
        Self {
            allow_users: std::sync::Arc::new(std::sync::RwLock::new(Default::default())),
            deny_users: std::sync::Arc::new(std::sync::RwLock::new(Default::default())),
        }
    }
}

impl WebhookAuthorizer {
    pub fn allow(&self, user: impl Into<String>) {
        self.allow_users.write().expect("wh").insert(user.into());
    }
    pub fn deny(&self, user: impl Into<String>) {
        self.deny_users.write().expect("wh").insert(user.into());
    }
}

impl Authorizer for WebhookAuthorizer {
    fn name(&self) -> &'static str {
        "Webhook"
    }
    fn authorize(&self, attrs: &Attributes) -> AuthzDecision {
        if self.deny_users.read().expect("wh").contains(&attrs.user.user) {
            return AuthzDecision::Deny(format!("webhook deny-listed {}", attrs.user.user));
        }
        if self.allow_users.read().expect("wh").contains(&attrs.user.user) {
            return AuthzDecision::Allow;
        }
        AuthzDecision::NoOpinion
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(user: &str, groups: &[&str]) -> Identity {
        Identity {
            user: user.into(),
            groups: groups.iter().map(|s| s.to_string()).collect(),
            uid: None,
            source: "test",
        }
    }

    fn attrs(user: &str, verb: Verb, resource: &str) -> Attributes {
        Attributes {
            user: id(user, &[]),
            verb,
            api_group: "".into(),
            resource: resource.into(),
            namespace: Some("default".into()),
            name: None,
        }
    }

    #[test]
    fn empty_chain_denies() {
        let c = ChainAuthorizer::new();
        let e = c.authorize(&attrs("alice", Verb::Get, "pods")).unwrap_err();
        assert!(matches!(e, Error::Forbidden(_)));
    }

    #[test]
    fn rbac_allows_via_role_binding() {
        let r = RbacAuthorizer::default();
        r.add_role(Role {
            name: "viewer".into(),
            namespace: Some("default".into()),
            rules: vec![PolicyRule {
                api_groups: vec!["".into()],
                resources: vec!["pods".into()],
                resource_names: vec![],
                verbs: vec![Verb::Get, Verb::List],
            }],
        });
        r.bind(Binding {
            name: "alice-view".into(),
            namespace: Some("default".into()),
            role_name: "viewer".into(),
            cluster_role: false,
            subjects: vec![Subject {
                kind: SubjectKind::User,
                name: "alice".into(),
            }],
        });
        let c = ChainAuthorizer::new().add(Box::new(r));
        c.authorize(&attrs("alice", Verb::Get, "pods")).unwrap();
        assert!(matches!(
            c.authorize(&attrs("alice", Verb::Delete, "pods")),
            Err(Error::Forbidden(_))
        ));
    }

    #[test]
    fn rbac_cluster_role_binding_crosses_namespaces() {
        let r = RbacAuthorizer::default();
        r.add_role(Role {
            name: "cluster-viewer".into(),
            namespace: None,
            rules: vec![PolicyRule {
                api_groups: vec!["*".into()],
                resources: vec!["*".into()],
                resource_names: vec![],
                verbs: vec![Verb::Get],
            }],
        });
        r.bind(Binding {
            name: "bob-cv".into(),
            namespace: None,
            role_name: "cluster-viewer".into(),
            cluster_role: true,
            subjects: vec![Subject {
                kind: SubjectKind::User,
                name: "bob".into(),
            }],
        });
        let c = ChainAuthorizer::new().add(Box::new(r));
        let mut a = attrs("bob", Verb::Get, "secrets");
        a.namespace = Some("kube-system".into());
        c.authorize(&a).unwrap();
    }

    #[test]
    fn node_authorizer_restricts_to_own_node() {
        let n = NodeAuthorizer;
        let mut a = Attributes {
            user: id("system:node:n1", &["system:nodes"]),
            verb: Verb::Update,
            api_group: "".into(),
            resource: "nodes".into(),
            namespace: None,
            name: Some("n1".into()),
        };
        assert_eq!(n.authorize(&a), AuthzDecision::Allow);
        a.name = Some("n2".into());
        assert!(matches!(n.authorize(&a), AuthzDecision::Deny(_)));
    }

    #[test]
    fn node_authorizer_allows_pod_actions() {
        let n = NodeAuthorizer;
        let a = Attributes {
            user: id("system:node:n3", &["system:nodes"]),
            verb: Verb::Get,
            api_group: "".into(),
            resource: "pods".into(),
            namespace: Some("default".into()),
            name: None,
        };
        assert_eq!(n.authorize(&a), AuthzDecision::Allow);
    }

    #[test]
    fn node_authorizer_ignores_non_nodes_users() {
        let n = NodeAuthorizer;
        let a = attrs("alice", Verb::Get, "nodes");
        assert_eq!(n.authorize(&a), AuthzDecision::NoOpinion);
    }

    #[test]
    fn webhook_priority_deny_over_allow() {
        let w = WebhookAuthorizer::default();
        w.allow("alice");
        w.deny("alice");
        let c = ChainAuthorizer::new().add(Box::new(w));
        assert!(matches!(
            c.authorize(&attrs("alice", Verb::Get, "pods")),
            Err(Error::Forbidden(_))
        ));
    }

    #[test]
    fn webhook_no_opinion_lets_chain_continue() {
        let w = WebhookAuthorizer::default();
        let r = RbacAuthorizer::default();
        r.add_role(Role {
            name: "cluster-admin".into(),
            namespace: None,
            rules: vec![PolicyRule {
                api_groups: vec!["*".into()],
                resources: vec!["*".into()],
                resource_names: vec![],
                verbs: vec![Verb::Get, Verb::Update],
            }],
        });
        r.bind(Binding {
            name: "alice-admin".into(),
            namespace: None,
            role_name: "cluster-admin".into(),
            cluster_role: true,
            subjects: vec![Subject {
                kind: SubjectKind::User,
                name: "alice".into(),
            }],
        });
        let c = ChainAuthorizer::new()
            .add(Box::new(w))
            .add(Box::new(r));
        c.authorize(&attrs("alice", Verb::Get, "pods")).unwrap();
    }

    #[test]
    fn rbac_group_subject() {
        let r = RbacAuthorizer::default();
        r.add_role(Role {
            name: "devs".into(),
            namespace: Some("default".into()),
            rules: vec![PolicyRule {
                api_groups: vec!["".into()],
                resources: vec!["pods".into()],
                resource_names: vec![],
                verbs: vec![Verb::Watch],
            }],
        });
        r.bind(Binding {
            name: "dev-group".into(),
            namespace: Some("default".into()),
            role_name: "devs".into(),
            cluster_role: false,
            subjects: vec![Subject {
                kind: SubjectKind::Group,
                name: "developers".into(),
            }],
        });
        let c = ChainAuthorizer::new().add(Box::new(r));
        let mut a = attrs("eve", Verb::Watch, "pods");
        a.user.groups = vec!["developers".into()];
        c.authorize(&a).unwrap();
    }

    #[test]
    fn policy_rule_wildcards() {
        let rule = PolicyRule {
            api_groups: vec!["*".into()],
            resources: vec!["*".into()],
            resource_names: vec![],
            verbs: vec![Verb::Get],
        };
        let a = attrs("x", Verb::Get, "anything");
        assert!(rule.matches(&a));
    }
}
