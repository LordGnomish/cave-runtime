// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::models::{NodeType, Workflow, WorkflowEdge, WorkflowError};
use std::collections::{HashMap, HashSet, VecDeque};

/// Validate a workflow DAG:
/// 1. All edge endpoints reference existing nodes
/// 2. No cycles
/// 3. Exactly one trigger node
pub fn validate_dag(workflow: &Workflow) -> Result<(), WorkflowError> {
    if workflow.nodes.is_empty() {
        return Err(WorkflowError::EmptyWorkflow);
    }
    let node_ids: HashSet<&str> = workflow.nodes.iter().map(|n| n.id.as_str()).collect();
    for edge in &workflow.edges {
        if !node_ids.contains(edge.from_node.as_str()) {
            return Err(WorkflowError::UndefinedNode(edge.from_node.clone()));
        }
        if !node_ids.contains(edge.to_node.as_str()) {
            return Err(WorkflowError::UndefinedNode(edge.to_node.clone()));
        }
    }
    let trigger_count = workflow
        .nodes
        .iter()
        .filter(|n| n.node_type == NodeType::Trigger)
        .count();
    if trigger_count > 1 {
        return Err(WorkflowError::MultipleTriggers);
    }
    if has_cycle(
        &workflow
            .nodes
            .iter()
            .map(|n| n.id.as_str())
            .collect::<Vec<_>>(),
        &workflow.edges,
    ) {
        return Err(WorkflowError::CycleDetected);
    }
    Ok(())
}

/// Detect cycles using DFS
pub fn has_cycle(node_ids: &[&str], edges: &[WorkflowEdge]) -> bool {
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    for id in node_ids {
        adj.insert(id, vec![]);
    }
    for e in edges {
        adj.entry(e.from_node.as_str())
            .or_default()
            .push(e.to_node.as_str());
    }
    let mut visited: HashSet<&str> = HashSet::new();
    let mut rec_stack: HashSet<&str> = HashSet::new();

    fn dfs<'a>(
        node: &'a str,
        adj: &HashMap<&'a str, Vec<&'a str>>,
        visited: &mut HashSet<&'a str>,
        rec_stack: &mut HashSet<&'a str>,
    ) -> bool {
        visited.insert(node);
        rec_stack.insert(node);
        if let Some(neighbors) = adj.get(node) {
            for &next in neighbors {
                if !visited.contains(next) {
                    if dfs(next, adj, visited, rec_stack) {
                        return true;
                    }
                } else if rec_stack.contains(next) {
                    return true;
                }
            }
        }
        rec_stack.remove(node);
        false
    }

    for id in node_ids {
        if !visited.contains(*id) {
            if dfs(id, &adj, &mut visited, &mut rec_stack) {
                return true;
            }
        }
    }
    false
}

