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
}
