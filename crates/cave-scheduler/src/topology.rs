// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Pod topology spread constraints — spread pods evenly across topology domains
//! (zones, hostnames). Implements maxSkew with DoNotSchedule and ScheduleAnyway, plus
//! minDomains (KEP-3094, GA in v1.30).
//!
//! Cite: kubernetes/kubernetes v1.36.0
//!   pkg/scheduler/framework/plugins/podtopologyspread/filtering.go
//!   pkg/scheduler/framework/plugins/podtopologyspread/scoring.go

use crate::framework::*;
use crate::models::{Node, NodeStatus, TaintEffect};
use std::collections::HashMap;

pub struct PodTopologySpread;

/// True iff `node` is eligible for this constraint under nodeAffinityPolicy
/// and nodeTaintsPolicy.
///
/// `nodeAffinityPolicy=Honor` (default) gates: the node must satisfy the
/// pod's nodeSelector and required nodeAffinity to participate in the skew
/// calculation. `nodeAffinityPolicy=Ignore` includes every node.
///
/// `nodeTaintsPolicy=Ignore` (default) includes tainted nodes. `Honor` skips
/// nodes carrying NoSchedule/NoExecute taints not tolerated by the pod.
fn node_eligible(c: &TopologySpreadConstraint, pod: &Pod, node: &Node) -> bool {
    if c.node_affinity_policy == NodeInclusionPolicy::Honor {
        // Note: cave-scheduler's `pod.spec.node_selector` doubles as the pod's
        // label set (a documented test-surface proxy in this codebase), so we
        // do *not* check it here as a node selector — that would cross the
        // wires. Only the explicit nodeAffinity.required check is honored.
        if let Some(aff) = &pod.spec.node_affinity {
            if !aff.required.is_empty() {
                let any = aff.required.iter().any(|t| crate::plugins::node_selector_term_matches(t, node));
                if !any { return false; }
            }
        }
    }
    if c.node_taints_policy == NodeInclusionPolicy::Honor {
        for taint in &node.taints {
            if !matches!(taint.effect, TaintEffect::NoSchedule | TaintEffect::NoExecute) {
                continue;
            }
            let tolerated = pod.spec.tolerations.iter().any(|t| {
                let key_ok = match (t.operator.as_str(), t.key.as_deref()) {
                    ("Exists", None) => true,
                    ("Exists", Some(k)) => k == taint.key,
                    ("Equal", Some(k)) => k == taint.key && t.value.as_deref() == taint.value.as_deref(),
                    _ => false,
                };
                let effect_ok = t.effect.is_none() || t.effect.as_ref() == Some(&taint.effect);
                key_ok && effect_ok
            });
            if !tolerated { return false; }
        }
    }
    true
}

/// Effective selector for one constraint: literal `label_selector` plus
/// values pulled from the *scheduling pod's* labels for every key listed in
/// `match_label_keys` (KEP-3243 GA in v1.31).
fn effective_selector(c: &TopologySpreadConstraint, pod: &Pod) -> HashMap<String, String> {
    let mut sel = c.label_selector.clone();
    for k in &c.match_label_keys {
        if let Some(v) = pod_labels(pod).get(k) {
            sel.insert(k.clone(), v.clone());
        }
    }
    sel
}

/// Pod labels — proxied through `spec.node_selector` for backwards
/// compatibility with the existing test surface (cave-scheduler treats
/// `spec.node_selector` as the pod's label set in plugin tests).
fn pod_labels(pod: &Pod) -> &HashMap<String, String> {
    &pod.spec.node_selector
}

