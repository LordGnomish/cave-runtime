// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD line-port of Keycloak composite-role resolution.
// Upstream (Apache-2.0):
//   server-spi/src/main/java/org/keycloak/models/utils/RoleUtils.java
//     - expandCompositeRoles(Set), expandCompositeRolesStream(role, visited), hasRole(Set, target)
//   server-spi-private/src/main/java/org/keycloak/models/utils/KeycloakModelUtils.java
//     - searchFor(role, composite, visited)   (== RoleModel.hasRole semantics)
//
// These are pure graph algorithms with explicit cycle detection. The cases below
// pin the upstream semantics exactly: deep (transitive) membership, diamond shapes,
// and self/mutual cycles that must terminate.

use cave_auth::role_utils::RoleGraph;
use std::collections::HashSet;

/// Build a graph: A -> {B, C}, B -> {D}.  (A,B composite; C,D leaf)
fn sample() -> RoleGraph {
    let mut g = RoleGraph::new();
    g.add_composite("A", "B");
    g.add_composite("A", "C");
    g.add_composite("B", "D");
    g
}

#[test]
fn is_composite_reflects_having_children() {
    let g = sample();
    assert!(g.is_composite("A"));
    assert!(g.is_composite("B"));
    assert!(!g.is_composite("C"));
    assert!(!g.is_composite("D"));
    // Unknown role is not composite.
    assert!(!g.is_composite("ZZZ"));
}

#[test]
fn role_has_role_direct_and_transitive() {
    let g = sample();
    // RoleModel.hasRole: this == target || searchFor(...)
    assert!(g.role_has_role("A", "A")); // identity
    assert!(g.role_has_role("A", "B")); // direct composite
    assert!(g.role_has_role("A", "C")); // direct composite
    assert!(g.role_has_role("A", "D")); // transitive via B
    assert!(g.role_has_role("B", "D")); // direct
    // Negatives: leaves contain nothing; no upward edges.
    assert!(!g.role_has_role("D", "A"));
    assert!(!g.role_has_role("C", "A"));
    assert!(!g.role_has_role("B", "C")); // siblings are not related
    assert!(!g.role_has_role("A", "UNKNOWN"));
}

#[test]
fn expand_composite_roles_collects_transitive_closure() {
    let g = sample();
    let got = g.expand_composite_roles(&["A".to_string()]);
    let want: HashSet<String> = ["A", "B", "C", "D"].iter().map(|s| s.to_string()).collect();
    assert_eq!(got, want);

    // Expanding a leaf yields just itself.
    let leaf = g.expand_composite_roles(&["D".to_string()]);
    assert_eq!(leaf, ["D".to_string()].into_iter().collect());

    // Multi-root with a shared sub-tree still yields the union, deduplicated.
    let multi = g.expand_composite_roles(&["B".to_string(), "C".to_string()]);
    let want_multi: HashSet<String> =
        ["B", "C", "D"].iter().map(|s| s.to_string()).collect();
    assert_eq!(multi, want_multi);
}

#[test]
fn diamond_shape_is_visited_once() {
    // A -> B, A -> C, B -> D, C -> D  (D reachable two ways)
    let mut g = RoleGraph::new();
    g.add_composite("A", "B");
    g.add_composite("A", "C");
    g.add_composite("B", "D");
    g.add_composite("C", "D");
    let got = g.expand_composite_roles(&["A".to_string()]);
    let want: HashSet<String> = ["A", "B", "C", "D"].iter().map(|s| s.to_string()).collect();
    assert_eq!(got, want);
    assert!(g.role_has_role("A", "D"));
}

#[test]
fn self_cycle_terminates() {
    // A -> A : self-referential composite must not loop forever.
    let mut g = RoleGraph::new();
    g.add_composite("A", "A");
    assert!(g.role_has_role("A", "A"));
    let got = g.expand_composite_roles(&["A".to_string()]);
    assert_eq!(got, ["A".to_string()].into_iter().collect());
}

#[test]
fn mutual_cycle_terminates_and_resolves() {
    // A -> B, B -> A : mutual cycle.
    let mut g = RoleGraph::new();
    g.add_composite("A", "B");
    g.add_composite("B", "A");
    // Both reach each other; both reach themselves via the cycle (or identity).
    assert!(g.role_has_role("A", "B"));
    assert!(g.role_has_role("B", "A"));
    assert!(g.role_has_role("A", "A"));
    // A query for a role that is NOT in the cycle must terminate and be false.
    assert!(!g.role_has_role("A", "GHOST"));
    let got = g.expand_composite_roles(&["A".to_string()]);
    let want: HashSet<String> = ["A", "B"].iter().map(|s| s.to_string()).collect();
    assert_eq!(got, want);
}

#[test]
fn set_has_role_matches_roleutils_semantics() {
    let g = sample();
    // RoleUtils.hasRole(Set, target): roles.contains(target) || any mapping.hasRole(target)
    let roles: HashSet<String> = ["A".to_string()].into_iter().collect();
    assert!(g.set_has_role(&roles, "A")); // contains
    assert!(g.set_has_role(&roles, "D")); // via A's composite closure
    assert!(!g.set_has_role(&roles, "GHOST"));

    let empty: HashSet<String> = HashSet::new();
    assert!(!g.set_has_role(&empty, "A"));
}
