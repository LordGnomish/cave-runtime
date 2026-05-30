// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD coverage for the Casbin RBAC role manager + rbac_api role-graph
//! queries.
//!
//! Upstream (Apache-2.0, line-port permitted): casbin v3.10.0
//!   rbac/default-role-manager/role_manager.go — RoleManagerImpl
//!     (AddLink / DeleteLink / HasLink / GetRoles / GetUsers / GetImplicitRoles)
//!   rbac_api.go — GetRolesForUser / GetUsersForRole / GetImplicitRolesForUser
//!
//! The crate previously had no role-graph engine at all: `policy.rs` only held
//! `AllowAllPermissionPolicy`, so role inheritance (`g, alice, admin`) could not
//! be evaluated. This is the foundation Casbin's `g(r.sub, p.sub)` matcher rests
//! on. (Closes manifest skip: rbac_api.go GetRolesForUser/GetUsersForRole.)

use cave_permission::rbac::RoleManager;

#[test]
fn direct_link_is_inherited() {
    let mut rm = RoleManager::new(10);
    // alice inherits role admin.
    rm.add_link("alice", "admin");
    assert!(rm.has_link("alice", "admin"));
    // Reflexive: a name always "has link" to itself.
    assert!(rm.has_link("alice", "alice"));
    // No spurious links.
    assert!(!rm.has_link("admin", "alice"));
    assert!(!rm.has_link("bob", "admin"));
}

#[test]
fn transitive_link_is_inherited_within_hierarchy() {
    let mut rm = RoleManager::new(10);
    // alice -> admin -> root  (two hops)
    rm.add_link("alice", "admin");
    rm.add_link("admin", "root");
    assert!(rm.has_link("alice", "admin"));
    assert!(rm.has_link("alice", "root")); // transitive
    assert!(rm.has_link("admin", "root"));
}

#[test]
fn max_hierarchy_level_is_respected() {
    // With maxHierarchyLevel = 1, only direct links resolve.
    let mut rm = RoleManager::new(1);
    rm.add_link("alice", "admin");
    rm.add_link("admin", "root");
    assert!(rm.has_link("alice", "admin")); // depth 1
    assert!(!rm.has_link("alice", "root")); // depth 2 — beyond the level cap
}

#[test]
fn delete_link_removes_inheritance() {
    let mut rm = RoleManager::new(10);
    rm.add_link("alice", "admin");
    assert!(rm.has_link("alice", "admin"));
    rm.delete_link("alice", "admin");
    assert!(!rm.has_link("alice", "admin"));
}

#[test]
fn get_roles_and_users_are_directional() {
    let mut rm = RoleManager::new(10);
    rm.add_link("alice", "admin");
    rm.add_link("bob", "admin");
    // GetRolesForUser(alice) — direct roles alice belongs to.
    assert_eq!(rm.get_roles("alice"), vec!["admin".to_string()]);
    // GetUsersForRole(admin) — direct members (sorted for determinism).
    assert_eq!(
        rm.get_users("admin"),
        vec!["alice".to_string(), "bob".to_string()]
    );
    // Unknown name yields no roles/users, never panics.
    assert!(rm.get_roles("nobody").is_empty());
    assert!(rm.get_users("nobody").is_empty());
}

#[test]
fn get_implicit_roles_is_transitive_closure() {
    let mut rm = RoleManager::new(10);
    // alice -> admin -> root, and admin -> auditor
    rm.add_link("alice", "admin");
    rm.add_link("admin", "root");
    rm.add_link("admin", "auditor");
    let mut implicit = rm.get_implicit_roles("alice");
    implicit.sort();
    assert_eq!(
        implicit,
        vec![
            "admin".to_string(),
            "auditor".to_string(),
            "root".to_string()
        ]
    );
    // Direct get_roles only returns the first hop.
    assert_eq!(rm.get_roles("alice"), vec!["admin".to_string()]);
}
