use crate::models::{AnomalousSpan, Span, SpanStatus, Trace, TraceNode};
use crate::TraceStore;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

// ── Ingestion ─────────────────────────────────────────────────────────────────

/// Push a new span into the store.
pub fn ingest_span(store: &mut TraceStore, span: Span) {
    store.spans.push(span);
}

// ── Trace assembly ────────────────────────────────────────────────────────────

/// Assemble a `Trace` from all spans that share `trace_id`.
/// Returns `None` if no matching spans exist.
pub fn build_trace(trace_id: Uuid, all_spans: &[Span]) -> Option<Trace> {
    let spans: Vec<Span> = all_spans
        .iter()
        .filter(|s| s.trace_id == trace_id)
        .cloned()
        .collect();

    if spans.is_empty() {
        return None;
    }

    let service_count = spans
        .iter()
        .map(|s| s.service.as_str())
        .collect::<HashSet<_>>()
        .len();

    let start_time = spans.iter().map(|s| s.start_time).min();

    let total_duration_ms = spans
        .iter()
        .filter(|s| s.parent_span_id.is_none())
        .map(|s| s.duration_ms)
        .sum::<f64>();

    let has_error = spans.iter().any(|s| s.status == SpanStatus::Error);

    let root_spans: Vec<Span> = spans
        .iter()
        .filter(|s| s.parent_span_id.is_none())
        .cloned()
        .collect();

    Some(Trace {
        trace_id,
        span_count: spans.len(),
        service_count,
        root_spans,
        all_spans: spans,
        start_time,
        total_duration_ms,
        status: if has_error {
            SpanStatus::Error
        } else {
            SpanStatus::Ok
        },
    })
}

// ── Tree building ─────────────────────────────────────────────────────────────

/// Build a parent-child tree from a flat slice of spans.
/// Spans whose parent is not in the slice become roots.
pub fn build_trace_tree(spans: &[Span]) -> Vec<TraceNode> {
    let all_ids: HashSet<Uuid> = spans.iter().map(|s| s.span_id).collect();

    let span_map: HashMap<Uuid, Span> =
        spans.iter().map(|s| (s.span_id, s.clone())).collect();

    let mut children_map: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    for span in spans {
        if let Some(pid) = span.parent_span_id {
            if all_ids.contains(&pid) {
                children_map.entry(pid).or_default().push(span.span_id);
            }
        }
    }

    let root_ids: Vec<Uuid> = spans
        .iter()
        .filter(|s| {
            s.parent_span_id
                .map(|pid| !all_ids.contains(&pid))
                .unwrap_or(true)
        })
        .map(|s| s.span_id)
        .collect();

    root_ids
        .iter()
        .map(|&rid| build_node(rid, &span_map, &children_map))
        .collect()
}

fn build_node(
    span_id: Uuid,
    span_map: &HashMap<Uuid, Span>,
    children_map: &HashMap<Uuid, Vec<Uuid>>,
) -> TraceNode {
    let span = span_map[&span_id].clone();
    let children = children_map
        .get(&span_id)
        .map(|ids| {
            ids.iter()
                .map(|&cid| build_node(cid, span_map, children_map))
                .collect()
        })
        .unwrap_or_default();
    TraceNode { span, children }
}

// ── Critical path ─────────────────────────────────────────────────────────────

/// Return the ordered list of span IDs forming the longest-duration path
/// through the trace tree (the critical path that drives overall latency).
pub fn calculate_critical_path(nodes: &[TraceNode]) -> Vec<Uuid> {
    nodes
        .iter()
        .map(critical_path_node)
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, path)| path)
        .unwrap_or_default()
}

fn critical_path_node(node: &TraceNode) -> (f64, Vec<Uuid>) {
    if node.children.is_empty() {
        return (node.span.duration_ms, vec![node.span.span_id]);
    }
    let (child_duration, mut child_path) = node
        .children
        .iter()
        .map(critical_path_node)
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap();
    child_path.insert(0, node.span.span_id);
    (node.span.duration_ms + child_duration, child_path)
}

// ── Anomaly detection ─────────────────────────────────────────────────────────

/// Identify spans that are either in error state or whose duration is more than
/// two standard deviations above the mean for the slice.
pub fn detect_anomalous_spans(spans: &[Span]) -> Vec<AnomalousSpan> {
    if spans.is_empty() {
        return Vec::new();
    }

    let durations: Vec<f64> = spans.iter().map(|s| s.duration_ms).collect();
    let mean = durations.iter().sum::<f64>() / durations.len() as f64;
    let variance = durations
        .iter()
        .map(|d| (d - mean).powi(2))
        .sum::<f64>()
        / durations.len() as f64;
    let stddev = variance.sqrt();

    spans
        .iter()
        .filter_map(|span| {
            let is_error = span.status == SpanStatus::Error;
            let z_score = if stddev > 0.0 {
                (span.duration_ms - mean) / stddev
            } else {
                0.0
            };
            let is_slow = z_score > 2.0;

            if !is_error && !is_slow {
                return None;
            }

            let reason = match (is_error, is_slow) {
                (true, true) => format!(
                    "Error status and slow duration ({:.1}ms, {:.1}σ above mean)",
                    span.duration_ms, z_score
                ),
                (true, false) => "Error status".to_string(),
                _ => format!(
                    "Slow duration ({:.1}ms, {:.1}σ above mean)",
                    span.duration_ms, z_score
                ),
            };

            Some(AnomalousSpan {
                span: span.clone(),
                reason,
                severity: if is_error { z_score.max(3.0) } else { z_score },
            })
        })
        .collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::collections::HashMap;

    fn make_span(trace_id: Uuid, span_id: Uuid, parent: Option<Uuid>, dur: f64) -> Span {
        Span {
            trace_id,
            span_id,
            parent_span_id: parent,
            operation: "op".into(),
            service: "svc".into(),
            start_time: Utc::now(),
            duration_ms: dur,
            status: SpanStatus::Ok,
            tags: HashMap::new(),
            events: Vec::new(),
            links: Vec::new(),
        }
    }

    #[test]
    fn build_trace_tree_single_root() {
        let tid = Uuid::new_v4();
        let root_id = Uuid::new_v4();
        let child_id = Uuid::new_v4();
        let spans = vec![
            make_span(tid, root_id, None, 100.0),
            make_span(tid, child_id, Some(root_id), 40.0),
        ];
        let tree = build_trace_tree(&spans);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].children.len(), 1);
    }

    #[test]
    fn critical_path_picks_longest() {
        let tid = Uuid::new_v4();
        let root = Uuid::new_v4();
        let slow = Uuid::new_v4();
        let fast = Uuid::new_v4();
        let spans = vec![
            make_span(tid, root, None, 10.0),
            make_span(tid, slow, Some(root), 80.0),
            make_span(tid, fast, Some(root), 20.0),
        ];
        let tree = build_trace_tree(&spans);
        let path = calculate_critical_path(&tree);
        assert!(path.contains(&slow));
        assert!(!path.contains(&fast));
    }

    #[test]
    fn detect_anomalous_flags_errors() {
        let tid = Uuid::new_v4();
        let mut span = make_span(tid, Uuid::new_v4(), None, 10.0);
        span.status = SpanStatus::Error;
        let anomalies = detect_anomalous_spans(&[span]);
        assert_eq!(anomalies.len(), 1);
    }
}
