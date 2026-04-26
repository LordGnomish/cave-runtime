//! Scheduling framework — Filter/Score plugin chain runtime.
//!
//! Cite: kubernetes/kubernetes v1.36.0
//!   pkg/scheduler/framework/interface.go
//!   pkg/scheduler/framework/runtime/framework.go
//!
//! A scheduling cycle runs in extension points:
//!   PreFilter → Filter → PostFilter → PreScore → Score → NormalizeScore → Reserve → Permit → Bind
//! cave-scheduler implements: Filter, PostFilter (preemption), Score with configurable weights.

use crate::models::{Node, NodeStatus, ResourceRequest};
use std::collections::HashMap;

/// Plugin status code — mirrors framework.Code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Code {
    Success,
    Unschedulable,
    UnschedulableAndUnresolvable,
    Error,
    Skip,
}

#[derive(Debug, Clone)]
pub struct Status {
    pub code: Code,
    pub reasons: Vec<String>,
    pub plugin: String,
}

impl Status {
    pub fn success(plugin: &str) -> Self {
        Self { code: Code::Success, reasons: vec![], plugin: plugin.into() }
    }
    pub fn unschedulable(plugin: &str, reason: impl Into<String>) -> Self {
        Self { code: Code::Unschedulable, reasons: vec![reason.into()], plugin: plugin.into() }
    }
    pub fn unresolvable(plugin: &str, reason: impl Into<String>) -> Self {
        Self { code: Code::UnschedulableAndUnresolvable, reasons: vec![reason.into()], plugin: plugin.into() }
    }
    pub fn is_success(&self) -> bool { self.code == Code::Success }
    pub fn is_skip(&self) -> bool { self.code == Code::Skip }
}

/// Pod descriptor used by the framework. Richer than the legacy ScheduleRequest;
/// every scheduling cycle carries tenant_id (multi-tenant invariant).
#[derive(Debug, Clone)]
pub struct Pod {
    pub name: String,
    pub namespace: String,
    pub tenant_id: String,
    pub uid: String,
    pub spec: PodSpec,
}

#[derive(Debug, Clone, Default)]
pub struct PodSpec {
    pub resources: ResourceRequest,
    pub node_selector: HashMap<String, String>,
    pub node_name: Option<String>,
    pub priority: i32,
    pub priority_class_name: Option<String>,
    pub tolerations: Vec<crate::models::Toleration>,
    pub node_affinity: Option<NodeAffinitySpec>,
    pub pod_affinity: Vec<PodAffinityTerm>,
    pub pod_anti_affinity: Vec<PodAffinityTerm>,
    pub topology_spread: Vec<TopologySpreadConstraint>,
    pub host_ports: Vec<HostPort>,
    pub volumes: Vec<VolumeSpec>,
    pub container_images: Vec<String>,
    pub resource_claims: Vec<ResourceClaimRef>,
    pub scheduler_name: String,
}

impl Pod {
    pub fn new(tenant_id: &str, namespace: &str, name: &str) -> Self {
        Self {
            name: name.into(),
            namespace: namespace.into(),
            tenant_id: tenant_id.into(),
            uid: format!("{}-{}-{}", tenant_id, namespace, name),
            spec: PodSpec::default(),
        }
    }
}

/// Subset of NodeAffinity: required nodeSelectorTerms (DNF). Each term is a list of
/// matchExpressions ANDed; terms are ORed. (Cite: api/core/v1/types.go NodeSelectorTerm.)
#[derive(Debug, Clone, Default)]
pub struct NodeAffinitySpec {
    pub required: Vec<NodeSelectorTerm>,
}

#[derive(Debug, Clone, Default)]
pub struct NodeSelectorTerm {
    pub match_expressions: Vec<NodeSelectorRequirement>,
}

