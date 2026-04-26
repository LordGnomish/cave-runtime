//! Pod topology spread constraints — spread pods evenly across topology domains
//! (zones, hostnames). Implements maxSkew with DoNotSchedule and ScheduleAnyway, plus
//! minDomains (KEP-3094, GA in v1.30).
//!
//! Cite: kubernetes/kubernetes v1.36.0
//!   pkg/scheduler/framework/plugins/podtopologyspread/filtering.go
//!   pkg/scheduler/framework/plugins/podtopologyspread/scoring.go

use crate::framework::*;
use crate::models::Node;
use std::collections::HashMap;

pub struct PodTopologySpread;

/// Per-constraint domain map: topology_value → match-count of selector-matching pods
/// already on nodes carrying that topology value.
fn domain_counts<'a>(
    constraint: &TopologySpreadConstraint,
    snapshot: &'a ClusterSnapshot,
    pod_namespace: &str,
) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    // Initialize all known domains to 0 so empty domains contribute to skew calc.
    for n in &snapshot.nodes {
        if let Some(v) = n.labels.get(&constraint.topology_key) {
            counts.entry(v.clone()).or_insert(0);
        }
    }
    for n in &snapshot.nodes {
        let Some(topo_v) = n.labels.get(&constraint.topology_key) else { continue; };
        let entry = counts.entry(topo_v.clone()).or_insert(0);
        for p in snapshot.pods_on(&n.name) {
            if p.namespace != pod_namespace { continue; }
            let matches = constraint.label_selector.iter().all(|(k, v)| {
                p.spec.node_selector.get(k) == Some(v)
            });
            if matches { *entry += 1; }
        }
    }
    counts
}

/// Effective domain count, padding with empty (0-count) domains up to minDomains.
fn effective_counts(
    raw: &HashMap<String, usize>,
    min_domains: Option<i32>,
) -> Vec<usize> {
    let mut vals: Vec<usize> = raw.values().copied().collect();
    if let Some(min_d) = min_domains {
        let pad = (min_d as usize).saturating_sub(vals.len());
        for _ in 0..pad { vals.push(0); }
    }
    vals
}

/// Compute skew that would result from placing the pod on `target_topology_value`.
fn projected_skew(
    raw: &HashMap<String, usize>,
    min_domains: Option<i32>,
    target: &str,
) -> i32 {
    let mut projected: HashMap<String, usize> = raw.clone();
    *projected.entry(target.to_string()).or_insert(0) += 1;
    let vals = effective_counts(&projected, min_domains);
    if vals.is_empty() { return 0; }
    let max = *vals.iter().max().unwrap();
    let min = *vals.iter().min().unwrap();
    (max - min) as i32
}

impl FilterPlugin for PodTopologySpread {
    fn name(&self) -> &str { "PodTopologySpread" }
    fn filter(&self, pod: &Pod, node: &Node, snap: &ClusterSnapshot) -> Status {
        for c in &pod.spec.topology_spread {
            if c.when_unsatisfiable != UnsatisfiableAction::DoNotSchedule { continue; }
            let Some(target) = node.labels.get(&c.topology_key) else {
                return Status::unschedulable("PodTopologySpread", format!("node lacks topology key {}", c.topology_key));
            };
            let counts = domain_counts(c, snap, &pod.namespace);
            let skew = projected_skew(&counts, c.min_domains, target);
            if skew > c.max_skew {
                return Status::unschedulable("PodTopologySpread",
                    format!("placing on {}={} yields skew {} > maxSkew {}", c.topology_key, target, skew, c.max_skew));
            }
        }
        Status::success("PodTopologySpread")
    }
}

