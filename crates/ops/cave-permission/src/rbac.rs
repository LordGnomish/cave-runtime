// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Casbin RBAC role manager + rbac_api role-graph queries.
//!
//! Line-port of casbin v3.10.0 (Apache-2.0):
//!   - `rbac/default-role-manager/role_manager.go` — `RoleManagerImpl`
//!     (`AddLink` / `DeleteLink` / `HasLink` / `GetRoles` / `GetUsers` /
//!     `GetImplicitRoles`), the default non-matching, single-domain manager.
//!   - `rbac_api.go` — `GetRolesForUser` / `GetUsersForRole` /
//!     `GetImplicitRolesForUser` query helpers.
//!
//! Upstream models the graph as a forest of `Role` nodes each holding a
//! `roles` map (what it inherits) and a `users` map (who inherits it). We model
//! the same bidirectional adjacency with two `BTreeMap`s keyed by name; the
//! `BTreeSet` values keep iteration deterministic (upstream relies on a
//! `sync.Map` plus a final de-dup, so order is unspecified — we make it stable).
//!
//! The matching-function, conditional, and multi-domain variants
//! (`ConditionalRoleManager`, `DomainManager`) are intentionally out of scope —
//! see the manifest skips (sub-component / parallel-track).

use std::collections::{BTreeMap, BTreeSet};

/// Casbin's default `maxHierarchyLevel` when none is supplied.
pub const DEFAULT_MAX_HIERARCHY_LEVEL: usize = 10;

/// Single-domain RBAC role manager.
///
/// Upstream: `RoleManagerImpl` in `rbac/default-role-manager/role_manager.go`.
#[derive(Debug, Clone, Default)]
pub struct RoleManager {
    /// name → roles it directly inherits (`Role.roles` upstream).
    roles: BTreeMap<String, BTreeSet<String>>,
    /// name → names that directly inherit it (`Role.users` upstream).
    users: BTreeMap<String, BTreeSet<String>>,
    /// Bound on inheritance-chain traversal (upstream `maxHierarchyLevel`).
    max_hierarchy_level: usize,
}

impl RoleManager {
    /// Upstream: `NewRoleManagerImpl(maxHierarchyLevel)`.
    pub fn new(max_hierarchy_level: usize) -> Self {
        Self {
            roles: BTreeMap::new(),
            users: BTreeMap::new(),
            max_hierarchy_level,
        }
    }

    /// Adds the inheritance link `name1` inherits `name2` (`g, name1, name2`).
    /// Upstream: `RoleManagerImpl.AddLink`.
    pub fn add_link(&mut self, name1: &str, name2: &str) {
        self.roles
            .entry(name1.to_string())
            .or_default()
            .insert(name2.to_string());
        self.users
            .entry(name2.to_string())
            .or_default()
            .insert(name1.to_string());
    }

    /// Removes the inheritance link between `name1` and `name2`.
    /// Upstream: `RoleManagerImpl.DeleteLink`.
    pub fn delete_link(&mut self, name1: &str, name2: &str) {
        if let Some(set) = self.roles.get_mut(name1) {
            set.remove(name2);
            if set.is_empty() {
                self.roles.remove(name1);
            }
        }
        if let Some(set) = self.users.get_mut(name2) {
            set.remove(name1);
            if set.is_empty() {
                self.users.remove(name2);
            }
        }
    }

    /// Determines whether `name1` inherits `name2`, directly or transitively,
    /// within `max_hierarchy_level` hops. Upstream: `RoleManagerImpl.HasLink`
    /// + `hasLinkHelper` (a breadth-first walk over the `roles` adjacency).
    pub fn has_link(&self, name1: &str, name2: &str) -> bool {
        if name1 == name2 {
            return true;
        }
        // BFS frontier, mirroring upstream's `roles map[string]*Role` per level.
        let mut frontier: BTreeSet<String> = BTreeSet::new();
        frontier.insert(name1.to_string());
        let mut level = self.max_hierarchy_level;
        while level > 0 && !frontier.is_empty() {
            let mut next: BTreeSet<String> = BTreeSet::new();
            for node in &frontier {
                if let Some(parents) = self.roles.get(node) {
                    if parents.contains(name2) {
                        return true;
                    }
                    for p in parents {
                        next.insert(p.clone());
                    }
                }
            }
            frontier = next;
            level -= 1;
        }
        false
    }

    /// Direct roles a name inherits. Upstream: `RoleManagerImpl.GetRoles`
    /// / `rbac_api.go GetRolesForUser`.
    pub fn get_roles(&self, name: &str) -> Vec<String> {
        self.roles
            .get(name)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Direct members of a role. Upstream: `RoleManagerImpl.GetUsers`
    /// / `rbac_api.go GetUsersForRole`.
    pub fn get_users(&self, name: &str) -> Vec<String> {
        self.users
            .get(name)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Transitive closure of roles a name inherits, respecting
    /// `max_hierarchy_level`. Upstream: `RoleManagerImpl.GetImplicitRoles`
    /// + `getImplicitRolesHelper` / `rbac_api.go GetImplicitRolesForUser`.
    pub fn get_implicit_roles(&self, name: &str) -> Vec<String> {
        let mut res: Vec<String> = Vec::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        seen.insert(name.to_string());
        let mut frontier: BTreeSet<String> = BTreeSet::new();
        frontier.insert(name.to_string());
        let mut level = 0;
        while level < self.max_hierarchy_level && !frontier.is_empty() {
            let mut next: BTreeSet<String> = BTreeSet::new();
            for node in &frontier {
                if let Some(parents) = self.roles.get(node) {
                    for p in parents {
                        if seen.insert(p.clone()) {
                            res.push(p.clone());
                            next.insert(p.clone());
                        }
                    }
                }
            }
            frontier = next;
            level += 1;
        }
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reflexive_and_direct() {
        let mut rm = RoleManager::new(DEFAULT_MAX_HIERARCHY_LEVEL);
        rm.add_link("u", "r");
        assert!(rm.has_link("u", "u"));
        assert!(rm.has_link("u", "r"));
        assert!(!rm.has_link("r", "u"));
    }

    #[test]
    fn implicit_closure_dedups() {
        let mut rm = RoleManager::new(DEFAULT_MAX_HIERARCHY_LEVEL);
        rm.add_link("a", "b");
        rm.add_link("b", "c");
        rm.add_link("a", "c"); // duplicate path to c
        let mut imp = rm.get_implicit_roles("a");
        imp.sort();
        assert_eq!(imp, vec!["b".to_string(), "c".to_string()]);
    }
}