/// Per-constraint domain map: topology_value → match-count of selector-matching pods
/// already on nodes carrying that topology value, restricted to eligible nodes.
fn domain_counts<'a>(
    constraint: &TopologySpreadConstraint,
    snapshot: &'a ClusterSnapshot,
    pod: &Pod,
) -> HashMap<String, usize> {
    let sel = effective_selector(constraint, pod);
    let mut counts: HashMap<String, usize> = HashMap::new();
    for n in &snapshot.nodes {
        if !node_eligible(constraint, pod, n) { continue; }
        if n.status == NodeStatus::NotReady { continue; }
        if let Some(v) = n.labels.get(&constraint.topology_key) {
            counts.entry(v.clone()).or_insert(0);
        }
    }
    for n in &snapshot.nodes {
        if !node_eligible(constraint, pod, n) { continue; }
        let Some(topo_v) = n.labels.get(&constraint.topology_key) else { continue; };
        let entry = counts.entry(topo_v.clone()).or_insert(0);
        for p in snapshot.pods_on(&n.name) {
            if p.namespace != pod.namespace { continue; }
            let matches = sel.iter().all(|(k, v)| {
                pod_labels(p).get(k) == Some(v)
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
            let counts = domain_counts(c, snap, pod);
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
            let counts = domain_counts(c, snap, pod);
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
        TopologySpreadConstraint { max_skew, topology_key: "zone".into(), when_unsatisfiable: action, label_selector: sel, min_domains, ..Default::default() }
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

    // ── matchLabelKeys (KEP-3243) ────────────────────────────────────────

    #[test]
    fn match_label_keys_lifts_pod_label_into_selector() {
        let a = n("a", "z1"); let b = n("b", "z2");
        let mut snap = ClusterSnapshot { nodes: vec![a.clone(), b.clone()], pods_by_node: HashMap::new() };
        // Existing pods on z1: one with rev=1, one with rev=2. Selector
        // base: app=web. With matchLabelKeys=[rev], scheduling pod's rev=1
        // value lifts into selector → only the rev=1 pod counts.
        let mut p1 = web_pod("t", "p1"); p1.spec.node_selector.insert("rev".into(), "1".into());
        let mut p2 = web_pod("t", "p2"); p2.spec.node_selector.insert("rev".into(), "2".into());
        let mut p3 = web_pod("t", "p3"); p3.spec.node_selector.insert("rev".into(), "1".into());
        snap.pods_by_node.insert("a".into(), vec![p1, p2]);
        snap.pods_by_node.insert("b".into(), vec![p3]);

        let mut sched_pod = web_pod("t", "newp");
        sched_pod.spec.node_selector.insert("rev".into(), "1".into());
        let mut c = spread(0, UnsatisfiableAction::DoNotSchedule, None);
        c.match_label_keys.push("rev".into());
        sched_pod.spec.topology_spread.push(c);
        // Effective counts (rev=1): z1=1, z2=1 → balanced. Place either side
        // → projected skew (2,1) - 1 = 1 > maxSkew 0 on whichever side.
        // We assert reject on a (after place: z1=2, z2=1, skew 1 > 0).
        assert!(PodTopologySpread.filter(&sched_pod, &a, &snap).is_rejected());
    }

    #[test]
    fn match_label_keys_missing_value_falls_back() {
        // If scheduling pod doesn't carry the lifted label, it just doesn't
        // narrow the selector — behaviour reduces to the literal selector.
        let a = n("a", "z1");
        let snap = ClusterSnapshot { nodes: vec![a.clone()], pods_by_node: HashMap::new() };
        let mut sched_pod = web_pod("t", "newp");
        let mut c = spread(0, UnsatisfiableAction::DoNotSchedule, None);
        c.match_label_keys.push("missing-key".into());
        sched_pod.spec.topology_spread.push(c);
        // No existing pods → counts={z1:0} → place → skew 0 → success.
        assert!(PodTopologySpread.filter(&sched_pod, &a, &snap).is_success());
    }

    // ── nodeAffinityPolicy ────────────────────────────────────────────────

    #[test]
    fn node_affinity_policy_honor_excludes_non_matching_nodes() {
        let mut a = n("a", "z1"); a.labels.insert("gpu".into(), "true".into());
        let b = n("b", "z2"); // no gpu label
        let mut snap = ClusterSnapshot { nodes: vec![a.clone(), b.clone()], pods_by_node: HashMap::new() };
        snap.pods_by_node.insert("a".into(), vec![web_pod("t", "p1")]);

        // Pod requires nodeAffinity gpu=true. Under Honor (default), only z1
        // participates in the skew calc; minDomains=2 forces one synthetic
        // empty domain → projected counts{z1:2, _:0}, skew 2 > maxSkew 0.
        let mut p = web_pod("t", "newp");
        p.spec.node_affinity = Some(NodeAffinitySpec {
            required: vec![NodeSelectorTerm {
                match_expressions: vec![NodeSelectorRequirement {
                    key: "gpu".into(), operator: NodeSelectorOp::In, values: vec!["true".into()],
                }],
            }],
            ..Default::default()
        });
        let c = spread(0, UnsatisfiableAction::DoNotSchedule, Some(2));
        p.spec.topology_spread.push(c);
        // a is in selector AND nodeAffinity-matched → counts{z1:1}; with minDomains=2
        // padded → projected (place on a) {z1:2, _:0}, skew 2 > 0.
        assert!(PodTopologySpread.filter(&p, &a, &snap).is_rejected());
    }

    #[test]
    fn node_affinity_policy_ignore_includes_all_nodes() {
        let mut a = n("a", "z1"); a.labels.insert("gpu".into(), "true".into());
        let b = n("b", "z2"); // no gpu label
        let mut snap = ClusterSnapshot { nodes: vec![a.clone(), b.clone()], pods_by_node: HashMap::new() };
        snap.pods_by_node.insert("a".into(), vec![web_pod("t", "p1")]);

        let mut p = web_pod("t", "newp");
        p.spec.node_affinity = Some(NodeAffinitySpec {
            required: vec![NodeSelectorTerm {
                match_expressions: vec![NodeSelectorRequirement {
                    key: "gpu".into(), operator: NodeSelectorOp::In, values: vec!["true".into()],
                }],
            }],
            ..Default::default()
        });
        let mut c = spread(1, UnsatisfiableAction::DoNotSchedule, None);
        c.node_affinity_policy = NodeInclusionPolicy::Ignore;
        p.spec.topology_spread.push(c);
        // Both zones counted → counts{z1:1, z2:0}; place on a → skew (2,0)=2 > 1.
        assert!(PodTopologySpread.filter(&p, &a, &snap).is_rejected());
        // Place on b → counts{z1:1, z2:1} skew 0 ≤ 1 → success.
        assert!(PodTopologySpread.filter(&p, &b, &snap).is_success());
    }

    // ── nodeTaintsPolicy ──────────────────────────────────────────────────

    #[test]
    fn node_taints_policy_honor_excludes_unsatisfied_taints() {
        let a = n("a", "z1");
        let mut b = n("b", "z2");
        b.taints.push(crate::models::Taint {
            key: "dedicated".into(), value: Some("gpu".into()),
            effect: crate::models::TaintEffect::NoSchedule,
        });
        let mut snap = ClusterSnapshot { nodes: vec![a.clone(), b.clone()], pods_by_node: HashMap::new() };
        snap.pods_by_node.insert("a".into(), vec![web_pod("t", "p1")]);

        // Pod doesn't tolerate dedicated taint. Honor → b excluded from skew.
        // minDomains=2 forces a synthetic empty so the skew calc sees imbalance.
        let mut p = web_pod("t", "newp");
        let mut c = spread(0, UnsatisfiableAction::DoNotSchedule, Some(2));
        c.node_taints_policy = NodeInclusionPolicy::Honor;
        p.spec.topology_spread.push(c);
        // counts ignoring b: {z1:1} padded to 2 → projected {z1:2, _:0}, skew 2 > 0.
        assert!(PodTopologySpread.filter(&p, &a, &snap).is_rejected());
    }

    #[test]
    fn node_taints_policy_ignore_default_includes_all() {
        let a = n("a", "z1");
        let mut b = n("b", "z2");
        b.taints.push(crate::models::Taint {
            key: "x".into(), value: None,
            effect: crate::models::TaintEffect::NoSchedule,
        });
        let mut snap = ClusterSnapshot { nodes: vec![a.clone(), b.clone()], pods_by_node: HashMap::new() };
        snap.pods_by_node.insert("a".into(), vec![web_pod("t", "p1")]);

        let mut p = web_pod("t", "newp");
        // Default node_taints_policy=Ignore — both zones counted.
        let c = spread(0, UnsatisfiableAction::DoNotSchedule, None);
        p.spec.topology_spread.push(c);
        // counts{z1:1, z2:0}; place on b → skew (1,1)=0 ≤ 0 → success.
        assert!(PodTopologySpread.filter(&p, &b, &snap).is_success());
    }

    // Helper module for trait-internal mutation.
    mod sched_pod_test_helpers {
        use crate::framework::TopologySpreadConstraint;
        pub fn set_match_label_keys(c: &mut TopologySpreadConstraint, keys: &[&str]) {
            c.match_label_keys = keys.iter().map(|s| s.to_string()).collect();
        }
    }
}