#[derive(Debug, Clone)]
pub struct NodeSelectorRequirement {
    pub key: String,
    pub operator: NodeSelectorOp,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeSelectorOp {
    In,
    NotIn,
    Exists,
    DoesNotExist,
    Gt,
    Lt,
}

/// Inter-pod affinity term — selects pods on a topology key.
#[derive(Debug, Clone)]
pub struct PodAffinityTerm {
    pub label_selector: HashMap<String, String>,
    pub topology_key: String,
    pub namespaces: Vec<String>,
}

/// Pod topology spread constraint (KEP-3094 minDomains GA v1.30).
#[derive(Debug, Clone)]
pub struct TopologySpreadConstraint {
    pub max_skew: i32,
    pub topology_key: String,
    pub when_unsatisfiable: UnsatisfiableAction,
    pub label_selector: HashMap<String, String>,
    pub min_domains: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnsatisfiableAction {
    DoNotSchedule,
    ScheduleAnyway,
}

#[derive(Debug, Clone)]
pub struct HostPort {
    pub host_ip: String,
    pub port: u16,
    pub protocol: String,
}

#[derive(Debug, Clone)]
pub struct VolumeSpec {
    pub name: String,
    pub kind: VolumeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VolumeKind {
    EBS { volume_id: String },
    GCEPD { pd_name: String },
    AzureDisk { disk_name: String },
    HostPath { path: String },
    PersistentVolumeClaim { claim_name: String, bound_node: Option<String> },
}

#[derive(Debug, Clone)]
pub struct ResourceClaimRef {
    pub name: String,
    pub claim_name: String,
}

/// Filter plugin — return Success or Unschedulable for each (pod, node) pair.
pub trait FilterPlugin: Send + Sync {
    fn name(&self) -> &str;
    fn filter(&self, pod: &Pod, node: &Node, snapshot: &ClusterSnapshot) -> Status;
}

/// Score plugin — return integer score in [0, MAX_NODE_SCORE]; 0 = worst, 100 = best.
pub trait ScorePlugin: Send + Sync {
    fn name(&self) -> &str;
    fn score(&self, pod: &Pod, node: &Node, snapshot: &ClusterSnapshot) -> i64;
}

pub const MAX_NODE_SCORE: i64 = 100;

/// Snapshot of the cluster used during a scheduling cycle.
#[derive(Debug, Default)]
pub struct ClusterSnapshot {
    pub nodes: Vec<Node>,
    pub pods_by_node: HashMap<String, Vec<Pod>>,
}

impl ClusterSnapshot {
    pub fn pods_on(&self, node_name: &str) -> &[Pod] {
        self.pods_by_node.get(node_name).map(|v| v.as_slice()).unwrap_or(&[])
    }
}

/// Per-plugin score weight (1..=10). Defaults to 1.
#[derive(Debug, Clone)]
pub struct ScoringWeights(pub HashMap<String, u32>);

impl Default for ScoringWeights {
    fn default() -> Self { Self(HashMap::new()) }
}

impl ScoringWeights {
    pub fn weight_for(&self, plugin: &str) -> u32 { self.0.get(plugin).copied().unwrap_or(1) }
    pub fn set(&mut self, plugin: &str, w: u32) {
        assert!((1..=10).contains(&w), "scoring weight must be 1..=10");
        self.0.insert(plugin.into(), w);
    }
}

/// Framework runs the plugin chain across a single scheduling cycle.
pub struct Framework {
    pub filters: Vec<Box<dyn FilterPlugin>>,
    pub scores: Vec<Box<dyn ScorePlugin>>,
    pub weights: ScoringWeights,
}

impl Framework {
    pub fn new() -> Self { Self { filters: vec![], scores: vec![], weights: ScoringWeights::default() } }

    pub fn with_filter(mut self, p: Box<dyn FilterPlugin>) -> Self { self.filters.push(p); self }
    pub fn with_score(mut self, p: Box<dyn ScorePlugin>) -> Self { self.scores.push(p); self }
    pub fn with_weight(mut self, plugin: &str, w: u32) -> Self { self.weights.set(plugin, w); self }

