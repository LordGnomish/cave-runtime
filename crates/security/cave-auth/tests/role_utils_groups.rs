// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD line-port of Keycloak group membership + group-role inheritance.
// Upstream (Apache-2.0, v26.6.2):
//   server-spi/.../models/utils/RoleUtils.java
//       isMember(groups, target), isDirectMember(groups, target),
//       hasRoleFromGroup(group, target, checkParentGroup)
//   model/jpa/.../GroupAdapter.java
//       hasRole(role): RoleUtils.hasRole(ownRoles, role) || parent?.hasRole(role)
//
// Semantics pinned here:
//   * membership is transitive up the parent chain,
//   * a group's effective roles include its own (composite-expanded) roles AND
//     every role inherited from its ancestor groups.

use cave_auth::role_utils::{GroupGraph, RoleGraph};

/// Roles: R_admin -> R_read (composite). R_write is unrelated.
fn roles() -> RoleGraph {
    let mut g = RoleGraph::new();
    g.add_composite("R_admin", "R_read");
    g
}

/// Groups: root <- child <- grandchild ; root holds R_admin.
fn groups() -> GroupGraph {
    let mut g = GroupGraph::new();
    g.set_parent("child", "root");
    g.set_parent("grandchild", "child");
    g.assign_role("root", "R_admin");
    g
}

#[test]
fn is_member_walks_parent_chain() {
    let g = groups();
    // grandchild is (indirectly) a member of child and root.
    assert!(g.is_member(&["grandchild".into()], "root"));
    assert!(g.is_member(&["grandchild".into()], "child"));
    // contains-directly also counts as member.
    assert!(g.is_member(&["root".into()], "root"));
    // downward / sibling relationships are NOT membership.
    assert!(!g.is_member(&["child".into()], "grandchild"));
    assert!(!g.is_member(&["root".into()], "child"));
    // unknown target.
    assert!(!g.is_member(&["grandchild".into()], "elsewhere"));
}

#[test]
fn is_direct_member_only_exact() {
    let g = groups();
    assert!(g.is_direct_member(&["grandchild".into(), "child".into()], "child"));
    assert!(!g.is_direct_member(&["grandchild".into()], "root"));
    assert!(!g.is_direct_member(&[], "root"));
}

#[test]
fn group_has_role_includes_own_composite_and_inherited() {
    let rg = roles();
    let gg = groups();
    // root owns R_admin directly, and R_read via composite expansion.
    assert!(gg.group_has_role("root", "R_admin", &rg));
    assert!(gg.group_has_role("root", "R_read", &rg));
    // child & grandchild inherit both from the ancestor root.
    assert!(gg.group_has_role("child", "R_admin", &rg));
    assert!(gg.group_has_role("grandchild", "R_read", &rg));
    // a role nobody holds.
    assert!(!gg.group_has_role("grandchild", "R_write", &rg));
}

#[test]
fn has_role_from_group_resolves_through_hierarchy() {
    let rg = roles();
    let gg = groups();
    // grandchild gets R_read transitively (ancestor root -> R_admin -> R_read).
    assert!(gg.has_role_from_group("grandchild", "R_read", false, &rg));
    assert!(gg.has_role_from_group("grandchild", "R_admin", true, &rg));
    // a standalone group with no roles and no qualifying parent -> false either way.
    let mut lone = GroupGraph::new();
    lone.set_parent("solo", "void"); // 'void' has no roles
    assert!(!lone.has_role_from_group("solo", "R_read", false, &rg));
    assert!(!lone.has_role_from_group("solo", "R_read", true, &rg));
}
