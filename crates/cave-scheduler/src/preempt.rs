//! PostFilter preemption — when no node admits the pod, evict lower-priority pods
//! to make room. Sets NominatedNodeName on the preemptor.
//!
//! Cite: kubernetes/kubernetes v1.36.0
//!   pkg/scheduler/framework/plugins/defaultpreemption/default_preemption.go

use crate::framework::*;
use crate::models::ResourceCapacity;
use std::collections::HashMap;

/// PodDisruptionBudget — limit on simultaneously disrupted pods of a selector.
#[derive(Debug, Clone)]
pub struct PodDisruptionBudget {
    pub name: String,
    pub namespace: String,
    pub tenant_id: String,
    pub selector: HashMap<String, String>,
    pub min_available: usize,
    pub current_healthy: usize,
}

impl PodDisruptionBudget {
    pub fn would_violate(&self, victims_to_evict: usize) -> bool {
        self.current_healthy.saturating_sub(victims_to_evict) < self.min_available
    }
    fn matches(&self, pod: &Pod) -> bool {
        if pod.namespace != self.namespace || pod.tenant_id != self.tenant_id { return false; }
        self.selector.iter().all(|(k, v)| pod.spec.node_selector.get(k) == Some(v))
    }
}

/// PreemptionResult — node selected and victims to evict on it.
#[derive(Debug, Clone)]
pub struct PreemptionResult {
    pub nominated_node_name: String,
    pub victims: Vec<Pod>,
    pub pdb_violations: usize,
}

