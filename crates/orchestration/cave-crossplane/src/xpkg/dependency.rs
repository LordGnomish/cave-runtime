// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Dependency resolver — DAG over Configuration → Provider/Function deps,
//! cycle detection, topo-ordered install plan.
//!
//! Upstream: internal/xpkg/dep/manager.go + internal/dag/dag.go

use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ResolveError {
    #[error("cycle detected involving: {0:?}")]
    Cycle(Vec<String>),
    #[error("unknown node: {0}")]
    UnknownNode(String),
}

#[derive(Debug, Clone, Default)]
pub struct DependencyGraph {
    nodes: BTreeSet<String>,
    /// Adjacency: edges[from] = {to₁, to₂, …} meaning `from` depends on `to`
    /// (must be installed before `from`).
    edges: BTreeMap<String, BTreeSet<String>>,
}

impl DependencyGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_node(&mut self, name: impl Into<String>) {
        let name = name.into();
        self.nodes.insert(name.clone());
        self.edges.entry(name).or_default();
    }

    pub fn add_edge(&mut self, from: impl Into<String>, to: impl Into<String>) -> Result<(), ResolveError> {
        let from = from.into();
        let to = to.into();
        if !self.nodes.contains(&from) {
            return Err(ResolveError::UnknownNode(from));
        }
        if !self.nodes.contains(&to) {
            return Err(ResolveError::UnknownNode(to));
        }
        self.edges.entry(from).or_default().insert(to);
        Ok(())
    }

    pub fn nodes(&self) -> Vec<String> {
        self.nodes.iter().cloned().collect()
    }

    /// Return the install order: dependencies first.
    pub fn topo_sort(&self) -> Result<Vec<String>, ResolveError> {
        let mut in_degree: BTreeMap<String, usize> = self
            .nodes
            .iter()
            .map(|n| (n.clone(), 0))
            .collect();
        for (from, deps) in &self.edges {
            for to in deps {
                // Edge from→to means `from` depends on `to`; so `to` must be
                // installed first → install order has fewer in-degrees on `to`.
                // We invert: in-degree counts how many dependents each node has.
                *in_degree.entry(from.clone()).or_default() += 0;
                *in_degree.entry(to.clone()).or_default() += 0;
            }
        }
        // Build reverse mapping: who depends on `to` ?
        let mut reverse: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (from, deps) in &self.edges {
            for to in deps {
                reverse.entry(to.clone()).or_default().insert(from.clone());
                *in_degree.entry(from.clone()).or_default() += 1;
            }
        }
        // Kahn's algorithm starting from nodes with in_degree == 0
        let mut ready: Vec<String> = in_degree
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(n, _)| n.clone())
            .collect();
        ready.sort();
        let mut order: Vec<String> = Vec::new();
        while let Some(n) = ready.pop() {
            order.push(n.clone());
            if let Some(dependents) = reverse.get(&n) {
                for d in dependents {
                    let entry = in_degree.entry(d.clone()).or_default();
                    if *entry > 0 {
                        *entry -= 1;
                        if *entry == 0 {
                            ready.push(d.clone());
                            ready.sort();
                        }
                    }
                }
            }
        }
        if order.len() != self.nodes.len() {
            let cycle: Vec<String> = in_degree
                .iter()
                .filter(|(_, d)| **d > 0)
                .map(|(n, _)| n.clone())
                .collect();
            return Err(ResolveError::Cycle(cycle));
        }
        Ok(order)
    }

    /// Detect cycle without computing full order.
    pub fn has_cycle(&self) -> bool {
        matches!(self.topo_sort(), Err(ResolveError::Cycle(_)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_graph_empty_order() {
        let g = DependencyGraph::new();
        assert!(g.topo_sort().unwrap().is_empty());
    }

    #[test]
    fn single_node() {
        let mut g = DependencyGraph::new();
        g.add_node("a");
        assert_eq!(g.topo_sort().unwrap(), vec!["a".to_string()]);
    }

    #[test]
    fn linear_dep_chain() {
        let mut g = DependencyGraph::new();
        g.add_node("a");
        g.add_node("b");
        g.add_node("c");
        // a depends on b, b depends on c → order is c, b, a
        g.add_edge("a", "b").unwrap();
        g.add_edge("b", "c").unwrap();
        let order = g.topo_sort().unwrap();
        assert_eq!(
            order.iter().position(|n| n == "c").unwrap()
                < order.iter().position(|n| n == "b").unwrap()
        , true);
        assert_eq!(
            order.iter().position(|n| n == "b").unwrap()
                < order.iter().position(|n| n == "a").unwrap()
        , true);
    }

    #[test]
    fn diamond_dependencies() {
        let mut g = DependencyGraph::new();
        for n in ["a", "b", "c", "d"] {
            g.add_node(n);
        }
        // a → b, a → c, b → d, c → d
        g.add_edge("a", "b").unwrap();
        g.add_edge("a", "c").unwrap();
        g.add_edge("b", "d").unwrap();
        g.add_edge("c", "d").unwrap();
        let order = g.topo_sort().unwrap();
        let pos = |n: &str| order.iter().position(|x| x == n).unwrap();
        assert!(pos("d") < pos("b"));
        assert!(pos("d") < pos("c"));
        assert!(pos("b") < pos("a"));
        assert!(pos("c") < pos("a"));
    }

    #[test]
    fn cycle_detected() {
        let mut g = DependencyGraph::new();
        g.add_node("a");
        g.add_node("b");
        g.add_edge("a", "b").unwrap();
        g.add_edge("b", "a").unwrap();
        assert!(g.has_cycle());
        assert!(matches!(g.topo_sort(), Err(ResolveError::Cycle(_))));
    }

    #[test]
    fn unknown_node_edge_errors() {
        let mut g = DependencyGraph::new();
        g.add_node("a");
        assert!(g.add_edge("a", "missing").is_err());
        assert!(g.add_edge("nope", "a").is_err());
    }

    #[test]
    fn nodes_returns_all() {
        let mut g = DependencyGraph::new();
        g.add_node("x");
        g.add_node("y");
        assert_eq!(g.nodes().len(), 2);
    }

    #[test]
    fn self_loop_is_cycle() {
        let mut g = DependencyGraph::new();
        g.add_node("a");
        g.add_edge("a", "a").unwrap();
        assert!(g.has_cycle());
    }
}
