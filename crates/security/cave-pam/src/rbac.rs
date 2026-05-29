// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Role-based access control for privileged access management.
//!
//! Implements Teleport's role model: roles carry allow/deny rules scoped to
//! resource kinds and actions. The policy engine evaluates them with
//! deny-overrides semantics (any explicit Deny wins over any Allow).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use uuid::Uuid;

// ── Domain types ──────────────────────────────────────────────────────────────

/// Categories of protected resources.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    Server,
    Database,
    Kubernetes,
    Application,
    WindowsDesktop,
}

/// Actions a principal can attempt on a resource.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Action {
    /// Open a session (SSH, DB, kubectl, RDP).
    Connect,
    /// Execute a command inside an existing session.
    Exec,
    /// Read audit recordings.
    ReadAudit,
    /// Approve access requests.
    ApproveRequest,
    /// View node inventory.
    ListResources,
}

/// Whether a rule permits or forbids an action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Allow,
    Deny,
}

/// Selects which resource names a rule applies to.
#[derive(Debug, Clone)]
pub enum ResourceSelector {
    /// Matches all resources of the given kind.
    All,
    /// Matches only resources whose name equals this string.
    Named(String),
    /// Matches resources whose name contains this prefix.
    Prefix(String),
}

impl ResourceSelector {
    /// Return true when this selector matches the provided resource name.
    pub fn matches(&self, name: &str) -> bool {
        match self {
            Self::All => true,
            Self::Named(n) => n == name,
            Self::Prefix(p) => name.starts_with(p.as_str()),
        }
    }
}

/// A single policy rule within a role.
#[derive(Debug, Clone)]
pub struct PolicyRule {
    pub resource_kind: ResourceKind,
    pub action: Action,
    pub effect: Effect,
    pub resource_selector: ResourceSelector,
}

/// A named set of policy rules that can be assigned to users.
#[derive(Debug, Clone)]
pub struct Role {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub rules: Vec<PolicyRule>,
}

/// A mapping that grants a specific role to a specific user.
#[derive(Debug, Clone)]
pub struct RoleAssignment {
    pub user_id: Uuid,
    pub role_id: Uuid,
}

/// A resource being accessed.
#[derive(Debug, Clone)]
pub struct Resource {
    pub kind: ResourceKind,
    pub name: String,
}

// ── Error type ────────────────────────────────────────────────────────────────

/// Errors produced by the RBAC subsystem.
#[derive(Debug, PartialEq, Clone)]
pub enum RbacError {
    RoleNotFound,
    AssignmentNotFound,
    DuplicateAssignment,
}

impl std::fmt::Display for RbacError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RoleNotFound => write!(f, "role not found"),
            Self::AssignmentNotFound => write!(f, "assignment not found"),
            Self::DuplicateAssignment => write!(f, "role already assigned to user"),
        }
    }
}

impl std::error::Error for RbacError {}

// ── Role store ────────────────────────────────────────────────────────────────

/// Thread-safe store for roles and user-role assignments.
#[derive(Debug, Default)]
pub struct RoleStore {
    roles: Arc<RwLock<HashMap<Uuid, Role>>>,
    /// user_id → set of role_ids
    assignments: Arc<RwLock<HashMap<Uuid, HashSet<Uuid>>>>,
}

impl RoleStore {
    /// Create a new empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new role. Returns its ID.
    pub fn register(&self, role: Role) -> Result<Uuid, RbacError> {
        let id = role.id;
        self.roles.write().unwrap().insert(id, role);
        Ok(id)
    }

    /// Assign a role to a user.
    pub fn assign(&self, assignment: RoleAssignment) -> Result<(), RbacError> {
        let mut map = self.assignments.write().unwrap();
        let set = map.entry(assignment.user_id).or_insert_with(HashSet::new);
        if !set.insert(assignment.role_id) {
            return Err(RbacError::DuplicateAssignment);
        }
        Ok(())
    }

    /// Remove a role assignment from a user.
    pub fn deassign(&self, user_id: &Uuid, role_id: &Uuid) -> Result<(), RbacError> {
        let mut map = self.assignments.write().unwrap();
        let removed = map
            .get_mut(user_id)
            .map(|set| set.remove(role_id))
            .unwrap_or(false);
        if removed {
            Ok(())
        } else {
            Err(RbacError::AssignmentNotFound)
        }
    }

    /// Return the list of Role objects assigned to a user.
    pub fn roles_for_user(&self, user_id: &Uuid) -> Vec<Role> {
        let role_ids: Vec<Uuid> = self
            .assignments
            .read()
            .unwrap()
            .get(user_id)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default();

        let roles = self.roles.read().unwrap();
        role_ids
            .iter()
            .filter_map(|id| roles.get(id).cloned())
            .collect()
    }

