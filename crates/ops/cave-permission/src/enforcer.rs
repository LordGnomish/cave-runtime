// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Casbin enforcer — in-memory policy store + management API.
//!
//! Line-port of casbin v3.10.0 (Apache-2.0):
//!   - `management_api.go` — `AddPolicy` / `RemovePolicy` / `HasPolicy` /
//!     `GetPolicy` / `AddGroupingPolicy` / `HasGroupingPolicy`.
//!
//! The enforcer models the canonical RBAC-with-resource-roles policy:
//!   `p = sub, obj, act`   (request `r = sub, obj, act`)
//!   `g = _, _`            (role inheritance, evaluated by [`RoleManager`])
//! Policy persistence (file / DB adapters), the generic CONF matcher DSL, and
//! the HTTP mutation surface stay out of scope per the manifest skips — this is
//! purely the runtime in-memory store the authorizer evaluates against.
//!
//! The [`enforce`](Enforcer::enforce) / [`batch_enforce`](Enforcer::batch_enforce)
//! decision functions live alongside the store (see the `enforce` impl block).

use crate::rbac::{RoleManager, DEFAULT_MAX_HIERARCHY_LEVEL};

/// A single `p` policy rule: `[sub, obj, act]`.
pub type PolicyRule = Vec<String>;

/// In-memory Casbin enforcer for the fixed RBAC-with-resource model.
///
/// Upstream: the policy-storage + management portion of `Enforcer` in
/// `enforcer.go` / `management_api.go`.
#[derive(Debug, Clone)]
pub struct Enforcer {
    /// `p` rules, each `[sub, obj, act]`.
    policies: Vec<PolicyRule>,
    /// `g` role inheritance graph.
    role_manager: RoleManager,
}

impl Default for Enforcer {
    fn default() -> Self {
        Self::new()
    }
}

impl Enforcer {
    /// Creates an empty enforcer with Casbin's default hierarchy bound.
    pub fn new() -> Self {
        Self {
            policies: Vec::new(),
            role_manager: RoleManager::new(DEFAULT_MAX_HIERARCHY_LEVEL),
        }
    }

    // ─── management_api.go — policy (`p`) rules ──────────────────────────────

    /// Adds a `p` rule. Returns `true` if added, `false` if it already exists
    /// (upstream `AddPolicy` returns the "rule affected" flag).
    pub fn add_policy(&mut self, sub: &str, obj: &str, act: &str) -> bool {
        let rule = vec![sub.to_string(), obj.to_string(), act.to_string()];
        if self.policies.contains(&rule) {
            return false;
        }
        self.policies.push(rule);
        true
    }

    /// Removes a `p` rule. Returns `true` if a rule was removed.
    /// Upstream: `RemovePolicy`.
    pub fn remove_policy(&mut self, sub: &str, obj: &str, act: &str) -> bool {
        let rule = vec![sub.to_string(), obj.to_string(), act.to_string()];
        if let Some(idx) = self.policies.iter().position(|r| *r == rule) {
            self.policies.remove(idx);
            true
        } else {
            false
        }
    }

    /// Whether a `p` rule is present. Upstream: `HasPolicy`.
    pub fn has_policy(&self, sub: &str, obj: &str, act: &str) -> bool {
        let rule = vec![sub.to_string(), obj.to_string(), act.to_string()];
        self.policies.contains(&rule)
    }

    /// All `p` rules, deterministically sorted. Upstream: `GetPolicy`.
    pub fn get_policy(&self) -> Vec<PolicyRule> {
        let mut out = self.policies.clone();
        out.sort();
        out
    }

    // ─── management_api.go — grouping (`g`) rules ────────────────────────────

    /// Adds a `g` rule (`child` inherits `parent`) and registers the link with
    /// the role manager. Returns `true` if newly added. Upstream:
    /// `AddGroupingPolicy`.
    pub fn add_grouping_policy(&mut self, child: &str, parent: &str) -> bool {
        if self.has_grouping_policy(child, parent) {
            return false;
        }
        self.role_manager.add_link(child, parent);
        true
    }

    /// Whether a `g` rule is present. Upstream: `HasGroupingPolicy`.
    pub fn has_grouping_policy(&self, child: &str, parent: &str) -> bool {
        self.role_manager.get_roles(child).iter().any(|r| r == parent)
    }

    /// The role manager backing the `g` rules (read access for enforce/tests).
    pub fn role_manager(&self) -> &RoleManager {
        &self.role_manager
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_has_policy() {
        let mut e = Enforcer::new();
        assert!(e.add_policy("alice", "data1", "read"));
        assert!(!e.add_policy("alice", "data1", "read"));
        assert!(e.has_policy("alice", "data1", "read"));
    }

    #[test]
    fn grouping_registers_link() {
        let mut e = Enforcer::new();
        assert!(e.add_grouping_policy("alice", "admin"));
        assert!(e.role_manager().has_link("alice", "admin"));
    }
}
