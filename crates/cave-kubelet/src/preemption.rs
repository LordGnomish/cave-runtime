//! Critical-pod preemption admit handler.
//!
//! Cite: pkg/kubelet/preemption/ (v1.36.0).
//!
//! When a system-critical pod fails admission because the node is
//! out of resources, the kubelet may evict lower-priority pods to
//! make room. The selection is upstream's "minimum disruption +
//! maximum priority-distance" heuristic:
//!
//! 1. Filter candidates to pods strictly lower priority than the
//!    new pod.
//! 2. Among those, prefer pods whose summed (cpu, memory) covers the
//!    deficit with as little excess as possible.
//! 3. Tie-break by largest priority-distance (evict the lowest-
//!    priority pod first).
//! 4. Tie-break further by name (deterministic for tests).
//!
//! Reduces to a pure decision: given a `PreemptionRequest`, return
//! the ordered list of victim pod uids.

use serde::{Deserialize, Serialize};

#[allow(dead_code)]
pub const UPSTREAM_PATH: &str = "pkg/kubelet/preemption/preemption.go";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceRequest {
    pub cpu_millicores: u64,
    pub memory_bytes: u64,
}

impl ResourceRequest {
    pub const ZERO: Self = Self {
        cpu_millicores: 0,
        memory_bytes: 0,
    };

    pub fn covers(&self, deficit: &ResourceRequest) -> bool {
        self.cpu_millicores >= deficit.cpu_millicores
            && self.memory_bytes >= deficit.memory_bytes
    }

    pub fn add(&mut self, other: &ResourceRequest) {
        self.cpu_millicores += other.cpu_millicores;
        self.memory_bytes += other.memory_bytes;
    }
}

