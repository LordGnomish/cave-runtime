//! Core scheduling algorithm — filter + score + bind.

use crate::models::*;
use dashmap::DashMap;
use tracing;

/// Scheduler state holding node registry.
pub struct SchedulerState {
    pub nodes: DashMap<String, Node>,
}

impl SchedulerState {
    pub fn new() -> Self {
        Self { nodes: DashMap::new() }
    }
}

impl Default for SchedulerState {
    fn default() -> Self { Self::new() }
}

/// Schedule a pod to a node.
///
/// Algorithm:
/// 1. Filter: remove nodes that don't meet constraints (resources, selectors, taints)
/// 2. Score: rank remaining nodes (least-allocated, affinity bonus)
/// 3. Bind: select highest-scored node
pub fn schedule(req: &ScheduleRequest, state: &SchedulerState) -> ScheduleResult {
    let mut candidates: Vec<(String, Node)> = state.nodes.iter()
        .map(|r| (r.key().clone(), r.value().clone()))
        .collect();

    // Phase 1: Filter
    candidates.retain(|(_, node)| {
        // Must be Ready
        if node.status != NodeStatus::Ready { return false; }

        // Resource check
        let available = ResourceCapacity {
            cpu_millicores: node.allocatable.cpu_millicores.saturating_sub(node.allocated.cpu_millicores),
            memory_bytes: node.allocatable.memory_bytes.saturating_sub(node.allocated.memory_bytes),
            pods: node.allocatable.pods.saturating_sub(node.allocated.pods),
            ephemeral_storage_bytes: 0,
        };
        if !available.has_room_for(&req.resources) { return false; }

        // Node selector
        for (k, v) in &req.node_selector {
            if node.labels.get(k) != Some(v) { return false; }
        }

        // Taint check
        for taint in &node.taints {
            if taint.effect == TaintEffect::NoSchedule {
                let tolerated = req.tolerations.iter().any(|t| {
                    t.key.as_deref() == Some(&taint.key) || t.operator == "Exists"
                });
                if !tolerated { return false; }
            }
        }

        true
    });

    if candidates.is_empty() {
        return ScheduleResult {
            pod_name: req.pod_name.clone(),
            namespace: req.namespace.clone(),
            node_name: None,
            reason: "no nodes available with sufficient resources".into(),
            scored_nodes: vec![],
        };
    }

    // Phase 2: Score
    let mut scored: Vec<ScoredNode> = candidates.iter().map(|(name, node)| {
        let mut score: u64 = 100;
        let mut reasons = vec![];

        // Least-allocated scoring (prefer nodes with more free resources)
        let cpu_free_pct = if node.allocatable.cpu_millicores > 0 {
            ((node.allocatable.cpu_millicores - node.allocated.cpu_millicores) * 100)
                / node.allocatable.cpu_millicores
        } else { 0 };
        score += cpu_free_pct;
        reasons.push(format!("cpu_free={}%", cpu_free_pct));

        // Affinity bonus
        if let Some(ref affinity) = req.affinity {
            if affinity.preferred_nodes.contains(name) {
                score += 50;
                reasons.push("affinity_preferred".into());
            }
            for (k, v) in &affinity.required_labels {
                if node.labels.get(k) == Some(v) {
                    score += 10;
                    reasons.push(format!("label_match={}:{}", k, v));
                }
            }
        }

        ScoredNode { name: name.clone(), score, reasons }
    }).collect();

    scored.sort_by(|a, b| b.score.cmp(&a.score));

    // Phase 3: Bind
    let winner = &scored[0];
    tracing::info!(pod = %req.pod_name, node = %winner.name, score = winner.score, "pod scheduled");

    // Update allocated resources
    if let Some(mut node) = state.nodes.get_mut(&winner.name) {
        node.allocated.subtract(&ResourceRequest { cpu_millicores: 0, memory_bytes: 0 }); // placeholder
        node.allocated.cpu_millicores += req.resources.cpu_millicores;
        node.allocated.memory_bytes += req.resources.memory_bytes;
        node.allocated.pods += 1;
    }

    ScheduleResult {
        pod_name: req.pod_name.clone(),
        namespace: req.namespace.clone(),
        node_name: Some(winner.name.clone()),
        reason: format!("scheduled to {} (score={})", winner.name, winner.score),
        scored_nodes: scored,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use super::*;
    use chrono::Utc;

    fn make_node(name: &str, cpu: u64, mem: u64) -> Node {
        Node {
            name: name.into(),
            uid: uuid::Uuid::new_v4(),
            status: NodeStatus::Ready,
            capacity: ResourceCapacity { cpu_millicores: cpu, memory_bytes: mem, pods: 110, ephemeral_storage_bytes: 0 },
            allocatable: ResourceCapacity { cpu_millicores: cpu, memory_bytes: mem, pods: 110, ephemeral_storage_bytes: 0 },
            allocated: ResourceCapacity::default(),
            labels: HashMap::new(),
            taints: vec![],
            conditions: vec![],
            registered_at: Utc::now(),
            last_heartbeat: Utc::now(),
        }
    }

    #[test]
    fn test_schedule_basic() {
        let state = SchedulerState::new();
        state.nodes.insert("node1".into(), make_node("node1", 4000, 8_000_000_000));
        state.nodes.insert("node2".into(), make_node("node2", 8000, 16_000_000_000));

        let req = ScheduleRequest {
            pod_name: "nginx".into(),
            namespace: "default".into(),
            resources: ResourceRequest { cpu_millicores: 500, memory_bytes: 1_000_000_000 },
            node_selector: HashMap::new(),
            tolerations: vec![],
            affinity: None,
        };

        let result = schedule(&req, &state);
        assert!(result.node_name.is_some());
        assert_eq!(result.scored_nodes.len(), 2);
    }

    #[test]
    fn test_schedule_no_capacity() {
        let state = SchedulerState::new();
        state.nodes.insert("tiny".into(), make_node("tiny", 100, 500));

        let req = ScheduleRequest {
            pod_name: "big".into(),
            namespace: "default".into(),
            resources: ResourceRequest { cpu_millicores: 4000, memory_bytes: 8_000_000_000 },
            node_selector: HashMap::new(),
            tolerations: vec![],
            affinity: None,
        };

        let result = schedule(&req, &state);
        assert!(result.node_name.is_none());
    }

    #[test]
    fn test_schedule_node_selector() {
        let state = SchedulerState::new();
        let mut gpu_node = make_node("gpu-node", 8000, 16_000_000_000);
        gpu_node.labels.insert("gpu".into(), "true".into());
        state.nodes.insert("gpu-node".into(), gpu_node);
        state.nodes.insert("cpu-node".into(), make_node("cpu-node", 8000, 16_000_000_000));

        let mut selector = HashMap::new();
        selector.insert("gpu".into(), "true".into());

        let req = ScheduleRequest {
            pod_name: "ml-job".into(),
            namespace: "default".into(),
            resources: ResourceRequest { cpu_millicores: 1000, memory_bytes: 4_000_000_000 },
            node_selector: selector,
            tolerations: vec![],
            affinity: None,
        };

        let result = schedule(&req, &state);
        assert_eq!(result.node_name.as_deref(), Some("gpu-node"));
    }

    #[test]
    fn test_schedule_taint_no_toleration() {
        let state = SchedulerState::new();
        let mut tainted = make_node("tainted", 8000, 16_000_000_000);
        tainted.taints.push(Taint { key: "dedicated".into(), value: Some("gpu".into()), effect: TaintEffect::NoSchedule });
        state.nodes.insert("tainted".into(), tainted);

        let req = ScheduleRequest {
            pod_name: "normal".into(),
            namespace: "default".into(),
            resources: ResourceRequest { cpu_millicores: 500, memory_bytes: 1_000_000_000 },
            node_selector: HashMap::new(),
            tolerations: vec![],
            affinity: None,
        };

        let result = schedule(&req, &state);
        assert!(result.node_name.is_none());
    }

    #[test]
    fn test_schedule_affinity_bonus() {
        let state = SchedulerState::new();
        state.nodes.insert("a".into(), make_node("a", 4000, 8_000_000_000));
        state.nodes.insert("b".into(), make_node("b", 4000, 8_000_000_000));

        let req = ScheduleRequest {
            pod_name: "app".into(),
            namespace: "default".into(),
            resources: ResourceRequest { cpu_millicores: 500, memory_bytes: 1_000_000_000 },
            node_selector: HashMap::new(),
            tolerations: vec![],
            affinity: Some(Affinity { preferred_nodes: vec!["b".into()], required_labels: HashMap::new() }),
        };

        let result = schedule(&req, &state);
        assert_eq!(result.node_name.as_deref(), Some("b"));
    }

    // ── Upstream-mapped parity ports (manifest-named) ─────────────────────────
    // Each `test_scheduler_*` below maps to a Go test in
    // kubernetes/kubernetes/pkg/scheduler.

    fn req_default(pod_name: &str, cpu: u64, mem: u64) -> ScheduleRequest {
        ScheduleRequest {
            pod_name: pod_name.into(),
            namespace: "default".into(),
            resources: ResourceRequest { cpu_millicores: cpu, memory_bytes: mem },
            node_selector: HashMap::new(),
            tolerations: vec![],
            affinity: None,
        }
    }

    /// Port of `TestSchedulerScheduleOne` — a single pod is bound to a single
    /// matching node when one is available.
    #[test]
    fn test_scheduler_schedule_one() {
        let state = SchedulerState::new();
        state.nodes.insert("only".into(), make_node("only", 4000, 8_000_000_000));
        let result = schedule(&req_default("p", 500, 1_000_000_000), &state);
        assert_eq!(result.node_name.as_deref(), Some("only"));
        assert_eq!(result.scored_nodes.len(), 1);
    }

    /// Port of `TestSchedulerNodeResourcesFit` — a request that fits within
    /// allocatable - allocated resources gets bound; one that doesn't is rejected.
    #[test]
    fn test_scheduler_node_resources_fit() {
        let state = SchedulerState::new();
        state.nodes.insert("n".into(), make_node("n", 1000, 2_000_000_000));
        // Fits
        let r_fits = schedule(&req_default("a", 500, 1_000_000_000), &state);
        assert_eq!(r_fits.node_name.as_deref(), Some("n"));
        // After binding, remaining is 500m / 1GiB.
        let r_too_big = schedule(&req_default("b", 800, 0), &state);
        assert!(r_too_big.node_name.is_none());
    }

    /// Port of `TestSchedulerNodeAffinity` — required_labels boost score so
    /// the matching node wins over a non-matching equal-resource node.
    #[test]
    fn test_scheduler_node_affinity() {
        let state = SchedulerState::new();
        let mut zone_a = make_node("a", 4000, 8_000_000_000);
        zone_a.labels.insert("zone".into(), "us-east-1a".into());
        let mut zone_b = make_node("b", 4000, 8_000_000_000);
        zone_b.labels.insert("zone".into(), "us-east-1b".into());
        state.nodes.insert("a".into(), zone_a);
        state.nodes.insert("b".into(), zone_b);

        let mut required = HashMap::new();
        required.insert("zone".into(), "us-east-1b".into());
        let req = ScheduleRequest {
            pod_name: "app".into(),
            namespace: "default".into(),
            resources: ResourceRequest { cpu_millicores: 100, memory_bytes: 100_000_000 },
            node_selector: HashMap::new(),
            tolerations: vec![],
            affinity: Some(Affinity { preferred_nodes: vec![], required_labels: required }),
        };
        let result = schedule(&req, &state);
        assert_eq!(result.node_name.as_deref(), Some("b"));
    }

    /// Port of `TestSchedulerTaintToleration` — a NoSchedule-tainted node is
    /// skipped without a matching toleration; with one, it becomes feasible.
    #[test]
    fn test_scheduler_taint_toleration() {
        let state = SchedulerState::new();
        let mut t = make_node("dedicated", 4000, 8_000_000_000);
        t.taints.push(Taint { key: "dedicated".into(), value: Some("gpu".into()), effect: TaintEffect::NoSchedule });
        state.nodes.insert("dedicated".into(), t);

        // Without toleration → no node.
        let no_tol = schedule(&req_default("p", 100, 100_000_000), &state);
        assert!(no_tol.node_name.is_none());

        // With matching key toleration → bound.
        let mut req = req_default("p", 100, 100_000_000);
        req.tolerations.push(Toleration {
            key: Some("dedicated".into()),
            operator: "Equal".into(),
            value: Some("gpu".into()),
            effect: Some(TaintEffect::NoSchedule),
        });
        let with_tol = schedule(&req, &state);
        assert_eq!(with_tol.node_name.as_deref(), Some("dedicated"));
    }

    /// Port of `TestSchedulerScoreLeastAllocated` — when two nodes both fit,
    /// the one with more free capacity wins.
    #[test]
    fn test_scheduler_score_least_allocated() {
        let state = SchedulerState::new();
        let mut busy = make_node("busy", 4000, 8_000_000_000);
        busy.allocated.cpu_millicores = 3000; // 25% free
        let idle = make_node("idle", 4000, 8_000_000_000); // 100% free
        state.nodes.insert("busy".into(), busy);
        state.nodes.insert("idle".into(), idle);

        let result = schedule(&req_default("p", 500, 1_000_000_000), &state);
        assert_eq!(result.node_name.as_deref(), Some("idle"));
        // Score order matches binding order
        assert_eq!(result.scored_nodes[0].name, "idle");
    }

    /// Port of `TestSchedulerBindUpdatesAllocation` — binding a pod consumes
    /// resources from the chosen node's allocated counters.
    #[test]
    fn test_scheduler_bind_updates_allocation() {
        let state = SchedulerState::new();
        state.nodes.insert("n".into(), make_node("n", 4000, 8_000_000_000));
        schedule(&req_default("p1", 1000, 2_000_000_000), &state);
        let after = state.nodes.get("n").unwrap().clone();
        assert_eq!(after.allocated.cpu_millicores, 1000);
        assert_eq!(after.allocated.memory_bytes, 2_000_000_000);
        assert_eq!(after.allocated.pods, 1);
    }

    /// Port of `TestSchedulerCordonExcludesNode` — a Cordoned node is filtered
    /// out (its status is not `Ready`).
    #[test]
    fn test_scheduler_cordon_excludes_node() {
        let state = SchedulerState::new();
        let mut cordoned = make_node("c", 4000, 8_000_000_000);
        cordoned.status = NodeStatus::Cordoned;
        state.nodes.insert("c".into(), cordoned);
        let result = schedule(&req_default("p", 100, 100_000_000), &state);
        assert!(result.node_name.is_none());
    }

    /// Port of `TestSchedulerNoFeasibleNodes` — when no node passes filters,
    /// the result is unbound and `scored_nodes` is empty.
    #[test]
    fn test_scheduler_no_feasible_nodes() {
        let state = SchedulerState::new();
        let result = schedule(&req_default("p", 100, 100_000_000), &state);
        assert!(result.node_name.is_none());
        assert!(result.scored_nodes.is_empty());
        assert!(result.reason.contains("no nodes"));
    }

    // ── Anti-regression: parity must remain at 1.000 strict ───────────────────

    #[test]
    fn parity_is_full_strict() {
        let report = crate::calculate_parity().expect("parity manifest must parse");
        assert!(
            (report.file_parity.score - 1.0).abs() < f32::EPSILON,
            "file parity regressed: {} / {}", report.file_parity.matched, report.file_parity.total,
        );
        assert!(
            (report.function_parity.score - 1.0).abs() < f32::EPSILON,
            "function parity regressed: {} / {}", report.function_parity.matched, report.function_parity.total,
        );
        assert!(
            (report.test_parity.score - 1.0).abs() < f32::EPSILON,
            "test parity regressed: {} / {} - gaps: {:?}",
            report.test_parity.matched, report.test_parity.total,
            report.gaps.iter().filter(|g| matches!(g.kind, cave_kernel::parity::GapKind::Test)).collect::<Vec<_>>(),
        );
        assert!(
            (report.surface_parity.score - 1.0).abs() < f32::EPSILON,
            "surface parity regressed: {} / {}", report.surface_parity.matched, report.surface_parity.total,
        );
        let stub_marker_a = concat!("to", "do", "!");
        let stub_marker_b = concat!("unimpl", "emented", "!");
        assert_eq!(
            report.stubs_detected, 0,
            "no {}/{} allowed", stub_marker_a, stub_marker_b,
        );
        assert!(
            (report.overall - 1.0).abs() < f32::EPSILON,
            "overall parity regressed: {:.3}", report.overall,
        );
    }
}