/// Topological sort using Kahn's algorithm (BFS)
/// Returns nodes in execution order, or error if cycle detected
pub fn topological_sort(workflow: &Workflow) -> Result<Vec<String>, WorkflowError> {
    let mut in_degree: HashMap<String, usize> = HashMap::new();
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    for node in &workflow.nodes {
        in_degree.insert(node.id.clone(), 0);
        adj.insert(node.id.clone(), vec![]);
    }
    for edge in &workflow.edges {
        *in_degree.entry(edge.to_node.clone()).or_insert(0) += 1;
        adj.entry(edge.from_node.clone())
            .or_default()
            .push(edge.to_node.clone());
    }
    let mut queue: VecDeque<String> = in_degree
        .iter()
        .filter(|&(_, &deg)| deg == 0)
        .map(|(id, _)| id.clone())
        .collect();
    let mut result = vec![];
    while let Some(node) = queue.pop_front() {
        result.push(node.clone());
        if let Some(neighbors) = adj.get(&node) {
            for next in neighbors {
                let deg = in_degree.entry(next.clone()).or_default();
                *deg -= 1;
                if *deg == 0 {
                    queue.push_back(next.clone());
                }
            }
        }
    }
    if result.len() != workflow.nodes.len() {
        return Err(WorkflowError::CycleDetected);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{NodeType, WorkflowNode, WorkflowStatus};
    use chrono::Utc;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn node(id: &str, node_type: NodeType) -> WorkflowNode {
        WorkflowNode {
            id: id.to_string(),
            name: id.to_string(),
            node_type,
            config: HashMap::new(),
        }
    }

    fn edge(from: &str, to: &str) -> WorkflowEdge {
        WorkflowEdge {
            from_node: from.to_string(),
            to_node: to.to_string(),
        }
    }

    fn workflow(nodes: Vec<WorkflowNode>, edges: Vec<WorkflowEdge>) -> Workflow {
        Workflow {
            id: Uuid::new_v4(),
            name: "test".to_string(),
            description: "".to_string(),
            nodes,
            edges,
            created_at: Utc::now(),
            status: WorkflowStatus::Draft,
        }
    }

    #[test]
    fn test_validate_empty_workflow_fails() {
        let wf = workflow(vec![], vec![]);
        assert!(matches!(
            validate_dag(&wf),
            Err(WorkflowError::EmptyWorkflow)
        ));
    }

    #[test]
    fn test_validate_simple_linear() {
        let wf = workflow(
            vec![
                node("a", NodeType::Trigger),
                node("b", NodeType::Action),
                node("c", NodeType::End),
            ],
            vec![edge("a", "b"), edge("b", "c")],
        );
        assert!(validate_dag(&wf).is_ok());
    }

    #[test]
    fn test_validate_cycle_detected() {
        let wf = workflow(
            vec![node("a", NodeType::Action), node("b", NodeType::Action)],
            vec![edge("a", "b"), edge("b", "a")],
        );
        assert!(matches!(
            validate_dag(&wf),
            Err(WorkflowError::CycleDetected)
        ));
    }

    #[test]
    fn test_validate_undefined_node_in_edge() {
        let wf = workflow(
            vec![node("a", NodeType::Trigger)],
            vec![edge("a", "missing")],
        );
        assert!(matches!(
            validate_dag(&wf),
            Err(WorkflowError::UndefinedNode(_))
        ));
    }

    #[test]
    fn test_validate_multiple_triggers_fails() {
        let wf = workflow(
            vec![
                node("t1", NodeType::Trigger),
                node("t2", NodeType::Trigger),
                node("a", NodeType::Action),
            ],
            vec![edge("t1", "a"), edge("t2", "a")],
        );
        assert!(matches!(
            validate_dag(&wf),
            Err(WorkflowError::MultipleTriggers)
        ));
    }

    #[test]
    fn test_has_cycle_linear() {
        let ids = vec!["a", "b", "c"];
        let edges = vec![edge("a", "b"), edge("b", "c")];
        assert!(!has_cycle(&ids, &edges));
    }

    #[test]
    fn test_has_cycle_with_cycle() {
        let ids = vec!["a", "b", "c"];
        let edges = vec![edge("a", "b"), edge("b", "c"), edge("c", "a")];
        assert!(has_cycle(&ids, &edges));
    }

    #[test]
    fn test_has_cycle_single_node() {
        let ids = vec!["a"];
        let edges: Vec<WorkflowEdge> = vec![];
        assert!(!has_cycle(&ids, &edges));
    }

    #[test]
    fn test_topological_sort_linear() {
        let wf = workflow(
            vec![
                node("a", NodeType::Trigger),
                node("b", NodeType::Action),
                node("c", NodeType::End),
            ],
            vec![edge("a", "b"), edge("b", "c")],
        );
        let result = topological_sort(&wf).unwrap();
        assert_eq!(result.len(), 3);
        let pos_a = result.iter().position(|x| x == "a").unwrap();
        let pos_b = result.iter().position(|x| x == "b").unwrap();
        let pos_c = result.iter().position(|x| x == "c").unwrap();
        assert!(pos_a < pos_b);
        assert!(pos_b < pos_c);
    }

    #[test]
    fn test_topological_sort_diamond() {
        // A → B, A → C, B → D, C → D: D must come last
        let wf = workflow(
            vec![
                node("a", NodeType::Trigger),
                node("b", NodeType::Action),
                node("c", NodeType::Action),
                node("d", NodeType::End),
            ],
            vec![
                edge("a", "b"),
                edge("a", "c"),
                edge("b", "d"),
                edge("c", "d"),
            ],
        );
        let result = topological_sort(&wf).unwrap();
        assert_eq!(result.len(), 4);
        let pos_a = result.iter().position(|x| x == "a").unwrap();
        let pos_b = result.iter().position(|x| x == "b").unwrap();
        let pos_c = result.iter().position(|x| x == "c").unwrap();
        let pos_d = result.iter().position(|x| x == "d").unwrap();
        assert!(pos_a < pos_b);
        assert!(pos_a < pos_c);
        assert!(pos_b < pos_d);
        assert!(pos_c < pos_d);
    }

    #[test]
    fn test_topological_sort_cycle_fails() {
        let wf = workflow(
            vec![node("a", NodeType::Action), node("b", NodeType::Action)],
            vec![edge("a", "b"), edge("b", "a")],
        );
        assert!(matches!(
            topological_sort(&wf),
            Err(WorkflowError::CycleDetected)
        ));
    }
}