    /// Retrieve a single role by ID.
    pub fn get_role(&self, role_id: &Uuid) -> Option<Role> {
        self.roles.read().unwrap().get(role_id).cloned()
    }

    /// Return all registered roles.
    pub fn list_roles(&self) -> Vec<Role> {
        self.roles.read().unwrap().values().cloned().collect()
    }
}

// ── Policy engine ─────────────────────────────────────────────────────────────

/// Evaluates access requests against the roles assigned to a principal.
///
/// Evaluation semantics: explicit Deny in any assigned role overrides any
/// Allow in any other role (deny-overrides / Teleport default).
pub struct PolicyEngine {
    store: RoleStore,
}

impl PolicyEngine {
    /// Create an engine backed by the given role store.
    pub fn new(store: RoleStore) -> Self {
        Self { store }
    }

    /// Return true if `user_id` is allowed to perform `action` on `resource`.
    ///
    /// Algorithm:
    /// 1. Collect all roles for the user.
    /// 2. For each matching rule (same kind + action + selector match), track
    ///    whether we have at least one Allow and at least one Deny.
    /// 3. If any Deny → false (deny-overrides).
    /// 4. If any Allow → true.
    /// 5. Otherwise → false (default-deny).
    pub fn is_allowed(&self, user_id: &Uuid, resource: &Resource, action: Action) -> bool {
        let roles = self.store.roles_for_user(user_id);
        let mut has_allow = false;
        let mut has_deny = false;

        for role in &roles {
            for rule in &role.rules {
                if rule.resource_kind != resource.kind {
                    continue;
                }
                if rule.action != action {
                    continue;
                }
                if !rule.resource_selector.matches(&resource.name) {
                    continue;
                }
                match rule.effect {
                    Effect::Allow => has_allow = true,
                    Effect::Deny => has_deny = true,
                }
            }
        }

        if has_deny {
            return false;
        }
        has_allow
    }

    /// Return the list of resource kinds a user can connect to.
    pub fn connectable_kinds(&self, user_id: &Uuid) -> Vec<ResourceKind> {
        let roles = self.store.roles_for_user(user_id);
        let mut kinds: HashSet<ResourceKind> = HashSet::new();
        for role in &roles {
            for rule in &role.rules {
                if rule.action == Action::Connect && rule.effect == Effect::Allow {
                    kinds.insert(rule.resource_kind.clone());
                }
            }
        }
        kinds.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simple_role(name: &str, kind: ResourceKind, action: Action, effect: Effect) -> Role {
        Role {
            id: Uuid::new_v4(),
            name: name.to_string(),
            description: String::new(),
            rules: vec![PolicyRule {
                resource_kind: kind,
                action,
                effect,
                resource_selector: ResourceSelector::All,
            }],
        }
    }

    #[test]
    fn default_deny_with_no_roles() {
        let store = RoleStore::new();
        let engine = PolicyEngine::new(store);
        let resource = Resource { kind: ResourceKind::Server, name: "s".to_string() };
        assert!(!engine.is_allowed(&Uuid::new_v4(), &resource, Action::Connect));
    }

    #[test]
    fn named_selector_matches_exact() {
        let sel = ResourceSelector::Named("db-prod".to_string());
        assert!(sel.matches("db-prod"));
        assert!(!sel.matches("db-staging"));
    }

    #[test]
    fn prefix_selector_matches() {
        let sel = ResourceSelector::Prefix("prod-".to_string());
        assert!(sel.matches("prod-db-01"));
        assert!(!sel.matches("staging-db-01"));
    }

    #[test]
    fn connectable_kinds_empty_when_no_roles() {
        let store = RoleStore::new();
        let engine = PolicyEngine::new(store);
        assert!(engine.connectable_kinds(&Uuid::new_v4()).is_empty());
    }

    #[test]
    fn connectable_kinds_returns_allowed_kinds() {
        let store = RoleStore::new();
        let role = simple_role("db-access", ResourceKind::Database, Action::Connect, Effect::Allow);
        let role_id = store.register(role).unwrap();
        let user_id = Uuid::new_v4();
        store.assign(RoleAssignment { user_id, role_id }).unwrap();
        let engine = PolicyEngine::new(store);
        let kinds = engine.connectable_kinds(&user_id);
        assert_eq!(kinds.len(), 1);
        assert!(kinds.contains(&ResourceKind::Database));
    }
}
