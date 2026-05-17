// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Service dependency graph.
//!
//! Builds a directed graph of service → service calls from span parent/child
//! relationships across all ingested traces, suitable for the Jaeger
//! /api/dependencies endpoint.

use std::collections::{HashMap, HashSet};
use serde::{Deserialize, Serialize};

use crate::types::{Span, ServiceDependency, Trace};

// ─── Graph types ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyGraph {
    pub nodes: Vec<ServiceNode>,
    pub edges: Vec<DependencyEdge>,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceNode {
    pub name: String,
    pub call_count: u64,
    pub error_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DependencyEdge {
    pub parent: String,
    pub child: String,
    pub call_count: u64,
    pub error_count: u64,
}

// ─── Jaeger /api/dependencies wire format ─────────────────────────────────

/// Jaeger's API returns a flat array of `{parent, child, callCount}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JaegerDependency {
    pub parent: String,
    pub child: String,
    pub call_count: u64,
}

// ─── Build graph ───────────────────────────────────────────────────────────

/// Build a service dependency graph from a collection of traces.
pub fn build_dependency_graph(traces: &[Trace]) -> DependencyGraph {
    let mut edge_map: HashMap<(String, String), (u64, u64)> = HashMap::new(); // (calls, errors)
    let mut node_map: HashMap<String, (u64, u64)> = HashMap::new(); // (calls, errors)

    for trace in traces {
        let span_map: HashMap<u64, &Span> =
            trace.spans.iter().map(|s| (s.span_id, s)).collect();

        for span in &trace.spans {
            // Count this span for its service
            let e = node_map.entry(span.service_name.clone()).or_insert((0, 0));
            e.0 += 1;
            if span.has_error() { e.1 += 1; }

            // Build edge: parent_service → this_service
            if let Some(parent_id) = span.parent_span_id {
                if let Some(parent) = span_map.get(&parent_id) {
                    if parent.service_name != span.service_name {
                        let key = (parent.service_name.clone(), span.service_name.clone());
                        let edge = edge_map.entry(key).or_insert((0, 0));
                        edge.0 += 1;
                        if span.has_error() { edge.1 += 1; }
                    }
                }
            }
        }
    }

    let now = chrono::Utc::now().to_rfc3339();

    let nodes = node_map
        .into_iter()
        .map(|(name, (calls, errors))| ServiceNode { name, call_count: calls, error_count: errors })
        .collect();

    let edges = edge_map
        .into_iter()
        .map(|((parent, child), (calls, errors))| DependencyEdge {
            parent,
            child,
            call_count: calls,
            error_count: errors,
        })
        .collect();

    DependencyGraph { nodes, edges, timestamp: now }
}

/// Convert to Jaeger /api/dependencies format.
pub fn to_jaeger_dependencies(graph: &DependencyGraph) -> Vec<JaegerDependency> {
    graph
        .edges
        .iter()
        .map(|e| JaegerDependency {
            parent: e.parent.clone(),
            child: e.child.clone(),
            call_count: e.call_count,
        })
        .collect()
}

// ─── Reachability / transitive closure ────────────────────────────────────

/// Find all services reachable (directly or transitively) from `service`.
pub fn reachable_from(graph: &DependencyGraph, service: &str) -> HashSet<String> {
    let mut visited = HashSet::new();
    let mut queue = vec![service.to_owned()];

    while let Some(svc) = queue.pop() {
        if !visited.insert(svc.clone()) { continue; }
        for edge in &graph.edges {
            if edge.parent == svc && !visited.contains(&edge.child) {
                queue.push(edge.child.clone());
            }
        }
    }

    visited.remove(service);
    visited
}

/// Find all services that call (directly or transitively) into `service`.
pub fn callers_of(graph: &DependencyGraph, service: &str) -> HashSet<String> {
    let mut visited = HashSet::new();
    let mut queue = vec![service.to_owned()];

    while let Some(svc) = queue.pop() {
        if !visited.insert(svc.clone()) { continue; }
        for edge in &graph.edges {
            if edge.child == svc && !visited.contains(&edge.parent) {
                queue.push(edge.parent.clone());
            }
        }
    }

    visited.remove(service);
    visited
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use std::collections::HashMap;

    fn make_trace(spans: Vec<Span>) -> Trace {
        Trace::from_spans(spans).unwrap()
    }

    fn span_with_parent(
        trace_id: TraceId, span_id: SpanId, parent_id: Option<SpanId>,
        svc: &str, op: &str, error: bool,
    ) -> Span {
        Span {
            trace_id,
            span_id,
            parent_span_id: parent_id,
            operation_name: op.into(),
            service_name: svc.into(),
            start_time_unix_nano: 1_000_000_000,
            end_time_unix_nano:   1_005_000_000,
            duration_ns: 5_000_000,
            status: if error { SpanStatus::Error } else { SpanStatus::Ok },
            kind: SpanKind::Server,
            tags: HashMap::new(),
            events: vec![],
            links: vec![],
            resource_attributes: HashMap::new(),
            tenant_id: "default".into(),
            baggage: HashMap::new(),
            log_labels: HashMap::new(),
        }
    }

    #[test]
    fn build_graph_single_edge() {
        let trace = make_trace(vec![
            span_with_parent(1, 1, None,    "frontend", "GET /", false),
            span_with_parent(1, 2, Some(1), "backend",  "query", false),
        ]);
        let graph = build_dependency_graph(&[trace]);
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].parent, "frontend");
        assert_eq!(graph.edges[0].child, "backend");
        assert_eq!(graph.edges[0].call_count, 1);
    }

    #[test]
    fn build_graph_same_service_spans_no_edge() {
        // Two spans of the same service do not create an edge
        let trace = make_trace(vec![
            span_with_parent(1, 1, None,    "svc", "op1", false),
            span_with_parent(1, 2, Some(1), "svc", "op2", false),
        ]);
        let graph = build_dependency_graph(&[trace]);
        assert_eq!(graph.edges.len(), 0);
    }

    #[test]
    fn reachable_from_test() {
        let graph = DependencyGraph {
            nodes: vec![],
            edges: vec![
                DependencyEdge { parent: "a".into(), child: "b".into(), call_count: 1, error_count: 0 },
                DependencyEdge { parent: "b".into(), child: "c".into(), call_count: 1, error_count: 0 },
            ],
            timestamp: String::new(),
        };
        let r = reachable_from(&graph, "a");
        assert!(r.contains("b"));
        assert!(r.contains("c"));
        assert!(!r.contains("a"));
    }

    #[test]
    fn callers_of_test() {
        let graph = DependencyGraph {
            nodes: vec![],
            edges: vec![
                DependencyEdge { parent: "a".into(), child: "b".into(), call_count: 1, error_count: 0 },
                DependencyEdge { parent: "b".into(), child: "c".into(), call_count: 1, error_count: 0 },
            ],
            timestamp: String::new(),
        };
        let callers = callers_of(&graph, "c");
        assert!(callers.contains("a"));
        assert!(callers.contains("b"));
    }

    #[test]
    fn jaeger_dependencies_format() {
        let graph = DependencyGraph {
            nodes: vec![],
            edges: vec![DependencyEdge {
                parent: "a".into(), child: "b".into(), call_count: 42, error_count: 0,
            }],
            timestamp: String::new(),
        };
        let jdeps = to_jaeger_dependencies(&graph);
        assert_eq!(jdeps[0].call_count, 42);
    }
}