    /// Run all filter plugins. Node passes only when EVERY plugin returns Success.
    /// Returns map of node_name → first failing Status (None if it passed all).
    pub fn run_filters(&self, pod: &Pod, snapshot: &ClusterSnapshot) -> HashMap<String, Option<Status>> {
        let mut out: HashMap<String, Option<Status>> = HashMap::new();
        for node in &snapshot.nodes {
            if node.status != NodeStatus::Ready && node.status != NodeStatus::Cordoned {
                out.insert(node.name.clone(), Some(Status::unschedulable("framework", "node not ready")));
                continue;
            }
            let mut fail: Option<Status> = None;
            for f in &self.filters {
                let s = f.filter(pod, node, snapshot);
                if !s.is_success() && !s.is_skip() {
                    fail = Some(s);
                    break;
                }
            }
            out.insert(node.name.clone(), fail);
        }
        out
    }

    /// Run all score plugins on a candidate node list. Final node score is the
    /// weighted sum of plugin scores: Σ weight(p) * score_p(node).
    pub fn run_scores(&self, pod: &Pod, candidates: &[String], snapshot: &ClusterSnapshot) -> HashMap<String, i64> {
        let mut totals: HashMap<String, i64> = HashMap::new();
        for cand in candidates {
            let Some(node) = snapshot.nodes.iter().find(|n| &n.name == cand) else { continue };
            let mut total: i64 = 0;
            for s in &self.scores {
                let raw = s.score(pod, node, snapshot).clamp(0, MAX_NODE_SCORE);
                let w = self.weights.weight_for(s.name()) as i64;
                total += raw * w;
            }
            totals.insert(cand.clone(), total);
        }
        totals
    }

    /// Convenience: filter then score, returning the chosen node (highest score, deterministic by name).
    pub fn schedule_one(&self, pod: &Pod, snapshot: &ClusterSnapshot) -> Result<String, Status> {
        let filtered = self.run_filters(pod, snapshot);
        let candidates: Vec<String> = filtered.iter()
            .filter(|(_, v)| v.is_none())
            .map(|(k, _)| k.clone())
            .collect();
        if candidates.is_empty() {
            return Err(Status::unschedulable("framework", "no nodes passed filter chain"));
        }
        let scores = self.run_scores(pod, &candidates, snapshot);
        let (winner, _) = scores.iter()
            .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0))) // deterministic: higher score, lower name first on ties
            .map(|(k, v)| (k.clone(), *v))
            .unwrap_or((candidates[0].clone(), 0));
        Ok(winner)
    }
}

impl Default for Framework {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ResourceCapacity;
    use chrono::Utc;
    use uuid::Uuid;

    fn ready_node(name: &str) -> Node {
        Node {
            name: name.into(), uid: Uuid::new_v4(), status: NodeStatus::Ready,
            capacity: ResourceCapacity { cpu_millicores: 4000, memory_bytes: 8_000_000_000, pods: 110, ephemeral_storage_bytes: 0 },
            allocatable: ResourceCapacity { cpu_millicores: 4000, memory_bytes: 8_000_000_000, pods: 110, ephemeral_storage_bytes: 0 },
            allocated: ResourceCapacity::default(),
            labels: HashMap::new(), taints: vec![], conditions: vec![],
            registered_at: Utc::now(), last_heartbeat: Utc::now(),
        }
    }

    struct AlwaysPass;
    impl FilterPlugin for AlwaysPass {
        fn name(&self) -> &str { "AlwaysPass" }
        fn filter(&self, _: &Pod, _: &Node, _: &ClusterSnapshot) -> Status { Status::success("AlwaysPass") }
    }

