// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD port of grafana/alloy `internal/runtime/internal/dag` (v1.5.0):
//! `dag.go` (Graph) + `walk.go` (WalkTopological) + `ops.go` (Validate).
//!
//! Edge `From→To` means `From` depends on `To`. Roots have no dependants
//! (no incoming edges); leaves have no dependencies (no outgoing edges). A
//! topological walk seeded from the leaves visits dependencies before
//! dependants — exactly how the Alloy controller (loader.go) evaluates the
//! component graph.

use cave_tracing::alloy::graph::{Graph, Node};

/// Minimal `Node` for tests: identity is the id string.
struct N(&'static str);
impl Node for N {
    fn node_id(&self) -> String {
        self.0.to_string()
    }
}

/// Builds A→B→C (A depends on B depends on C).
fn chain() -> Graph {
    let mut g = Graph::new();
    g.add(Box::new(N("A")));
    g.add(Box::new(N("B")));
    g.add(Box::new(N("C")));
    g.add_edge("A", "B");
    g.add_edge("B", "C");
    g
}

#[test]
fn add_nodes_edges_and_lookup() {
    let g = chain();
    assert_eq!(g.node_ids().len(), 3);
    assert!(g.get_by_id("B").is_some());
    assert!(g.get_by_id("Z").is_none());
    assert_eq!(g.get_by_id("B").unwrap().node_id(), "B");
}

#[test]
fn dependencies_and_dependants() {
    let g = chain();
    // A depends on B.
    assert_eq!(g.dependencies("A"), vec!["B".to_string()]);
    // B is depended on by A.
    assert_eq!(g.dependants("B"), vec!["A".to_string()]);
    // C has no dependencies; B depends on C.
    assert!(g.dependencies("C").is_empty());
    assert_eq!(g.dependencies("B"), vec!["C".to_string()]);
}

#[test]
fn roots_and_leaves() {
    let g = chain();
    // Root = no dependants (no incoming edges) = A.
    assert_eq!(g.roots(), vec!["A".to_string()]);
    // Leaf = no dependencies (no outgoing edges) = C.
    assert_eq!(g.leaves(), vec!["C".to_string()]);
}

#[test]
fn walk_topological_visits_dependencies_first() {
    let g = chain();
    let mut order = Vec::new();
    g.walk_topological(&g.leaves(), &mut |id: &str| order.push(id.to_string()))
        .unwrap();
    // C (no deps) must come before B, which must come before A.
    let pos = |x: &str| order.iter().position(|y| y == x).unwrap();
    assert_eq!(order.len(), 3);
    assert!(pos("C") < pos("B"));
    assert!(pos("B") < pos("A"));
}

#[test]
fn diamond_topological_order() {
    // D depends on B and C; B and C both depend on A.
    //   B → A,  C → A,  D → B,  D → C
    let mut g = Graph::new();
    for id in ["A", "B", "C", "D"] {
        g.add(Box::new(N(Box::leak(id.to_string().into_boxed_str()))));
    }
    g.add_edge("B", "A");
    g.add_edge("C", "A");
    g.add_edge("D", "B");
    g.add_edge("D", "C");

    let mut order = Vec::new();
    g.walk_topological(&g.leaves(), &mut |id: &str| order.push(id.to_string()))
        .unwrap();
    let pos = |x: &str| order.iter().position(|y| y == x).unwrap();
    assert_eq!(order.len(), 4);
    assert!(pos("A") < pos("B"));
    assert!(pos("A") < pos("C"));
    assert!(pos("B") < pos("D"));
    assert!(pos("C") < pos("D"));
}

#[test]
fn validate_accepts_acyclic_graph() {
    assert!(chain().validate().is_ok());
}

#[test]
fn validate_detects_cycle() {
    let mut g = Graph::new();
    g.add(Box::new(N("A")));
    g.add(Box::new(N("B")));
    g.add_edge("A", "B");
    g.add_edge("B", "A");
    let err = g.validate().unwrap_err();
    assert!(err.to_lowercase().contains("cycle"), "err was: {err}");
}

#[test]
fn validate_detects_self_reference() {
    let mut g = Graph::new();
    g.add(Box::new(N("A")));
    g.add_edge("A", "A");
    let err = g.validate().unwrap_err();
    assert!(
        err.to_lowercase().contains("self reference") || err.to_lowercase().contains("cycle"),
        "err was: {err}"
    );
}
