// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Directed acyclic graph for the Alloy component controller.
//!
//! Line-ported from grafana/alloy `internal/runtime/internal/dag` (v1.5.0,
//! Apache-2.0): `dag.go` (Graph), `walk.go` (WalkTopological — Kahn's
//! algorithm), and `ops.go` (Validate — cycle + self-reference detection).
//!
//! An [`Edge`] `From → To` means `From` *depends on* `To`. Out-edges are a
//! node's dependencies; in-edges are its dependants. [`Graph::leaves`] are the
//! dependency-free nodes the controller seeds the topological walk from, so a
//! node's dependencies are always evaluated before it.
//!
//! Upstream keys its maps by node pointer identity; this port keys by the
//! node's [`Node::node_id`], which the Alloy controller guarantees is unique.

use std::collections::{BTreeMap, BTreeSet};

/// An individual vertex in the DAG.
pub trait Node {
    /// The unique display name / id of the node.
    fn node_id(&self) -> String;
}

/// A directed connection `from → to` (`from` depends on `to`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    /// The dependant.
    pub from: String,
    /// The dependency.
    pub to: String,
}

/// A directed acyclic graph of [`Node`]s.
#[derive(Default)]
pub struct Graph {
    node_by_id: BTreeMap<String, Box<dyn Node>>,
    out_edges: BTreeMap<String, BTreeSet<String>>, // node → dependencies
    in_edges: BTreeMap<String, BTreeSet<String>>,  // node → dependants
}

impl Graph {
    /// Creates an empty graph.
    pub fn new() -> Graph {
        Graph::default()
    }

    /// Adds `n` into the graph if its id doesn't already exist. Mirrors
    /// `Graph.Add`.
    pub fn add(&mut self, n: Box<dyn Node>) {
        let id = n.node_id();
        self.out_edges.entry(id.clone()).or_default();
        self.in_edges.entry(id.clone()).or_default();
        self.node_by_id.entry(id).or_insert(n);
    }

    /// Adds a directed edge `from → to`. Both endpoints must already be in the
    /// graph. Does not prevent cycles (use [`Graph::validate`]). Mirrors
    /// `Graph.AddEdge`.
    pub fn add_edge(&mut self, from: &str, to: &str) {
        self.out_edges.entry(from.to_string()).or_default().insert(to.to_string());
        self.in_edges.entry(to.to_string()).or_default().insert(from.to_string());
    }

    /// Returns the node with id `id`, if present. Mirrors `Graph.GetByID`.
    pub fn get_by_id(&self, id: &str) -> Option<&dyn Node> {
        self.node_by_id.get(id).map(|b| b.as_ref())
    }

    /// All node ids (sorted for determinism). Mirrors `Graph.Nodes`.
    pub fn node_ids(&self) -> Vec<String> {
        self.node_by_id.keys().cloned().collect()
    }

    /// All edges (sorted for determinism). Mirrors `Graph.Edges`.
    pub fn edges(&self) -> Vec<Edge> {
        let mut out = Vec::new();
        for (from, tos) in &self.out_edges {
            for to in tos {
                out.push(Edge { from: from.clone(), to: to.clone() });
            }
        }
        out
    }

    /// The nodes that `id` depends on (its outgoing edges). Mirrors
    /// `Graph.Dependencies`.
    pub fn dependencies(&self, id: &str) -> Vec<String> {
        self.out_edges.get(id).map(|s| s.iter().cloned().collect()).unwrap_or_default()
    }

    /// The nodes that depend on `id` (its incoming edges). Mirrors
    /// `Graph.Dependants`.
    pub fn dependants(&self, id: &str) -> Vec<String> {
        self.in_edges.get(id).map(|s| s.iter().cloned().collect()).unwrap_or_default()
    }

    /// Nodes with no dependants (no incoming edges). Mirrors `Graph.Roots`.
    pub fn roots(&self) -> Vec<String> {
        self.node_by_id
            .keys()
            .filter(|id| self.in_edges.get(*id).map(|s| s.is_empty()).unwrap_or(true))
            .cloned()
            .collect()
    }