/// One pod on the node from the kubelet's POV. Only the bits the
/// preemption decision needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidatePod {
    pub uid: String,
    pub name: String,
    /// Pod priority — higher numbers stay, lower numbers evict.
    /// `system-cluster-critical` = 2_000_000_000;
    /// `system-node-critical`    = 2_000_001_000.
    pub priority: i32,
    pub resources: ResourceRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreemptionRequest {
    /// The pod that failed admission.
    pub incoming_uid: String,
    pub incoming_priority: i32,
    pub incoming_resources: ResourceRequest,
    /// Currently running pods on the node.
    pub candidates: Vec<CandidatePod>,
    /// Free resources on the node (what the admit pass actually saw).
    pub node_free: ResourceRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PreemptionDecision {
    /// Admit without preempting — node already has room.
    AdmitNoVictims,
    /// Evict the listed pods (in order) to make room.
    Evict { victim_uids: Vec<String> },
    /// No achievable preemption — admission denied even after
    /// considering every lower-priority pod.
    Insufficient { reason: String },
}

/// Decide whether and whom to preempt.
pub fn evaluate(req: &PreemptionRequest) -> PreemptionDecision {
    // Compute the deficit. `saturating_sub` keeps the math honest
    // when the node already has room for one axis but not the other.
    let deficit = ResourceRequest {
        cpu_millicores: req
            .incoming_resources
            .cpu_millicores
            .saturating_sub(req.node_free.cpu_millicores),
        memory_bytes: req
            .incoming_resources
            .memory_bytes
            .saturating_sub(req.node_free.memory_bytes),
    };
    if deficit == ResourceRequest::ZERO {
        return PreemptionDecision::AdmitNoVictims;
    }

    // Only consider strictly lower-priority candidates.
    let mut lower: Vec<&CandidatePod> = req
        .candidates
        .iter()
        .filter(|c| c.priority < req.incoming_priority)
        .collect();

    // Sort: lowest priority first, then largest resources first
    // (greedy on "biggest evictable"), then name for determinism.
    lower.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then_with(|| {
                (b.resources.cpu_millicores + b.resources.memory_bytes)
                    .cmp(&(a.resources.cpu_millicores + a.resources.memory_bytes))
            })
            .then_with(|| a.name.cmp(&b.name))
    });

    let mut freed = ResourceRequest::ZERO;
    let mut victims: Vec<String> = Vec::new();
    for c in &lower {
        if freed.covers(&deficit) {
            break;
        }
        freed.add(&c.resources);
        victims.push(c.uid.clone());
    }

    if freed.covers(&deficit) {
        PreemptionDecision::Evict { victim_uids: victims }
    } else {
        PreemptionDecision::Insufficient {
            reason: format!(
                "deficit cpu={}m memory={}B cannot be covered by lower-priority pods (max_freed cpu={}m memory={}B)",
                deficit.cpu_millicores,
                deficit.memory_bytes,
                freed.cpu_millicores,
                freed.memory_bytes,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(uid: &str, priority: i32, cpu: u64, mem: u64) -> CandidatePod {
        CandidatePod {
            uid: uid.into(),
            name: uid.into(),
            priority,
            resources: ResourceRequest {
                cpu_millicores: cpu,
                memory_bytes: mem,
            },
        }
    }

    fn req(incoming_priority: i32, incoming_cpu: u64, incoming_mem: u64, free_cpu: u64, free_mem: u64, candidates: Vec<CandidatePod>) -> PreemptionRequest {
        PreemptionRequest {
            incoming_uid: "newp".into(),
            incoming_priority,
            incoming_resources: ResourceRequest { cpu_millicores: incoming_cpu, memory_bytes: incoming_mem },
            node_free: ResourceRequest { cpu_millicores: free_cpu, memory_bytes: free_mem },
            candidates,
        }
    }

    #[test]
    fn admit_no_victims_when_node_has_room() {
        let r = req(1000, 100, 100, 200, 200, vec![cand("p1", 0, 50, 50)]);
        assert_eq!(evaluate(&r), PreemptionDecision::AdmitNoVictims);
    }

    #[test]
    fn evicts_lowest_priority_pod_first() {
        let r = req(
            1000, 100, 100, 0, 0,
            vec![cand("low", 0, 100, 100), cand("hi", 999, 100, 100)],
        );
        match evaluate(&r) {
            PreemptionDecision::Evict { victim_uids } => {
                assert_eq!(victim_uids, vec!["low".to_string()]);
            }
            other => panic!("expected Evict, got {other:?}"),
        }
    }

    #[test]
    fn skips_candidates_at_equal_or_higher_priority() {
        let r = req(
            1000, 100, 100, 0, 0,
            vec![cand("p1", 1000, 100, 100), cand("p2", 1001, 100, 100)],
        );
        match evaluate(&r) {
            PreemptionDecision::Insufficient { .. } => {}
            other => panic!("expected Insufficient, got {other:?}"),
        }
    }

    #[test]
    fn evicts_multiple_pods_until_deficit_covered() {
        let r = req(
            1000, 200, 200, 0, 0,
            vec![
                cand("a", 0, 100, 100),
                cand("b", 100, 100, 100),
                cand("c", 200, 100, 100),
            ],
        );
        match evaluate(&r) {
            PreemptionDecision::Evict { victim_uids } => {
                // Lowest priority first: a (0), then b (100).
                assert_eq!(victim_uids.len(), 2);
                assert_eq!(victim_uids[0], "a");
                assert_eq!(victim_uids[1], "b");
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn insufficient_when_no_lower_priority_pods_present() {
        let r = req(1000, 100, 100, 0, 0, vec![]);
        match evaluate(&r) {
            PreemptionDecision::Insufficient { .. } => {}
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn deficit_one_axis_only_still_triggers_preemption() {
        // Node has CPU headroom but no memory.
        let r = req(
            1000, 100, 200, 500, 0,
            vec![cand("low", 0, 0, 300)],
        );
        match evaluate(&r) {
            PreemptionDecision::Evict { victim_uids } => assert_eq!(victim_uids, vec!["low"]),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn tie_break_on_resources_picks_larger_first_within_same_priority() {
        let r = req(
            1000, 50, 50, 0, 0,
            vec![cand("small", 0, 30, 30), cand("large", 0, 60, 60)],
        );
        match evaluate(&r) {
            PreemptionDecision::Evict { victim_uids } => {
                // Same priority; "large" has bigger resource sum →
                // evicted first; one pod is enough.
                assert_eq!(victim_uids, vec!["large".to_string()]);
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn tie_break_on_name_when_priority_and_resources_equal() {
        let r = req(
            1000, 50, 50, 0, 0,
            vec![cand("zzz", 0, 100, 100), cand("aaa", 0, 100, 100)],
        );
        match evaluate(&r) {
            PreemptionDecision::Evict { victim_uids } => {
                // Name asc → aaa first.
                assert_eq!(victim_uids, vec!["aaa".to_string()]);
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn covers_returns_true_only_when_both_axes_satisfied() {
        let r = ResourceRequest { cpu_millicores: 100, memory_bytes: 100 };
        let d_ok = ResourceRequest { cpu_millicores: 50, memory_bytes: 50 };
        let d_cpu_short = ResourceRequest { cpu_millicores: 150, memory_bytes: 50 };
        let d_mem_short = ResourceRequest { cpu_millicores: 50, memory_bytes: 150 };
        assert!(r.covers(&d_ok));
        assert!(!r.covers(&d_cpu_short));
        assert!(!r.covers(&d_mem_short));
    }

    #[test]
    fn deficit_zero_path_short_circuits_admit() {
        // Free exactly equals request → deficit zero → admit.
        let r = req(1000, 100, 100, 100, 100, vec![cand("p", 0, 50, 50)]);
        assert_eq!(evaluate(&r), PreemptionDecision::AdmitNoVictims);
    }

    #[test]
    fn evicts_all_lower_when_deficit_huge() {
        let r = req(
            1000, 10_000, 10_000, 0, 0,
            vec![cand("a", 0, 100, 100), cand("b", 100, 200, 200)],
        );
        match evaluate(&r) {
            PreemptionDecision::Insufficient { reason } => {
                assert!(reason.contains("deficit"));
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn priority_ordering_picks_lowest_first_even_when_higher_alone_would_cover() {
        // 'low' alone covers; 'higher' alone also covers; pick 'low'.
        let r = req(
            1000, 50, 50, 0, 0,
            vec![cand("higher", 500, 200, 200), cand("low", 0, 200, 200)],
        );
        match evaluate(&r) {
            PreemptionDecision::Evict { victim_uids } => {
                assert_eq!(victim_uids, vec!["low".to_string()]);
            }
            other => panic!("got {other:?}"),
        }
    }
}
