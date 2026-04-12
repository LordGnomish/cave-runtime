use crate::types::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceNode {
    pub name: String,
    pub call_count: u64,
    pub error_count: u64,
    pub avg_duration_us: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceEdge {
    pub parent: String,
    pub child: String,
    pub call_count: u64,
    pub error_count: u64,
    pub avg_duration_us: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyGraph {
    pub nodes: Vec<ServiceNode>,
    pub edges: Vec<ServiceEdge>,
}

pub struct DependencyComputer;

impl DependencyComputer {
    /// Compute dependency graph from a set of traces
    pub fn compute(traces: &[Trace]) -> DependencyGraph {
        let mut nodes: HashMap<String, ServiceNode> = HashMap::new();
        let mut edges: HashMap<(String, String), ServiceEdge> = HashMap::new();

        for trace in traces {
            let span_map: HashMap<&str, &Span> =
                trace.spans.iter().map(|s| (s.span_id.as_str(), s)).collect();

            for span in &trace.spans {
                // Update node stats
                let node = nodes
                    .entry(span.service_name.clone())
                    .or_insert_with(|| ServiceNode {
                        name: span.service_name.clone(),
                        call_count: 0,
                        error_count: 0,
                        avg_duration_us: 0.0,
                    });
                node.call_count += 1;
                if span.status == SpanStatus::Error {
                    node.error_count += 1;
                }
                node.avg_duration_us = (node.avg_duration_us * (node.call_count - 1) as f64
                    + span.duration_us as f64)
                    / node.call_count as f64;

                // Update edge
                if let Some(parent_id) = &span.parent_span_id {
                    if let Some(parent_span) = span_map.get(parent_id.as_str()) {
                        if parent_span.service_name != span.service_name {
                            let key = (
                                parent_span.service_name.clone(),
                                span.service_name.clone(),
                            );
                            let edge =
                                edges
                                    .entry(key.clone())
                                    .or_insert_with(|| ServiceEdge {
                                        parent: key.0.clone(),
                                        child: key.1.clone(),
                                        call_count: 0,
                                        error_count: 0,
                                        avg_duration_us: 0.0,
                                    });
                            edge.call_count += 1;
                            if span.status == SpanStatus::Error {
                                edge.error_count += 1;
                            }
                            edge.avg_duration_us = (edge.avg_duration_us
                                * (edge.call_count - 1) as f64
                                + span.duration_us as f64)
                                / edge.call_count as f64;
                        }
                    }
                }
            }
        }

        DependencyGraph {
            nodes: nodes.into_values().collect(),
            edges: edges.into_values().collect(),
        }
    }

    /// Find critical path (longest path by duration) in a trace
    pub fn critical_path(trace: &Trace) -> Vec<&Span> {
        let span_map: HashMap<&str, &Span> =
            trace.spans.iter().map(|s| (s.span_id.as_str(), s)).collect();
        let root = trace
            .spans
            .iter()
            .find(|s| s.parent_span_id.is_none())
            .or_else(|| trace.spans.first());

        if let Some(root) = root {
            Self::find_critical_path_from(root, &span_map)
        } else {
            vec![]
        }
    }

    fn find_critical_path_from<'a>(
        span: &'a Span,
        span_map: &HashMap<&str, &'a Span>,
    ) -> Vec<&'a Span> {
        let children: Vec<&&Span> = span_map
            .values()
            .filter(|s| s.parent_span_id.as_deref() == Some(&span.span_id))
            .collect();

        if children.is_empty() {
            return vec![span];
        }

        let mut longest: Vec<&Span> = vec![];
        let mut longest_dur = 0i64;
        for child in children {
            let path = Self::find_critical_path_from(child, span_map);
            let dur: i64 = path.iter().map(|s| s.duration_us).sum();
            if dur > longest_dur {
                longest_dur = dur;
                longest = path;
            }
        }
        let mut result = vec![span];
        result.extend(longest);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_span(
        trace_id: &str,
        span_id: &str,
        parent: Option<&str>,
        service: &str,
        op: &str,
        duration_us: i64,
    ) -> Span {
        let now = Utc::now();
        Span {
            trace_id: trace_id.to_string(),
            span_id: span_id.to_string(),
            parent_span_id: parent.map(|s| s.to_string()),
            operation_name: op.to_string(),
            service_name: service.to_string(),
            start_time: now,
            end_time: now,
            duration_us,
            status: SpanStatus::Ok,
            kind: SpanKind::Server,
            tags: HashMap::new(),
            events: vec![],
            links: vec![],
            resource_attributes: HashMap::new(),
        }
    }

    #[test]
    fn dependency_graph_two_services() {
        let root = make_span("t1", "s1", None, "frontend", "GET /", 5000);
        let child = make_span("t1", "s2", Some("s1"), "backend", "db.query", 3000);
        let trace = Trace::from_spans(vec![root, child]).unwrap();
        let graph = DependencyComputer::compute(&[trace]);

        let svc_names: Vec<&str> = graph.nodes.iter().map(|n| n.name.as_str()).collect();
        assert!(svc_names.contains(&"frontend"));
        assert!(svc_names.contains(&"backend"));

        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].parent, "frontend");
        assert_eq!(graph.edges[0].child, "backend");
    }

    #[test]
    fn dependency_critical_path() {
        // root (5000us) -> child1 (100us) and child2 (4000us)
        // critical path should be root -> child2
        let root = make_span("t1", "s1", None, "svc", "root", 5000);
        let child1 = make_span("t1", "s2", Some("s1"), "svc", "fast", 100);
        let child2 = make_span("t1", "s3", Some("s1"), "svc", "slow", 4000);
        let trace = Trace::from_spans(vec![root, child1, child2]).unwrap();
        let path = DependencyComputer::critical_path(&trace);

        // Should have root + longest child
        assert_eq!(path.len(), 2);
        assert_eq!(path[0].span_id, "s1");
        assert_eq!(path[1].span_id, "s3");
    }
}