impl ScorePlugin for PodTopologySpread {
    fn name(&self) -> &str { "PodTopologySpread" }
    fn score(&self, pod: &Pod, node: &Node, snap: &ClusterSnapshot) -> i64 {
        // For ScheduleAnyway constraints, prefer nodes that minimize skew.
        // Score: MAX_NODE_SCORE - sum of projected skews across constraints (clamped).
        let mut penalty: i32 = 0;
        let mut active = 0;
        for c in &pod.spec.topology_spread {
            if c.when_unsatisfiable != UnsatisfiableAction::ScheduleAnyway { continue; }
            let Some(target) = node.labels.get(&c.topology_key) else { continue; };
            let counts = domain_counts(c, snap, &pod.namespace);
            penalty += projected_skew(&counts, c.min_domains, target);
            active += 1;
        }
        if active == 0 { return MAX_NODE_SCORE; }
        let s = (MAX_NODE_SCORE - penalty as i64).max(0);
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{NodeStatus, ResourceCapacity};
    use chrono::Utc;
    use uuid::Uuid;

    fn n(name: &str, zone: &str) -> Node {
        let mut node = Node {
            name: name.into(), uid: Uuid::new_v4(), status: NodeStatus::Ready,
            capacity: ResourceCapacity { cpu_millicores: 4000, memory_bytes: 8_000_000_000, pods: 110, ephemeral_storage_bytes: 0 },
            allocatable: ResourceCapacity { cpu_millicores: 4000, memory_bytes: 8_000_000_000, pods: 110, ephemeral_storage_bytes: 0 },
            allocated: ResourceCapacity::default(),
            labels: HashMap::new(), taints: vec![], conditions: vec![],
            registered_at: Utc::now(), last_heartbeat: Utc::now(),
        };
        node.labels.insert("zone".into(), zone.into());
        node
    }

    fn web_pod(tenant: &str, name: &str) -> Pod {
        let mut p = Pod::new(tenant, "ns", name);
        p.spec.node_selector.insert("app".into(), "web".into());
        p
    }

    fn spread(max_skew: i32, action: UnsatisfiableAction, min_domains: Option<i32>) -> TopologySpreadConstraint {
        let mut sel = HashMap::new(); sel.insert("app".into(), "web".into());
        TopologySpreadConstraint { max_skew, topology_key: "zone".into(), when_unsatisfiable: action, label_selector: sel, min_domains }
    }

    #[test]
    fn do_not_schedule_blocks_when_skew_exceeded() {
        let a = n("a", "z1"); let b = n("b", "z2");
        let mut snap = ClusterSnapshot { nodes: vec![a.clone(), b.clone()], pods_by_node: HashMap::new() };
        snap.pods_by_node.insert("a".into(), vec![web_pod("t", "p1"), web_pod("t", "p2")]);
        // z1 has 2, z2 has 0 → placing on z1 → skew 3-0 = 3 > maxSkew 1.
        let mut p = web_pod("t", "newp");
        p.spec.topology_spread.push(spread(1, UnsatisfiableAction::DoNotSchedule, None));
        assert_eq!(PodTopologySpread.filter(&p, &a, &snap).code, Code::Unschedulable);
        // Placing on z2 → skew 2-1 = 1 ≤ 1 → OK.
        assert!(PodTopologySpread.filter(&p, &b, &snap).is_success());
    }

    #[test]
    fn schedule_anyway_does_not_filter() {
        let a = n("a", "z1"); let b = n("b", "z2");
        let mut snap = ClusterSnapshot { nodes: vec![a.clone(), b.clone()], pods_by_node: HashMap::new() };
        snap.pods_by_node.insert("a".into(), vec![web_pod("t", "p1"), web_pod("t", "p2")]);
        let mut p = web_pod("t", "newp");
        p.spec.topology_spread.push(spread(1, UnsatisfiableAction::ScheduleAnyway, None));
        assert!(PodTopologySpread.filter(&p, &a, &snap).is_success());
    }

    #[test]
    fn schedule_anyway_score_prefers_low_skew_node() {
        let a = n("a", "z1"); let b = n("b", "z2");
        let mut snap = ClusterSnapshot { nodes: vec![a.clone(), b.clone()], pods_by_node: HashMap::new() };
        snap.pods_by_node.insert("a".into(), vec![web_pod("t", "p1"), web_pod("t", "p2")]);
        let mut p = web_pod("t", "newp");
        p.spec.topology_spread.push(spread(1, UnsatisfiableAction::ScheduleAnyway, None));
        let sa = PodTopologySpread.score(&p, &a, &snap);
        let sb = PodTopologySpread.score(&p, &b, &snap);
        assert!(sb > sa, "z2 (rebalances) should outscore z1");
    }

    #[test]
    fn min_domains_pads_with_empty_domains() {
        // Only one domain known → minDomains=3 forces two synthetic empty domains,
        // making skew calc count placement against 3 domains, not 1.
        let a = n("a", "z1");
        let mut snap = ClusterSnapshot { nodes: vec![a.clone()], pods_by_node: HashMap::new() };
        snap.pods_by_node.insert("a".into(), vec![web_pod("t", "p1")]);
        let mut p = web_pod("t", "newp");
        p.spec.topology_spread.push(spread(1, UnsatisfiableAction::DoNotSchedule, Some(3)));
        // Placing on z1 → counts {z1:2, _:0, _:0} → skew 2 > 1 → reject.
        assert_eq!(PodTopologySpread.filter(&p, &a, &snap).code, Code::Unschedulable);
    }

    #[test]
    fn missing_topology_key_on_node_rejected() {
        let mut a = n("a", "z1"); a.labels.remove("zone");
        let snap = ClusterSnapshot { nodes: vec![a.clone()], pods_by_node: HashMap::new() };
        let mut p = web_pod("t", "newp");
        p.spec.topology_spread.push(spread(1, UnsatisfiableAction::DoNotSchedule, None));
        assert_eq!(PodTopologySpread.filter(&p, &a, &snap).code, Code::Unschedulable);
    }
}
