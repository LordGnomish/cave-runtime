//! Module for comparing two distributed traces.
//!
//! This module provides functionality to diff two `Trace` objects,
//! identifying common and unique operations, as well as calculating
//! duration deltas for each operation.

use crate::types::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Represents the result of comparing two traces.
///
/// This struct contains the IDs of the compared traces, the difference
/// in total duration and span count, lists of common and unique operations,
/// and detailed per-operation duration differences.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceComparison {
    /// The ID of the first trace.
    pub trace_a_id: String,
    /// The ID of the second trace.
    pub trace_b_id: String,
    /// The difference in total duration (trace_b - trace_a) in microseconds.
    pub duration_diff_us: i64,
    /// The difference in span count (trace_b - trace_a).
    pub span_count_diff: i32,
    /// List of operation names present in both traces, sorted alphabetically.
    pub common_operations: Vec<String>,
    /// List of operation names present only in trace A, sorted alphabetically.
    pub only_in_a: Vec<String>,
    /// List of operation names present only in trace B, sorted alphabetically.
    pub only_in_b: Vec<String>,
    /// Detailed duration differences for each operation, sorted by absolute delta.
    pub operation_diffs: Vec<OperationDiff>,
}

/// Represents the duration difference for a single operation between two traces.
///
/// If an operation is missing in one of the traces, its duration is `None`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationDiff {
    /// The name of the operation.
    pub operation_name: String,
    /// Total duration of the operation in trace A, if present.
    pub duration_a_us: Option<i64>,
    /// Total duration of the operation in trace B, if present.
    pub duration_b_us: Option<i64>,
    /// The difference in duration (B - A) in microseconds.
    pub duration_diff_us: i64,
}

/// Utility struct for comparing two traces.
pub struct TraceComparer;

impl TraceComparer {
    /// Compares two traces and returns a `TraceComparison`.
    ///
    /// This function calculates the difference in total duration and span count,
    /// identifies common and unique operations, and computes per-operation duration
    /// deltas. The resulting operation diffs are sorted by the absolute value of
    /// the duration difference.
    pub fn compare(a: &Trace, b: &Trace) -> TraceComparison {
        let ops_a: HashSet<&str> = a.spans.iter().map(|s| s.operation_name.as_str()).collect();
        let ops_b: HashSet<&str> = b.spans.iter().map(|s| s.operation_name.as_str()).collect();

        let mut common: Vec<String> = ops_a
             .intersection(&ops_b)
             .map(|s| s.to_string())
             .collect();
        common.sort();
        let mut only_a: Vec<String> = ops_a.difference(&ops_b).map(|s| s.to_string()).collect();
        only_a.sort();
        let mut only_b: Vec<String> = ops_b.difference(&ops_a).map(|s| s.to_string()).collect();
        only_b.sort();

        let all_ops: HashSet<&str> = ops_a.union(&ops_b).copied().collect();
        let mut op_diffs: Vec<OperationDiff> = all_ops
             .iter()
             .map(|op| {
                let dur_a: i64 = a
                     .spans
                     .iter()
                     .filter(|s| s.operation_name == *op)
                     .map(|s| s.duration_us)
                     .sum();
                let dur_b: i64 = b
                     .spans
                     .iter()
                     .filter(|s| s.operation_name == *op)
                     .map(|s| s.duration_us)
                     .sum();
                let has_a = ops_a.contains(op);
                let has_b = ops_b.contains(op);
                OperationDiff {
                    operation_name: op.to_string(),
                    duration_a_us: if has_a { Some(dur_a) } else { None },
                    duration_b_us: if has_b { Some(dur_b) } else { None },
                    duration_diff_us: dur_b - dur_a,
                }
             })
             .collect();
        op_diffs.sort_by(|x, y| {
            y.duration_diff_us
                 .abs()
                 .cmp(&x.duration_diff_us.abs())
         });

        TraceComparison {
            trace_a_id: a.trace_id.clone(),
            trace_b_id: b.trace_id.clone(),
            duration_diff_us: b.duration_us - a.duration_us,
            span_count_diff: b.span_count as i32 - a.span_count as i32,
            common_operations: common,
            only_in_a: only_a,
            only_in_b: only_b,
            operation_diffs: op_diffs,
        }
     }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::collections::HashMap;

    fn make_span(trace_id: &str, span_id: &str, service: &str, op: &str, duration_us: i64) -> Span {
        let now = Utc::now();
        Span {
            trace_id: trace_id.to_string(),
            span_id: span_id.to_string(),
            parent_span_id: None,
            operation_name: op.to_string(),
            service_name: service.to_string(),
            start_time: now,
            end_time: now,
            duration_us,
            status: SpanStatus::Ok,
            kind: SpanKind::Internal,
            tags: HashMap::new(),
            events: vec![],
            links: vec![],
            resource_attributes: HashMap::new(),
        }
    }

    #[test]
    fn trace_comparison() {
        let trace_a = Trace::from_spans(vec![
            make_span("t1", "s1", "svc", "GET /users", 1000),
            make_span("t1", "s2", "svc", "db.query", 500),
            make_span("t1", "s3", "svc", "only-in-a", 200),
        ])
        .unwrap();

        let trace_b = Trace::from_spans(vec![
            make_span("t2", "s4", "svc", "GET /users", 2000),
            make_span("t2", "s5", "svc", "db.query", 800),
            make_span("t2", "s6", "svc", "only-in-b", 300),
        ])
        .unwrap();

        let cmp = TraceComparer::compare(&trace_a, &trace_b);

        assert_eq!(cmp.trace_a_id, "t1");
        assert_eq!(cmp.trace_b_id, "t2");
        assert!(cmp.common_operations.contains(&"GET /users".to_string()));
        assert!(cmp.common_operations.contains(&"db.query".to_string()));
        assert!(cmp.only_in_a.contains(&"only-in-a".to_string()));
        assert!(cmp.only_in_b.contains(&"only-in-b".to_string()));

        // trace_b has more duration
        assert!(cmp.duration_diff_us > 0);
    }
}
