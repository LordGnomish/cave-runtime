// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Scheduler facade.
//!
//! The strongly-typed scheduler implementation lives in
//! `cave-scheduler` (plugin framework + 13 extension points). This
//! facade exposes the *outcome* surface used by the umbrella: pod
//! placement requests, scheduling decisions, scheduler-name routing.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacementRequest {
    pub namespace: String,
    pub pod_name: String,
    /// Default `"default-scheduler"`; overridden by Pod
    /// `.spec.schedulerName`.
    pub scheduler_name: String,
    pub cpu_request_millis: u32,
    pub memory_request_bytes: u64,
    pub node_selector: std::collections::BTreeMap<String, String>,
    pub tolerations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlacementOutcome {
    Bound { node: String, score: i64 },
    Unschedulable { reason: String },
    PreemptionNeeded { victims: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeCandidate {
    pub name: String,
    pub cpu_allocatable_millis: u32,
    pub memory_allocatable_bytes: u64,
    pub labels: std::collections::BTreeMap<String, String>,
    pub taints: Vec<String>,
}

/// Pure-Rust placement decision over the candidate set.  Mirrors the
/// `NodeResourcesFit + NodeAffinity + TaintToleration + Score` sub-set
/// of the upstream scheduler framework that the umbrella needs for
/// integration tests.
pub fn place(
    req: &PlacementRequest,
    candidates: &[NodeCandidate],
) -> PlacementOutcome {
    let feasible: Vec<&NodeCandidate> = candidates
        .iter()
        .filter(|c| c.cpu_allocatable_millis >= req.cpu_request_millis)
        .filter(|c| c.memory_allocatable_bytes >= req.memory_request_bytes)
        .filter(|c| {
            req.node_selector
                .iter()
                .all(|(k, v)| c.labels.get(k).map(|x| x.as_str()) == Some(v.as_str()))
        })
        .filter(|c| {
            c.taints.iter().all(|t| req.tolerations.iter().any(|r| r == t))
        })
        .collect();
    if feasible.is_empty() {
        return PlacementOutcome::Unschedulable {
            reason: "no feasible nodes".into(),
        };
    }
    let mut best = (&feasible[0], score_node(feasible[0], req));
    for c in &feasible[1..] {
        let s = score_node(c, req);
        if s > best.1 {
            best = (c, s);
        }
    }
    PlacementOutcome::Bound {
        node: best.0.name.clone(),
        score: best.1,
    }
}

fn score_node(c: &NodeCandidate, req: &PlacementRequest) -> i64 {
    // Higher is better: prefer nodes with the most *remaining* capacity
    // after placement.  Mirrors `noderesources.LeastAllocated`.
    let cpu_left = c.cpu_allocatable_millis.saturating_sub(req.cpu_request_millis) as i64;
    let mem_left = (c.memory_allocatable_bytes.saturating_sub(req.memory_request_bytes) / (1024 * 1024)) as i64;
    cpu_left + mem_left
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, cpu: u32, mem: u64) -> NodeCandidate {
        NodeCandidate {
            name: name.into(),
            cpu_allocatable_millis: cpu,
            memory_allocatable_bytes: mem,
            labels: Default::default(),
            taints: Vec::new(),
        }
    }

    fn req(cpu: u32, mem: u64) -> PlacementRequest {
        PlacementRequest {
            namespace: "default".into(),
            pod_name: "p".into(),
            scheduler_name: "default-scheduler".into(),
            cpu_request_millis: cpu,
            memory_request_bytes: mem,
            node_selector: Default::default(),
            tolerations: Vec::new(),
        }
    }

    #[test]
    fn no_nodes_unschedulable() {
        assert!(matches!(
            place(&req(100, 100), &[]),
            PlacementOutcome::Unschedulable { .. }
        ));
    }

    #[test]
    fn picks_node_with_most_remaining_capacity() {
        let nodes = vec![
            node("small", 200, 1024),
            node("large", 4000, 1024 * 1024),
        ];
        let out = place(&req(100, 100), &nodes);
        match out {
            PlacementOutcome::Bound { node, .. } => assert_eq!(node, "large"),
            o => panic!("{:?}", o),
        }
    }

    #[test]
    fn rejects_when_cpu_insufficient() {
        let nodes = vec![node("a", 50, 1024 * 1024 * 1024)];
        assert!(matches!(
            place(&req(1000, 0), &nodes),
            PlacementOutcome::Unschedulable { .. }
        ));
    }

    #[test]
    fn rejects_when_memory_insufficient() {
        let nodes = vec![node("a", 4000, 1024)];
        assert!(matches!(
            place(&req(100, 1024 * 1024 * 1024), &nodes),
            PlacementOutcome::Unschedulable { .. }
        ));
    }

    #[test]
    fn node_selector_must_match() {
        let mut n = node("a", 1000, 1024 * 1024 * 1024);
        n.labels.insert("zone".into(), "eu-1".into());
        let mut r = req(100, 0);
        r.node_selector.insert("zone".into(), "us-1".into());
        assert!(matches!(
            place(&r, &[n.clone()]),
            PlacementOutcome::Unschedulable { .. }
        ));
        let mut r2 = req(100, 0);
        r2.node_selector.insert("zone".into(), "eu-1".into());
        assert!(matches!(
            place(&r2, &[n]),
            PlacementOutcome::Bound { .. }
        ));
    }

    #[test]
    fn taints_require_matching_toleration() {
        let mut n = node("a", 1000, 1024 * 1024 * 1024);
        n.taints.push("dedicated=gpu:NoSchedule".into());
        let r1 = req(100, 0);
        assert!(matches!(
            place(&r1, &[n.clone()]),
            PlacementOutcome::Unschedulable { .. }
        ));
        let mut r2 = req(100, 0);
        r2.tolerations.push("dedicated=gpu:NoSchedule".into());
        assert!(matches!(
            place(&r2, &[n]),
            PlacementOutcome::Bound { .. }
        ));
    }

    #[test]
    fn outcome_serialization_roundtrip() {
        let o = PlacementOutcome::Bound {
            node: "n1".into(),
            score: 42,
        };
        let s = serde_json::to_string(&o).unwrap();
        let back: PlacementOutcome = serde_json::from_str(&s).unwrap();
        assert_eq!(back, o);
    }
}