    struct RejectByName(&'static str);
    impl FilterPlugin for RejectByName {
        fn name(&self) -> &str { "RejectByName" }
        fn filter(&self, _: &Pod, n: &Node, _: &ClusterSnapshot) -> Status {
            if n.name == self.0 { Status::unschedulable("RejectByName", "rejected") } else { Status::success("RejectByName") }
        }
    }

    struct ConstScore(i64);
    impl ScorePlugin for ConstScore {
        fn name(&self) -> &str { "ConstScore" }
        fn score(&self, _: &Pod, _: &Node, _: &ClusterSnapshot) -> i64 { self.0 }
    }

    #[test]
    fn pod_carries_tenant_id() {
        let pod = Pod::new("acme", "default", "web");
        assert_eq!(pod.tenant_id, "acme");
        assert_eq!(pod.namespace, "default");
        assert_eq!(pod.uid, "acme-default-web");
    }

    #[test]
    fn framework_filter_chain_passes_when_all_succeed() {
        let snap = ClusterSnapshot { nodes: vec![ready_node("a"), ready_node("b")], pods_by_node: HashMap::new() };
        let fw = Framework::new().with_filter(Box::new(AlwaysPass));
        let pod = Pod::new("t1", "ns", "p");
        let res = fw.run_filters(&pod, &snap);
        assert!(res.get("a").unwrap().is_none());
        assert!(res.get("b").unwrap().is_none());
    }

    #[test]
    fn framework_filter_chain_short_circuits_on_first_fail() {
        let snap = ClusterSnapshot { nodes: vec![ready_node("a"), ready_node("b")], pods_by_node: HashMap::new() };
        let fw = Framework::new()
            .with_filter(Box::new(AlwaysPass))
            .with_filter(Box::new(RejectByName("a")));
        let pod = Pod::new("t1", "ns", "p");
        let res = fw.run_filters(&pod, &snap);
        assert!(res.get("a").unwrap().is_some());
        assert_eq!(res.get("a").unwrap().as_ref().unwrap().plugin, "RejectByName");
        assert!(res.get("b").unwrap().is_none());
    }

    #[test]
    fn scoring_weights_clamp_and_sum() {
        let snap = ClusterSnapshot { nodes: vec![ready_node("a")], pods_by_node: HashMap::new() };
        let fw = Framework::new()
            .with_score(Box::new(ConstScore(50)))
            .with_score(Box::new(ConstScore(30)))
            .with_weight("ConstScore", 2);
        let pod = Pod::new("t1", "ns", "p");
        let scores = fw.run_scores(&pod, &["a".into()], &snap);
        // Both score plugins share name "ConstScore" → both get weight 2: (50+30)*2 = 160
        assert_eq!(scores["a"], 160);
    }

    #[test]
    fn scoring_clamps_above_max() {
        let snap = ClusterSnapshot { nodes: vec![ready_node("a")], pods_by_node: HashMap::new() };
        let fw = Framework::new().with_score(Box::new(ConstScore(9999)));
        let pod = Pod::new("t1", "ns", "p");
        let scores = fw.run_scores(&pod, &["a".into()], &snap);
        assert_eq!(scores["a"], MAX_NODE_SCORE);
    }

    #[test]
    fn schedule_one_picks_highest_score() {
        let mut a = ready_node("a"); a.allocated.cpu_millicores = 1000;
        let mut b = ready_node("b"); b.allocated.cpu_millicores = 100;
        let snap = ClusterSnapshot { nodes: vec![a, b], pods_by_node: HashMap::new() };
        struct ByName;
        impl ScorePlugin for ByName {
            fn name(&self) -> &str { "ByName" }
            fn score(&self, _: &Pod, n: &Node, _: &ClusterSnapshot) -> i64 { if n.name == "b" { 90 } else { 10 } }
        }
        let fw = Framework::new().with_score(Box::new(ByName));
        let pod = Pod::new("t", "ns", "p");
        assert_eq!(fw.schedule_one(&pod, &snap).unwrap(), "b");
    }

    #[test]
    #[should_panic(expected = "scoring weight must be 1..=10")]
    fn scoring_weight_out_of_range_panics() {
        let mut w = ScoringWeights::default();
        w.set("X", 11);
    }

    #[test]
    fn status_constructors() {
        assert!(Status::success("p").is_success());
        let s = Status::unschedulable("p", "x");
        assert_eq!(s.code, Code::Unschedulable);
        assert_eq!(s.reasons, vec!["x"]);
        let s2 = Status::unresolvable("p", "y");
        assert_eq!(s2.code, Code::UnschedulableAndUnresolvable);
    }
}
