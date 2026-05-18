// SPDX-License-Identifier: AGPL-3.0-or-later
//! Resource dependency graph — topological sort for apply ordering.

use crate::error::{InfraError, InfraResult};
use crate::resource::ResourceSpec;
use petgraph::algo::toposort;
use petgraph::graph::{DiGraph, NodeIndex};
use std::collections::HashMap;

/// Build a directed graph from resource specs and return them in apply order.
/// Edges represent "depends on": A → B means A depends on B (B must be created first).
pub fn apply_order(specs: &[ResourceSpec]) -> InfraResult<Vec<&ResourceSpec>> {
    let mut graph: DiGraph<usize, ()> = DiGraph::new();
    let mut name_to_idx: HashMap<&str, NodeIndex> = HashMap::new();
    let mut idx_to_spec: HashMap<NodeIndex, usize> = HashMap::new();

    // Add nodes
    for (i, spec) in specs.iter().enumerate() {
        let node = graph.add_node(i);
        name_to_idx.insert(&spec.name, node);
        idx_to_spec.insert(node, i);
    }

    // Add edges
    for spec in specs {
        let from = name_to_idx[spec.name.as_str()];
        for dep_name in &spec.depends_on {
            let to = name_to_idx.get(dep_name.as_str()).ok_or_else(|| {
                InfraError::DependencyNotMet {
                    resource: spec.name.clone(),
                    depends_on: dep_name.clone(),
                }
            })?;
            // Edge from spec → dep (spec depends on dep, dep must come first)
            graph.add_edge(*to, from, ()); // dep → spec in apply order
        }
    }

    // Topological sort
    let sorted = toposort(&graph, None).map_err(|cycle| {
        let node_idx = cycle.node_id();
        let spec_idx = idx_to_spec[&node_idx];
        InfraError::DependencyCycle(specs[spec_idx].name.clone())
    })?;

    Ok(sorted.into_iter().map(|n| &specs[idx_to_spec[&n]]).collect())
}

/// Return the destroy order (reverse of apply order).
pub fn destroy_order(specs: &[ResourceSpec]) -> InfraResult<Vec<&ResourceSpec>> {
    let mut ordered = apply_order(specs)?;
    ordered.reverse();
    Ok(ordered)
}

/// Validate that all declared dependencies exist in the spec list.
pub fn validate_dependencies(specs: &[ResourceSpec]) -> InfraResult<()> {
    let names: std::collections::HashSet<&str> = specs.iter().map(|s| s.name.as_str()).collect();
    for spec in specs {
        for dep in &spec.depends_on {
            if !names.contains(dep.as_str()) {
                return Err(InfraError::DependencyNotMet {
                    resource: spec.name.clone(),
                    depends_on: dep.clone(),
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::ResourceKind;
    use std::collections::HashMap;

    fn spec(name: &str, depends_on: Vec<&str>) -> ResourceSpec {
        ResourceSpec {
            kind: ResourceKind::Server,
            name: name.to_string(),
            provider: "noop".into(),
            properties: HashMap::new(),
            depends_on: depends_on.into_iter().map(|s| s.to_string()).collect(),
            tags: HashMap::new(),
        }
    }

    #[test]
    fn simple_ordering() {
        let specs = vec![
            spec("web", vec!["db", "network"]),
            spec("db", vec!["network"]),
            spec("network", vec![]),
        ];
        let order = apply_order(&specs).unwrap();
        let names: Vec<&str> = order.iter().map(|s| s.name.as_str()).collect();
        // network must come before db, db before web
        assert!(names.iter().position(|&n| n == "network") < names.iter().position(|&n| n == "db"));
        assert!(names.iter().position(|&n| n == "db") < names.iter().position(|&n| n == "web"));
    }

    #[test]
    fn no_dependencies_any_order() {
        let specs = vec![spec("a", vec![]), spec("b", vec![]), spec("c", vec![])];
        let order = apply_order(&specs).unwrap();
        assert_eq!(order.len(), 3);
    }

    #[test]
    fn cycle_detected() {
        let specs = vec![
            spec("a", vec!["b"]),
            spec("b", vec!["a"]),
        ];
        assert!(matches!(apply_order(&specs), Err(InfraError::DependencyCycle(_))));
    }

    #[test]
    fn missing_dependency_fails() {
        let specs = vec![spec("web", vec!["missing-resource"])];
        assert!(matches!(apply_order(&specs), Err(InfraError::DependencyNotMet { .. })));
    }

    #[test]
    fn destroy_is_reverse_of_apply() {
        let specs = vec![
            spec("lb", vec!["web"]),
            spec("web", vec!["db"]),
            spec("db", vec![]),
        ];
        let apply = apply_order(&specs).unwrap();
        let destroy = destroy_order(&specs).unwrap();
        let apply_names: Vec<&str> = apply.iter().map(|s| s.name.as_str()).collect();
        let destroy_names: Vec<&str> = destroy.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            destroy_names,
            apply_names.iter().copied().rev().collect::<Vec<_>>()
        );
    }
}