    /// Nodes with no dependencies (no outgoing edges). Mirrors `Graph.Leaves`.
    pub fn leaves(&self) -> Vec<String> {
        self.node_by_id
            .keys()
            .filter(|id| self.out_edges.get(*id).map(|s| s.is_empty()).unwrap_or(true))
            .cloned()
            .collect()
    }

    /// Performs a topological walk of all nodes reachable from `start` in
    /// dependency order: a node is not visited until all of its outgoing edges
    /// (dependencies) have been visited. Implements Kahn's algorithm, leaving
    /// the graph unmodified. Mirrors `dag.WalkTopological`.
    ///
    /// `f_` returning an error stops the walk and propagates the error.
    pub fn walk_topological(
        &self,
        start: &[String],
        f: &mut dyn FnMut(&str),
    ) -> Result<(), String> {
        self.walk_topological_try(start, &mut |id| {
            f(id);
            Ok(())
        })
    }

    /// As [`Graph::walk_topological`] but the visitor may abort with an error.
    pub fn walk_topological_try(
        &self,
        start: &[String],
        f: &mut dyn FnMut(&str) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut visited: BTreeSet<String> = BTreeSet::new();
        let mut unchecked: Vec<String> = start.to_vec();
        let mut remaining_deps: BTreeMap<String, usize> = BTreeMap::new();

        while let Some(check) = unchecked.pop() {
            if visited.contains(&check) {
                continue;
            }
            visited.insert(check.clone());
            f(&check)?;

            // For each dependant of `check`, decrement its remaining-dep count;
            // enqueue once all of its dependencies have been visited.
            if let Some(dependants) = self.in_edges.get(&check) {
                for n in dependants {
                    let entry = remaining_deps
                        .entry(n.clone())
                        .or_insert_with(|| self.out_edges.get(n).map(|s| s.len()).unwrap_or(0));
                    *entry -= 1;
                    if *entry == 0 {
                        unchecked.push(n.clone());
                    }
                }
            }
        }
        Ok(())
    }

    /// Checks that the graph contains no cycles and no self-references.
    /// Mirrors `dag.Validate` (SCC-based cycle detection is equivalent to the
    /// DFS back-edge detection used here for directed graphs).
    pub fn validate(&self) -> Result<(), String> {
        let mut errors: Vec<String> = Vec::new();

        // Self references.
        for e in self.edges() {
            if e.from == e.to {
                errors.push(format!("self reference: {}", e.from));
            }
        }

        // Cycle detection via DFS with a recursion stack.
        let mut color: BTreeMap<String, u8> = BTreeMap::new(); // 0=white,1=gray,2=black
        for id in self.node_by_id.keys() {
            if color.get(id).copied().unwrap_or(0) == 0 {
                let mut stack: Vec<String> = Vec::new();
                if let Some(cycle) = self.dfs_cycle(id, &mut color, &mut stack) {
                    errors.push(format!("cycle: {}", cycle.join(", ")));
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }

    fn dfs_cycle(
        &self,
        node: &str,
        color: &mut BTreeMap<String, u8>,
        stack: &mut Vec<String>,
    ) -> Option<Vec<String>> {
        color.insert(node.to_string(), 1); // gray
        stack.push(node.to_string());
        if let Some(deps) = self.out_edges.get(node) {
            for dep in deps {
                if dep == node {
                    continue; // self-reference reported separately
                }
                match color.get(dep).copied().unwrap_or(0) {
                    0 => {
                        if let Some(c) = self.dfs_cycle(dep, color, stack) {
                            return Some(c);
                        }
                    }
                    1 => {
                        // back-edge → cycle; slice the stack from `dep`.
                        let start = stack.iter().position(|x| x == dep).unwrap_or(0);
                        return Some(stack[start..].to_vec());
                    }
                    _ => {}
                }
            }
        }
        stack.pop();
        color.insert(node.to_string(), 2); // black
        None
    }
}