/// Try to preempt: for each node where the preemptor failed only on Resources,
/// pick the smallest set of strictly-lower-priority victims whose removal frees
/// enough resources. Tie-break by node name (deterministic).
pub fn preempt(
    preemptor: &Pod,
    snapshot: &ClusterSnapshot,
    pdbs: &[PodDisruptionBudget],
) -> Option<PreemptionResult> {
    let mut best: Option<PreemptionResult> = None;
    let mut nodes: Vec<&crate::models::Node> = snapshot.nodes.iter().collect();
    nodes.sort_by(|a, b| a.name.cmp(&b.name));
    for node in nodes {
        let pods = snapshot.pods_on(&node.name);
        // Only victims with strictly lower priority are eligible; cross-tenant pods
        // are never preemptable (multi-tenant invariant).
        let mut candidates: Vec<&Pod> = pods.iter()
            .filter(|p| p.spec.priority < preemptor.spec.priority && p.tenant_id == preemptor.tenant_id)
            .collect();
        // Lowest priority first → if tie, larger CPU first (frees more per evict).
        candidates.sort_by(|a, b| a.spec.priority.cmp(&b.spec.priority)
            .then_with(|| b.spec.resources.cpu_millicores.cmp(&a.spec.resources.cpu_millicores)));

        let need = &preemptor.spec.resources;
        let mut freed_cpu: u64 = 0;
        let mut freed_mem: u64 = 0;
        let avail = ResourceCapacity {
            cpu_millicores: node.allocatable.cpu_millicores.saturating_sub(node.allocated.cpu_millicores),
            memory_bytes: node.allocatable.memory_bytes.saturating_sub(node.allocated.memory_bytes),
            pods: node.allocatable.pods.saturating_sub(node.allocated.pods),
            ephemeral_storage_bytes: 0,
        };

        let mut victims: Vec<Pod> = vec![];
        let mut pdb_violations: usize = 0;

        for v in candidates {
            if avail.cpu_millicores + freed_cpu >= need.cpu_millicores
                && avail.memory_bytes + freed_mem >= need.memory_bytes {
                break;
            }
            // PDB respect: if evicting this pod would violate any PDB, count and skip.
            let v_pdbs: Vec<&PodDisruptionBudget> = pdbs.iter().filter(|b| b.matches(v)).collect();
            let violates = v_pdbs.iter().any(|b| b.would_violate(1));
            if violates { pdb_violations += 1; continue; }
            freed_cpu += v.spec.resources.cpu_millicores;
            freed_mem += v.spec.resources.memory_bytes;
            victims.push(v.clone());
        }

        let fits_after = avail.cpu_millicores + freed_cpu >= need.cpu_millicores
            && avail.memory_bytes + freed_mem >= need.memory_bytes;
        if !fits_after { continue; }

        let candidate = PreemptionResult {
            nominated_node_name: node.name.clone(),
            victims,
            pdb_violations,
        };
        // Prefer node with fewer victims; tie → fewer PDB violations; tie → name.
        best = Some(match best.take() {
            None => candidate,
            Some(prev) => {
                if (candidate.victims.len(), candidate.pdb_violations)
                    < (prev.victims.len(), prev.pdb_violations)
                { candidate } else { prev }
            }
        });
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Node, NodeStatus, ResourceCapacity, ResourceRequest};
    use chrono::Utc;
    use uuid::Uuid;

    fn full_node(name: &str) -> Node {
        Node {
            name: name.into(), uid: Uuid::new_v4(), status: NodeStatus::Ready,
            capacity: ResourceCapacity { cpu_millicores: 4000, memory_bytes: 8_000_000_000, pods: 110, ephemeral_storage_bytes: 0 },
            allocatable: ResourceCapacity { cpu_millicores: 4000, memory_bytes: 8_000_000_000, pods: 110, ephemeral_storage_bytes: 0 },
            allocated: ResourceCapacity { cpu_millicores: 4000, memory_bytes: 8_000_000_000, pods: 5, ephemeral_storage_bytes: 0 },
            labels: HashMap::new(), taints: vec![], conditions: vec![],
            registered_at: Utc::now(), last_heartbeat: Utc::now(),
        }
    }

    fn pod_at(tenant: &str, name: &str, prio: i32, cpu: u64, mem: u64) -> Pod {
        let mut p = Pod::new(tenant, "ns", name);
        p.spec.priority = prio;
        p.spec.resources = ResourceRequest { cpu_millicores: cpu, memory_bytes: mem };
        p
    }

    #[test]
    fn preempt_picks_minimum_victims() {
        let mut node = full_node("a");
        node.allocated.cpu_millicores = 3500;
        let snap = ClusterSnapshot {
            nodes: vec![node],
            pods_by_node: HashMap::from([("a".into(), vec![
                pod_at("t", "low", 0, 1000, 1_000_000_000),
                pod_at("t", "low2", 0, 500, 500_000_000),
                pod_at("t", "high", 100, 2000, 6_000_000_000),
            ])]),
        };
        let preemptor = pod_at("t", "new", 50, 1500, 0);
        let res = preempt(&preemptor, &snap, &[]).expect("should preempt");
        assert_eq!(res.nominated_node_name, "a");
        // need 1500 cpu beyond avail 500 → free 1000 → smallest set is `low` (1000 cpu).
        assert_eq!(res.victims.len(), 1);
        assert_eq!(res.victims[0].name, "low");
    }

    #[test]
    fn preempt_does_not_evict_higher_priority() {
        let mut node = full_node("a");
        node.allocated.cpu_millicores = 4000;
        let snap = ClusterSnapshot {
            nodes: vec![node],
            pods_by_node: HashMap::from([("a".into(), vec![
                pod_at("t", "boss", 100, 4000, 8_000_000_000),
            ])]),
        };
        let preemptor = pod_at("t", "new", 50, 1000, 0);
        assert!(preempt(&preemptor, &snap, &[]).is_none());
    }

    #[test]
    fn preempt_respects_tenant_isolation() {
        let mut node = full_node("a");
        node.allocated.cpu_millicores = 4000;
        let snap = ClusterSnapshot {
            nodes: vec![node],
            pods_by_node: HashMap::from([("a".into(), vec![
                pod_at("other", "low", 0, 4000, 8_000_000_000),
            ])]),
        };
        let preemptor = pod_at("t", "new", 100, 1000, 0);
        assert!(preempt(&preemptor, &snap, &[]).is_none(),
            "cross-tenant preemption forbidden");
    }

    #[test]
    fn preempt_skips_pdb_violators() {
        let mut node = full_node("a");
        node.allocated.cpu_millicores = 4000;
        let v1 = pod_at("t", "v1", 0, 2000, 4_000_000_000);
        let mut v1l = v1.clone(); v1l.spec.node_selector.insert("app".into(), "db".into());
        let v2 = pod_at("t", "v2", 0, 2000, 4_000_000_000);
        let snap = ClusterSnapshot {
            nodes: vec![node],
            pods_by_node: HashMap::from([("a".into(), vec![v1l, v2])]),
        };
        let pdb = PodDisruptionBudget {
            name: "db-pdb".into(), namespace: "ns".into(), tenant_id: "t".into(),
            selector: HashMap::from([("app".into(), "db".into())]),
            min_available: 1, current_healthy: 1,
        };
        let preemptor = pod_at("t", "new", 50, 2000, 4_000_000_000);
        let res = preempt(&preemptor, &snap, &[pdb]).expect("should still find a victim");
        // v1 is PDB-protected; only v2 should be evicted.
        assert_eq!(res.victims.len(), 1);
        assert_eq!(res.victims[0].name, "v2");
        assert_eq!(res.pdb_violations, 1);
    }

    #[test]
    fn preempt_sets_nominated_node_name() {
        let mut a = full_node("a"); a.allocated.cpu_millicores = 4000;
        let mut b = full_node("b"); b.allocated.cpu_millicores = 4000;
        let snap = ClusterSnapshot {
            nodes: vec![a, b],
            pods_by_node: HashMap::from([
                ("a".into(), vec![pod_at("t", "x", 0, 1000, 0), pod_at("t", "y", 0, 1000, 0), pod_at("t", "z", 0, 1000, 0)]),
                ("b".into(), vec![pod_at("t", "single", 0, 1000, 0)]),
            ]),
        };
        let preemptor = pod_at("t", "new", 50, 500, 0);
        let res = preempt(&preemptor, &snap, &[]).expect("should preempt");
        // Both nodes can fit with 1 victim → tie-broken by name.
        assert_eq!(res.nominated_node_name, "a");
        assert_eq!(res.victims.len(), 1);
    }
}
