// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
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
///
/// `Wait` and `Pending` were added when the framework grew Permit and PreEnqueue
/// extension points; existing `Filter` plugins never produce them. They are
/// distinct enough from `Unschedulable` that callers must match explicitly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Code {
    Success,
    Unschedulable,
    UnschedulableAndUnresolvable,
    Error,
    Skip,
    /// Permit plugin asks the framework to delay Bind for up to `wait_duration`.
    Wait,
    /// PreEnqueue plugin says the pod is not yet ready to enter the active queue
    /// (e.g. SchedulingGates). The pod stays in the unschedulable subqueue until
    /// a relevant cluster event re-enqueues it.
    Pending,
}

#[derive(Debug, Clone)]
pub struct Status {
    pub code: Code,
    pub reasons: Vec<String>,
    pub plugin: String,
    /// Set when `code == Wait`. Permit plugins' max wait wins at the framework level.
    pub wait_duration: Option<chrono::Duration>,
    /// On rejection, the plugin whose Filter/PreFilter ultimately rejected the pod.
    /// Filled by the framework from the `plugin` field at first non-Success.
    pub failed_plugin: Option<String>,
}

impl Status {
    pub fn success(plugin: &str) -> Self {
        Self {
            code: Code::Success,
            reasons: vec![],
            plugin: plugin.into(),
            wait_duration: None,
            failed_plugin: None,
        }
    }
    pub fn unschedulable(plugin: &str, reason: impl Into<String>) -> Self {
        Self {
            code: Code::Unschedulable,
            reasons: vec![reason.into()],
            plugin: plugin.into(),
            wait_duration: None,
            failed_plugin: Some(plugin.into()),
        }
    }
    pub fn unresolvable(plugin: &str, reason: impl Into<String>) -> Self {
        Self {
            code: Code::UnschedulableAndUnresolvable,
            reasons: vec![reason.into()],
            plugin: plugin.into(),
            wait_duration: None,
            failed_plugin: Some(plugin.into()),
        }
    }
    /// Permit plugin asks scheduler to wait up to `dur` before binding.
    pub fn wait(plugin: &str, reason: impl Into<String>, dur: chrono::Duration) -> Self {
        Self {
            code: Code::Wait,
            reasons: vec![reason.into()],
            plugin: plugin.into(),
            wait_duration: Some(dur),
            failed_plugin: None,
        }
    }
    /// PreEnqueue plugin keeps the pod in the unschedulable subqueue.
    pub fn pending(plugin: &str, reason: impl Into<String>) -> Self {
        Self {
            code: Code::Pending,
            reasons: vec![reason.into()],
            plugin: plugin.into(),
            wait_duration: None,
            failed_plugin: None,
        }
    }
    /// Internal error — plugin author bug or transient infra failure.
    pub fn error(plugin: &str, reason: impl Into<String>) -> Self {
        Self {
            code: Code::Error,
            reasons: vec![reason.into()],
            plugin: plugin.into(),
            wait_duration: None,
            failed_plugin: None,
        }
    }
    /// PreFilter/PreScore signals the matching Filter/Score plugin should be skipped.
    pub fn skip(plugin: &str) -> Self {
        Self {
            code: Code::Skip,
            reasons: vec![],
            plugin: plugin.into(),
            wait_duration: None,
            failed_plugin: None,
        }
    }
    pub fn is_success(&self) -> bool {
        self.code == Code::Success
    }
    pub fn is_skip(&self) -> bool {
        self.code == Code::Skip
    }
    pub fn is_wait(&self) -> bool {
        self.code == Code::Wait
    }
    pub fn is_pending(&self) -> bool {
        self.code == Code::Pending
    }
    pub fn is_error(&self) -> bool {
        self.code == Code::Error
    }
    /// True when the node was rejected (Unschedulable or UnschedulableAndUnresolvable).
    pub fn is_rejected(&self) -> bool {
        matches!(
            self.code,
            Code::Unschedulable | Code::UnschedulableAndUnresolvable
        )
    }
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
    /// Scheduling gates (KEP-3521 GA in v1.30) — controllers add named gates
    /// while the pod is being prepared; SchedulingGates PreEnqueue keeps the
    /// pod in the unschedulable subqueue until every gate is removed.
    pub scheduling_gates: Vec<String>,
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

/// Subset of NodeAffinity: required nodeSelectorTerms (DNF) plus preferred
/// (weighted) terms. Required: each term is a list of matchExpressions ANDed;
/// terms are ORed. Preferred: each term is weighted [1..=100] and contributes
/// to the score when matched. (Cite: api/core/v1/types.go NodeSelectorTerm.)
#[derive(Debug, Clone, Default)]
pub struct NodeAffinitySpec {
    pub required: Vec<NodeSelectorTerm>,
    /// Preferred (soft) terms — weighted, used by the Score plugin only.
    #[allow(dead_code)]
    pub preferred: Vec<PreferredSchedulingTerm>,
}

#[derive(Debug, Clone)]
pub struct PreferredSchedulingTerm {
    pub weight: i32,
    pub preference: NodeSelectorTerm,
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

/// LabelSelector — matchLabels (AND) plus matchExpressions (AND). All entries
/// are ANDed across both halves. Empty selector matches everything (upstream
/// invariant: nil selector → match all; we encode that as `Default::default()`).
#[derive(Debug, Clone, Default)]
pub struct LabelSelector {
    pub match_labels: HashMap<String, String>,
    pub match_expressions: Vec<LabelSelectorRequirement>,
}

#[derive(Debug, Clone)]
pub struct LabelSelectorRequirement {
    pub key: String,
    pub operator: LabelSelectorOp,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabelSelectorOp {
    In,
    NotIn,
    Exists,
    DoesNotExist,
}

impl LabelSelector {
    /// True when every match_label and match_expression is satisfied by `labels`.
    pub fn matches(&self, labels: &HashMap<String, String>) -> bool {
        for (k, v) in &self.match_labels {
            if labels.get(k) != Some(v) {
                return false;
            }
        }
        for req in &self.match_expressions {
            let v = labels.get(&req.key);
            let ok = match req.operator {
                LabelSelectorOp::In => v.map_or(false, |x| req.values.iter().any(|w| w == x)),
                LabelSelectorOp::NotIn => v.map_or(true, |x| !req.values.iter().any(|w| w == x)),
                LabelSelectorOp::Exists => v.is_some(),
                LabelSelectorOp::DoesNotExist => v.is_none(),
            };
            if !ok {
                return false;
            }
        }
        true
    }

    pub fn from_match_labels(m: HashMap<String, String>) -> Self {
        Self {
            match_labels: m,
            match_expressions: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.match_labels.is_empty() && self.match_expressions.is_empty()
    }
}

/// Inter-pod affinity term — selects pods on a topology key.
///
/// Backwards-compatible: legacy `label_selector: HashMap` is preserved and
/// AND-merged with the optional richer `selector_v2` (which supports
/// matchExpressions). New optional fields default to "no constraint":
///
/// - `namespace_selector`: when `Some`, matches namespaces by label (in
///   addition to the literal `namespaces` list); `None` → only the literal list.
/// - `match_label_keys`: each name lifts the value from the *scheduling* pod's
///   labels into an additional matchLabels entry (KEP-3243).
/// - `mismatch_label_keys`: same but adds a `NotIn [value]` exclusion.
#[derive(Debug, Clone, Default)]
pub struct PodAffinityTerm {
    pub label_selector: HashMap<String, String>,
    pub topology_key: String,
    pub namespaces: Vec<String>,
    pub selector_v2: Option<LabelSelector>,
    pub namespace_selector: Option<LabelSelector>,
    pub match_label_keys: Vec<String>,
    pub mismatch_label_keys: Vec<String>,
}

/// Weighted soft pod-affinity term — used by the Score plugin only.
#[derive(Debug, Clone)]
pub struct WeightedPodAffinityTerm {
    pub weight: i32,
    pub term: PodAffinityTerm,
}

/// Pod topology spread constraint (KEP-3094 minDomains GA v1.30,
/// KEP-3243 matchLabelKeys GA v1.31, KEP-3094 nodeAffinity/TaintsPolicy GA v1.30).
#[derive(Debug, Clone, Default)]
pub struct TopologySpreadConstraint {
    pub max_skew: i32,
    pub topology_key: String,
    pub when_unsatisfiable: UnsatisfiableAction,
    pub label_selector: HashMap<String, String>,
    pub min_domains: Option<i32>,
    /// Each name lifts the value from the *scheduling* pod's labels into the
    /// effective selector when matching pre-existing pods (KEP-3243).
    pub match_label_keys: Vec<String>,
    /// `Honor` (default) — only nodes that satisfy nodeAffinity / nodeSelector
    /// participate in skew calc. `Ignore` — every node is considered.
    pub node_affinity_policy: NodeInclusionPolicy,
    /// `Ignore` (default) — tainted nodes still count. `Honor` — tainted nodes
    /// are excluded unless tolerated.
    pub node_taints_policy: NodeInclusionPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeInclusionPolicy {
    Honor,
    Ignore,
}

impl Default for NodeInclusionPolicy {
    fn default() -> Self {
        Self::Honor
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnsatisfiableAction {
    DoNotSchedule,
    ScheduleAnyway,
}

impl Default for UnsatisfiableAction {
    fn default() -> Self {
        Self::DoNotSchedule
    }
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
    EBS {
        volume_id: String,
    },
    GCEPD {
        pd_name: String,
    },
    AzureDisk {
        disk_name: String,
    },
    HostPath {
        path: String,
    },
    PersistentVolumeClaim {
        claim_name: String,
        bound_node: Option<String>,
    },
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
        self.pods_by_node
            .get(node_name)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }
}

/// Per-plugin score weight (1..=10). Defaults to 1.
#[derive(Debug, Clone)]
pub struct ScoringWeights(pub HashMap<String, u32>);

impl Default for ScoringWeights {
    fn default() -> Self {
        Self(HashMap::new())
    }
}

impl ScoringWeights {
    pub fn weight_for(&self, plugin: &str) -> u32 {
        self.0.get(plugin).copied().unwrap_or(1)
    }
    pub fn set(&mut self, plugin: &str, w: u32) {
        assert!((1..=10).contains(&w), "scoring weight must be 1..=10");
        self.0.insert(plugin.into(), w);
    }
}

/// Framework runs the plugin chain across a single scheduling cycle.
///
/// Plugins are registered per extension point. The `with_*` builders preserve
/// registration order — every `run_*` method walks plugins in that order.
pub struct Framework {
    pub filters: Vec<Box<dyn FilterPlugin>>,
    pub scores: Vec<Box<dyn ScorePlugin>>,
    pub weights: ScoringWeights,
    pub queue_sorts: Vec<Box<dyn crate::extension_points::QueueSortPlugin>>,
    pub pre_enqueues: Vec<Box<dyn crate::extension_points::PreEnqueuePlugin>>,
    pub pre_filters: Vec<Box<dyn crate::extension_points::PreFilterPlugin>>,
    pub post_filters: Vec<Box<dyn crate::extension_points::PostFilterPlugin>>,
    pub pre_scores: Vec<Box<dyn crate::extension_points::PreScorePlugin>>,
    pub reserves: Vec<Box<dyn crate::extension_points::ReservePlugin>>,
    pub permits: Vec<Box<dyn crate::extension_points::PermitPlugin>>,
    pub pre_binds: Vec<Box<dyn crate::extension_points::PreBindPlugin>>,
    pub binds: Vec<Box<dyn crate::extension_points::BindPlugin>>,
    pub post_binds: Vec<Box<dyn crate::extension_points::PostBindPlugin>>,
    /// `(plugin_name, extension)` map for NormalizeScore. Plugin name must
    /// match a registered Score plugin's name.
    pub score_extensions: HashMap<String, Box<dyn crate::extension_points::ScoreExtensions>>,
}

impl Framework {
    pub fn new() -> Self {
        Self {
            filters: vec![],
            scores: vec![],
            weights: ScoringWeights::default(),
            queue_sorts: vec![],
            pre_enqueues: vec![],
            pre_filters: vec![],
            post_filters: vec![],
            pre_scores: vec![],
            reserves: vec![],
            permits: vec![],
            pre_binds: vec![],
            binds: vec![],
            post_binds: vec![],
            score_extensions: HashMap::new(),
        }
    }

    pub fn with_filter(mut self, p: Box<dyn FilterPlugin>) -> Self {
        self.filters.push(p);
        self
    }
    pub fn with_score(mut self, p: Box<dyn ScorePlugin>) -> Self {
        self.scores.push(p);
        self
    }
    pub fn with_weight(mut self, plugin: &str, w: u32) -> Self {
        self.weights.set(plugin, w);
        self
    }
    pub fn with_queue_sort(mut self, p: Box<dyn crate::extension_points::QueueSortPlugin>) -> Self {
        self.queue_sorts.push(p);
        self
    }
    pub fn with_pre_enqueue(
        mut self,
        p: Box<dyn crate::extension_points::PreEnqueuePlugin>,
    ) -> Self {
        self.pre_enqueues.push(p);
        self
    }
    pub fn with_pre_filter(mut self, p: Box<dyn crate::extension_points::PreFilterPlugin>) -> Self {
        self.pre_filters.push(p);
        self
    }
    pub fn with_post_filter(
        mut self,
        p: Box<dyn crate::extension_points::PostFilterPlugin>,
    ) -> Self {
        self.post_filters.push(p);
        self
    }
    pub fn with_pre_score(mut self, p: Box<dyn crate::extension_points::PreScorePlugin>) -> Self {
        self.pre_scores.push(p);
        self
    }
    pub fn with_reserve(mut self, p: Box<dyn crate::extension_points::ReservePlugin>) -> Self {
        self.reserves.push(p);
        self
    }
    pub fn with_permit(mut self, p: Box<dyn crate::extension_points::PermitPlugin>) -> Self {
        self.permits.push(p);
        self
    }
    pub fn with_pre_bind(mut self, p: Box<dyn crate::extension_points::PreBindPlugin>) -> Self {
        self.pre_binds.push(p);
        self
    }
    pub fn with_bind(mut self, p: Box<dyn crate::extension_points::BindPlugin>) -> Self {
        self.binds.push(p);
        self
    }
    pub fn with_post_bind(mut self, p: Box<dyn crate::extension_points::PostBindPlugin>) -> Self {
        self.post_binds.push(p);
        self
    }
    pub fn with_score_extension(
        mut self,
        plugin_name: &str,
        ext: Box<dyn crate::extension_points::ScoreExtensions>,
    ) -> Self {
        self.score_extensions.insert(plugin_name.into(), ext);
        self
    }

    /// Run all filter plugins. Node passes only when EVERY plugin returns Success.
    /// Returns map of node_name → first failing Status (None if it passed all).
    pub fn run_filters(
        &self,
        pod: &Pod,
        snapshot: &ClusterSnapshot,
    ) -> HashMap<String, Option<Status>> {
        let mut out: HashMap<String, Option<Status>> = HashMap::new();
        for node in &snapshot.nodes {
            if node.status != NodeStatus::Ready && node.status != NodeStatus::Cordoned {
                out.insert(
                    node.name.clone(),
                    Some(Status::unschedulable("framework", "node not ready")),
                );
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

    /// Run every PreFilter plugin in order. First non-Success-non-Skip aborts.
    ///
    /// `Skip` records this plugin's matching Filter to be skipped this cycle
    /// (via [`CycleState::mark_filter_skipped`]). `PreFilterResult.node_names`
    /// from each plugin are intersected and returned.
    pub fn run_pre_filters(
        &self,
        pod: &Pod,
        snapshot: &ClusterSnapshot,
        state: &crate::cycle_state::CycleState,
    ) -> (crate::extension_points::PreFilterResult, Status) {
        use crate::extension_points::PreFilterResult;
        let mut merged = PreFilterResult::all_nodes();
        for p in &self.pre_filters {
            let (res, status) = p.pre_filter(pod, snapshot, state);
            if status.is_skip() {
                state.mark_filter_skipped(p.name());
                continue;
            }
            if !status.is_success() {
                return (merged, status);
            }
            merged = merged.merge(&res);
        }
        (merged, Status::success("framework"))
    }

    /// Run every PostFilter plugin in order. First plugin that returns a
    /// nominated node wins. If every plugin returns rejection, returns the
    /// last status — matching upstream "no preemption succeeded" behaviour.
    pub fn run_post_filters(
        &self,
        pod: &Pod,
        snapshot: &ClusterSnapshot,
        filtered: &crate::extension_points::NodeToStatusMap,
        state: &crate::cycle_state::CycleState,
    ) -> (crate::extension_points::PostFilterResult, Status) {
        use crate::extension_points::PostFilterResult;
        let mut last = (
            PostFilterResult::default(),
            Status::unschedulable("framework", "no PostFilter plugin nominated a node"),
        );
        for p in &self.post_filters {
            let (res, status) = p.post_filter(pod, snapshot, filtered, state);
            if status.is_success() && res.nominating_info.is_some() {
                return (res, status);
            }
            last = (res, status);
        }
        last
    }

    /// Run every PreScore plugin. `Skip` marks the matching Score plugin skipped.
    pub fn run_pre_scores(
        &self,
        pod: &Pod,
        snapshot: &ClusterSnapshot,
        state: &crate::cycle_state::CycleState,
    ) -> Status {
        for p in &self.pre_scores {
            let s = p.pre_score(pod, snapshot, state);
            if s.is_skip() {
                state.mark_score_skipped(p.name());
                continue;
            }
            if !s.is_success() {
                return s;
            }
        }
        Status::success("framework")
    }

    /// Run every Reserve plugin. On any non-Success, runs Unreserve in
    /// reverse for every plugin that already succeeded.
    pub fn run_reserve(
        &self,
        pod: &Pod,
        node: &str,
        state: &crate::cycle_state::CycleState,
    ) -> Status {
        for (idx, p) in self.reserves.iter().enumerate() {
            let s = p.reserve(pod, node, state);
            if !s.is_success() {
                for prev in self.reserves[..idx].iter().rev() {
                    prev.unreserve(pod, node, state);
                }
                return s;
            }
        }
        Status::success("framework")
    }

    /// Run Unreserve on every Reserve plugin in reverse order.
    pub fn run_unreserve(&self, pod: &Pod, node: &str, state: &crate::cycle_state::CycleState) {
        for p in self.reserves.iter().rev() {
            p.unreserve(pod, node, state);
        }
    }

    /// Run every Permit plugin. Aggregates Wait durations: the largest wait
    /// wins. Any Unschedulable / Error aborts immediately.
    pub fn run_permit(
        &self,
        pod: &Pod,
        node: &str,
        state: &crate::cycle_state::CycleState,
    ) -> Status {
        let mut max_wait = chrono::Duration::zero();
        let mut waiting_plugin: Option<String> = None;
        for p in &self.permits {
            let s = p.permit(pod, node, state);
            match s.code {
                Code::Success => {}
                Code::Wait => {
                    let d = s.wait_duration.unwrap_or_else(chrono::Duration::zero);
                    if d > max_wait {
                        max_wait = d;
                        waiting_plugin = Some(p.name().to_string());
                    }
                }
                _ => return s,
            }
        }
        if max_wait > chrono::Duration::zero() {
            Status::wait(
                waiting_plugin.as_deref().unwrap_or("framework"),
                "permit wait",
                max_wait,
            )
        } else {
            Status::success("framework")
        }
    }

    /// Run every PreBind plugin in order. First non-Success aborts.
    pub fn run_pre_bind(
        &self,
        pod: &Pod,
        node: &str,
        state: &crate::cycle_state::CycleState,
    ) -> Status {
        for p in &self.pre_binds {
            let s = p.pre_bind(pod, node, state);
            if !s.is_success() {
                return s;
            }
        }
        Status::success("framework")
    }

    /// Run Bind plugins in order. First plugin that does not return `Skip`
    /// is the winner. If every plugin returns Skip, returns Error.
    pub fn run_bind(
        &self,
        pod: &Pod,
        node: &str,
        state: &crate::cycle_state::CycleState,
    ) -> Status {
        for p in &self.binds {
            let s = p.bind(pod, node, state);
            if s.is_skip() {
                continue;
            }
            return s;
        }
        Status::error("framework", "no Bind plugin claimed the pod")
    }

    /// Run every PostBind plugin. Best-effort, no status.
    pub fn run_post_bind(&self, pod: &Pod, node: &str, state: &crate::cycle_state::CycleState) {
        for p in &self.post_binds {
            p.post_bind(pod, node, state);
        }
    }

    /// Run every PreEnqueue plugin in order. First non-Success aborts.
    /// Returning `Pending` keeps the pod in the unschedulable subqueue.
    pub fn run_pre_enqueue(&self, pod: &Pod) -> Status {
        for p in &self.pre_enqueues {
            let s = p.pre_enqueue(pod);
            if !s.is_success() {
                return s;
            }
        }
        Status::success("framework")
    }

    /// Order pods using the first registered QueueSort plugin, or by priority
    /// then UID if none is registered. Higher priority first; UID ascending
    /// breaks ties.
    pub fn queue_sort(&self, a: &Pod, b: &Pod) -> std::cmp::Ordering {
        if let Some(p) = self.queue_sorts.first() {
            return p.less(a, b);
        }
        b.spec
            .priority
            .cmp(&a.spec.priority)
            .then_with(|| a.uid.cmp(&b.uid))
    }

    /// Run all score plugins on a candidate node list. Final node score is the
    /// weighted sum of plugin scores: Σ weight(p) * score_p(node).
    pub fn run_scores(
        &self,
        pod: &Pod,
        candidates: &[String],
        snapshot: &ClusterSnapshot,
    ) -> HashMap<String, i64> {
        let mut totals: HashMap<String, i64> = HashMap::new();
        for cand in candidates {
            let Some(node) = snapshot.nodes.iter().find(|n| &n.name == cand) else {
                continue;
            };
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
        let candidates: Vec<String> = filtered
            .iter()
            .filter(|(_, v)| v.is_none())
            .map(|(k, _)| k.clone())
            .collect();
        if candidates.is_empty() {
            return Err(Status::unschedulable(
                "framework",
                "no nodes passed filter chain",
            ));
        }
        let scores = self.run_scores(pod, &candidates, snapshot);
        let (winner, _) = scores
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0))) // deterministic: higher score, lower name first on ties
            .map(|(k, v)| (k.clone(), *v))
            .unwrap_or((candidates[0].clone(), 0));
        Ok(winner)
    }
}

impl Default for Framework {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ResourceCapacity;
    use chrono::Utc;
    use uuid::Uuid;

    fn ready_node(name: &str) -> Node {
        Node {
            name: name.into(),
            uid: Uuid::new_v4(),
            status: NodeStatus::Ready,
            capacity: ResourceCapacity {
                cpu_millicores: 4000,
                memory_bytes: 8_000_000_000,
                pods: 110,
                ephemeral_storage_bytes: 0,
            },
            allocatable: ResourceCapacity {
                cpu_millicores: 4000,
                memory_bytes: 8_000_000_000,
                pods: 110,
                ephemeral_storage_bytes: 0,
            },
            allocated: ResourceCapacity::default(),
            labels: HashMap::new(),
            taints: vec![],
            conditions: vec![],
            registered_at: Utc::now(),
            last_heartbeat: Utc::now(),
        }
    }

    struct AlwaysPass;
    impl FilterPlugin for AlwaysPass {
        fn name(&self) -> &str {
            "AlwaysPass"
        }
        fn filter(&self, _: &Pod, _: &Node, _: &ClusterSnapshot) -> Status {
            Status::success("AlwaysPass")
        }
    }

    struct RejectByName(&'static str);
    impl FilterPlugin for RejectByName {
        fn name(&self) -> &str {
            "RejectByName"
        }
        fn filter(&self, _: &Pod, n: &Node, _: &ClusterSnapshot) -> Status {
            if n.name == self.0 {
                Status::unschedulable("RejectByName", "rejected")
            } else {
                Status::success("RejectByName")
            }
        }
    }

    struct ConstScore(i64);
    impl ScorePlugin for ConstScore {
        fn name(&self) -> &str {
            "ConstScore"
        }
        fn score(&self, _: &Pod, _: &Node, _: &ClusterSnapshot) -> i64 {
            self.0
        }
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
        let snap = ClusterSnapshot {
            nodes: vec![ready_node("a"), ready_node("b")],
            pods_by_node: HashMap::new(),
        };
        let fw = Framework::new().with_filter(Box::new(AlwaysPass));
        let pod = Pod::new("t1", "ns", "p");
        let res = fw.run_filters(&pod, &snap);
        assert!(res.get("a").unwrap().is_none());
        assert!(res.get("b").unwrap().is_none());
    }

    #[test]
    fn framework_filter_chain_short_circuits_on_first_fail() {
        let snap = ClusterSnapshot {
            nodes: vec![ready_node("a"), ready_node("b")],
            pods_by_node: HashMap::new(),
        };
        let fw = Framework::new()
            .with_filter(Box::new(AlwaysPass))
            .with_filter(Box::new(RejectByName("a")));
        let pod = Pod::new("t1", "ns", "p");
        let res = fw.run_filters(&pod, &snap);
        assert!(res.get("a").unwrap().is_some());
        assert_eq!(
            res.get("a").unwrap().as_ref().unwrap().plugin,
            "RejectByName"
        );
        assert!(res.get("b").unwrap().is_none());
    }

    #[test]
    fn scoring_weights_clamp_and_sum() {
        let snap = ClusterSnapshot {
            nodes: vec![ready_node("a")],
            pods_by_node: HashMap::new(),
        };
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
        let snap = ClusterSnapshot {
            nodes: vec![ready_node("a")],
            pods_by_node: HashMap::new(),
        };
        let fw = Framework::new().with_score(Box::new(ConstScore(9999)));
        let pod = Pod::new("t1", "ns", "p");
        let scores = fw.run_scores(&pod, &["a".into()], &snap);
        assert_eq!(scores["a"], MAX_NODE_SCORE);
    }

    #[test]
    fn schedule_one_picks_highest_score() {
        let mut a = ready_node("a");
        a.allocated.cpu_millicores = 1000;
        let mut b = ready_node("b");
        b.allocated.cpu_millicores = 100;
        let snap = ClusterSnapshot {
            nodes: vec![a, b],
            pods_by_node: HashMap::new(),
        };
        struct ByName;
        impl ScorePlugin for ByName {
            fn name(&self) -> &str {
                "ByName"
            }
            fn score(&self, _: &Pod, n: &Node, _: &ClusterSnapshot) -> i64 {
                if n.name == "b" {
                    90
                } else {
                    10
                }
            }
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

    #[test]
    fn status_wait_carries_duration() {
        let s = Status::wait("permit", "needs volume", chrono::Duration::seconds(30));
        assert!(s.is_wait());
        assert_eq!(s.code, Code::Wait);
        assert_eq!(s.wait_duration, Some(chrono::Duration::seconds(30)));
        assert_eq!(s.plugin, "permit");
        assert!(!s.is_rejected());
    }

    #[test]
    fn status_pending_distinct_from_unschedulable() {
        let s = Status::pending("gates", "waiting on quota");
        assert!(s.is_pending());
        assert_eq!(s.code, Code::Pending);
        assert!(!s.is_rejected());
        assert!(!s.is_success());
        assert!(!s.is_skip());
    }

    #[test]
    fn status_error_constructor() {
        let s = Status::error("p", "boom");
        assert!(s.is_error());
        assert_eq!(s.code, Code::Error);
        assert!(!s.is_rejected());
    }

    #[test]
    fn status_skip_constructor() {
        let s = Status::skip("PreFilter");
        assert!(s.is_skip());
        assert_eq!(s.code, Code::Skip);
        assert_eq!(s.plugin, "PreFilter");
    }

    #[test]
    fn status_rejected_predicate_covers_both_codes() {
        assert!(Status::unschedulable("p", "x").is_rejected());
        assert!(Status::unresolvable("p", "x").is_rejected());
        assert!(!Status::success("p").is_rejected());
        assert!(!Status::wait("p", "x", chrono::Duration::seconds(1)).is_rejected());
    }

    #[test]
    fn status_unschedulable_carries_failed_plugin() {
        let s = Status::unschedulable("NodeAffinity", "no match");
        assert_eq!(s.failed_plugin.as_deref(), Some("NodeAffinity"));
    }

    // ── Framework run_pre_filters ────────────────────────────────────────

    struct PrePass(&'static str);
    impl crate::extension_points::PreFilterPlugin for PrePass {
        fn name(&self) -> &str {
            self.0
        }
        fn pre_filter(
            &self,
            _: &Pod,
            _: &ClusterSnapshot,
            _: &crate::cycle_state::CycleState,
        ) -> (crate::extension_points::PreFilterResult, Status) {
            (
                crate::extension_points::PreFilterResult::all_nodes(),
                Status::success(self.0),
            )
        }
    }

    struct PreRestrict(&'static str, Vec<String>);
    impl crate::extension_points::PreFilterPlugin for PreRestrict {
        fn name(&self) -> &str {
            self.0
        }
        fn pre_filter(
            &self,
            _: &Pod,
            _: &ClusterSnapshot,
            _: &crate::cycle_state::CycleState,
        ) -> (crate::extension_points::PreFilterResult, Status) {
            (
                crate::extension_points::PreFilterResult::restrict(self.1.clone()),
                Status::success(self.0),
            )
        }
    }

    struct PreSkip(&'static str);
    impl crate::extension_points::PreFilterPlugin for PreSkip {
        fn name(&self) -> &str {
            self.0
        }
        fn pre_filter(
            &self,
            _: &Pod,
            _: &ClusterSnapshot,
            _: &crate::cycle_state::CycleState,
        ) -> (crate::extension_points::PreFilterResult, Status) {
            (
                crate::extension_points::PreFilterResult::all_nodes(),
                Status::skip(self.0),
            )
        }
    }

    struct PreError(&'static str);
    impl crate::extension_points::PreFilterPlugin for PreError {
        fn name(&self) -> &str {
            self.0
        }
        fn pre_filter(
            &self,
            _: &Pod,
            _: &ClusterSnapshot,
            _: &crate::cycle_state::CycleState,
        ) -> (crate::extension_points::PreFilterResult, Status) {
            (
                crate::extension_points::PreFilterResult::all_nodes(),
                Status::error(self.0, "boom"),
            )
        }
    }

    #[test]
    fn run_pre_filters_intersects_results() {
        let snap = ClusterSnapshot {
            nodes: vec![],
            pods_by_node: HashMap::new(),
        };
        let fw = Framework::new()
            .with_pre_filter(Box::new(PreRestrict(
                "A",
                vec!["n1".into(), "n2".into(), "n3".into()],
            )))
            .with_pre_filter(Box::new(PreRestrict(
                "B",
                vec!["n2".into(), "n3".into(), "n4".into()],
            )));
        let cs = crate::cycle_state::CycleState::new();
        let (res, status) = fw.run_pre_filters(&Pod::new("t", "ns", "p"), &snap, &cs);
        assert!(status.is_success());
        let names = res.node_names.unwrap();
        assert_eq!(names.len(), 2);
        assert!(names.contains("n2"));
        assert!(names.contains("n3"));
    }

    #[test]
    fn run_pre_filters_skip_marks_filter_skipped() {
        let fw = Framework::new().with_pre_filter(Box::new(PreSkip("Skipper")));
        let cs = crate::cycle_state::CycleState::new();
        let snap = ClusterSnapshot {
            nodes: vec![],
            pods_by_node: HashMap::new(),
        };
        let (_, status) = fw.run_pre_filters(&Pod::new("t", "ns", "p"), &snap, &cs);
        assert!(status.is_success());
        assert!(cs.should_skip_filter("Skipper"));
    }

    #[test]
    fn run_pre_filters_error_aborts() {
        let fw = Framework::new()
            .with_pre_filter(Box::new(PrePass("A")))
            .with_pre_filter(Box::new(PreError("Boom")))
            .with_pre_filter(Box::new(PrePass("Never")));
        let cs = crate::cycle_state::CycleState::new();
        let snap = ClusterSnapshot {
            nodes: vec![],
            pods_by_node: HashMap::new(),
        };
        let (_, status) = fw.run_pre_filters(&Pod::new("t", "ns", "p"), &snap, &cs);
        assert!(status.is_error());
        assert_eq!(status.plugin, "Boom");
    }

    // ── Framework run_post_filters ───────────────────────────────────────

    struct NoNominator(&'static str);
    impl crate::extension_points::PostFilterPlugin for NoNominator {
        fn name(&self) -> &str {
            self.0
        }
        fn post_filter(
            &self,
            _: &Pod,
            _: &ClusterSnapshot,
            _: &crate::extension_points::NodeToStatusMap,
            _: &crate::cycle_state::CycleState,
        ) -> (crate::extension_points::PostFilterResult, Status) {
            (
                crate::extension_points::PostFilterResult::default(),
                Status::unschedulable(self.0, "no preempt"),
            )
        }
    }

    struct Nominator(&'static str, &'static str);
    impl crate::extension_points::PostFilterPlugin for Nominator {
        fn name(&self) -> &str {
            self.0
        }
        fn post_filter(
            &self,
            _: &Pod,
            _: &ClusterSnapshot,
            _: &crate::extension_points::NodeToStatusMap,
            _: &crate::cycle_state::CycleState,
        ) -> (crate::extension_points::PostFilterResult, Status) {
            (
                crate::extension_points::PostFilterResult::nominate(self.1),
                Status::success(self.0),
            )
        }
    }

    #[test]
    fn run_post_filters_first_nominator_wins() {
        let fw = Framework::new()
            .with_post_filter(Box::new(NoNominator("None")))
            .with_post_filter(Box::new(Nominator("Yes", "n5")))
            .with_post_filter(Box::new(Nominator("Late", "nX")));
        let cs = crate::cycle_state::CycleState::new();
        let snap = ClusterSnapshot {
            nodes: vec![],
            pods_by_node: HashMap::new(),
        };
        let nodes = crate::extension_points::NodeToStatusMap::new();
        let (res, st) = fw.run_post_filters(&Pod::new("t", "ns", "p"), &snap, &nodes, &cs);
        assert!(st.is_success());
        assert_eq!(st.plugin, "Yes");
        assert_eq!(res.nominating_info.unwrap().nominated_node_name, "n5");
    }

    #[test]
    fn run_post_filters_all_reject_returns_last_status() {
        let fw = Framework::new()
            .with_post_filter(Box::new(NoNominator("First")))
            .with_post_filter(Box::new(NoNominator("Last")));
        let cs = crate::cycle_state::CycleState::new();
        let snap = ClusterSnapshot {
            nodes: vec![],
            pods_by_node: HashMap::new(),
        };
        let nodes = crate::extension_points::NodeToStatusMap::new();
        let (res, st) = fw.run_post_filters(&Pod::new("t", "ns", "p"), &snap, &nodes, &cs);
        assert!(res.nominating_info.is_none());
        assert_eq!(st.plugin, "Last");
        assert!(st.is_rejected());
    }

    // ── Framework run_pre_scores ──────────────────────────────────────────

    struct PreScorePass(&'static str);
    impl crate::extension_points::PreScorePlugin for PreScorePass {
        fn name(&self) -> &str {
            self.0
        }
        fn pre_score(
            &self,
            _: &Pod,
            _: &ClusterSnapshot,
            _: &crate::cycle_state::CycleState,
        ) -> Status {
            Status::success(self.0)
        }
    }
    struct PreScoreSkip(&'static str);
    impl crate::extension_points::PreScorePlugin for PreScoreSkip {
        fn name(&self) -> &str {
            self.0
        }
        fn pre_score(
            &self,
            _: &Pod,
            _: &ClusterSnapshot,
            _: &crate::cycle_state::CycleState,
        ) -> Status {
            Status::skip(self.0)
        }
    }

    #[test]
    fn run_pre_scores_skip_marks_score_skipped() {
        let fw = Framework::new()
            .with_pre_score(Box::new(PreScorePass("A")))
            .with_pre_score(Box::new(PreScoreSkip("Skipper")));
        let cs = crate::cycle_state::CycleState::new();
        let snap = ClusterSnapshot {
            nodes: vec![],
            pods_by_node: HashMap::new(),
        };
        let st = fw.run_pre_scores(&Pod::new("t", "ns", "p"), &snap, &cs);
        assert!(st.is_success());
        assert!(cs.should_skip_score("Skipper"));
        assert!(!cs.should_skip_score("A"));
    }

    // ── Framework run_reserve / run_unreserve ────────────────────────────

    struct LoggingReserve {
        plugin_name: String,
        log: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        fail: bool,
    }
    impl crate::extension_points::ReservePlugin for LoggingReserve {
        fn name(&self) -> &str {
            &self.plugin_name
        }
        fn reserve(&self, _p: &Pod, _n: &str, _s: &crate::cycle_state::CycleState) -> Status {
            self.log
                .lock()
                .unwrap()
                .push(format!("reserve:{}", self.plugin_name));
            if self.fail {
                Status::unschedulable(&self.plugin_name, "reserve failed")
            } else {
                Status::success(&self.plugin_name)
            }
        }
        fn unreserve(&self, _p: &Pod, _n: &str, _s: &crate::cycle_state::CycleState) {
            self.log
                .lock()
                .unwrap()
                .push(format!("unreserve:{}", self.plugin_name));
        }
    }

    #[test]
    fn run_reserve_failure_rolls_back_in_reverse() {
        let log: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let fw = Framework::new()
            .with_reserve(Box::new(LoggingReserve {
                plugin_name: "A".into(),
                log: log.clone(),
                fail: false,
            }))
            .with_reserve(Box::new(LoggingReserve {
                plugin_name: "B".into(),
                log: log.clone(),
                fail: true,
            }))
            .with_reserve(Box::new(LoggingReserve {
                plugin_name: "C".into(),
                log: log.clone(),
                fail: false,
            }));
        let cs = crate::cycle_state::CycleState::new();
        let st = fw.run_reserve(&Pod::new("t", "ns", "p"), "n1", &cs);
        assert!(st.is_rejected());
        let l = log.lock().unwrap();
        // A reserves OK, B reserves fails → rollback A; C never reserves.
        assert_eq!(
            *l,
            vec![
                "reserve:A".to_string(),
                "reserve:B".to_string(),
                "unreserve:A".to_string(),
            ]
        );
    }

    #[test]
    fn run_reserve_success_does_not_unreserve() {
        let log: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let fw = Framework::new()
            .with_reserve(Box::new(LoggingReserve {
                plugin_name: "A".into(),
                log: log.clone(),
                fail: false,
            }))
            .with_reserve(Box::new(LoggingReserve {
                plugin_name: "B".into(),
                log: log.clone(),
                fail: false,
            }));
        let cs = crate::cycle_state::CycleState::new();
        let st = fw.run_reserve(&Pod::new("t", "ns", "p"), "n1", &cs);
        assert!(st.is_success());
        let l = log.lock().unwrap();
        assert_eq!(*l, vec!["reserve:A".to_string(), "reserve:B".to_string()]);
    }

    #[test]
    fn run_unreserve_walks_in_reverse() {
        let log: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let fw = Framework::new()
            .with_reserve(Box::new(LoggingReserve {
                plugin_name: "A".into(),
                log: log.clone(),
                fail: false,
            }))
            .with_reserve(Box::new(LoggingReserve {
                plugin_name: "B".into(),
                log: log.clone(),
                fail: false,
            }))
            .with_reserve(Box::new(LoggingReserve {
                plugin_name: "C".into(),
                log: log.clone(),
                fail: false,
            }));
        let cs = crate::cycle_state::CycleState::new();
        fw.run_unreserve(&Pod::new("t", "ns", "p"), "n1", &cs);
        let l = log.lock().unwrap();
        assert_eq!(
            *l,
            vec![
                "unreserve:C".to_string(),
                "unreserve:B".to_string(),
                "unreserve:A".to_string(),
            ]
        );
    }

    // ── Framework run_permit ─────────────────────────────────────────────

    struct WaitPermit(&'static str, chrono::Duration);
    impl crate::extension_points::PermitPlugin for WaitPermit {
        fn name(&self) -> &str {
            self.0
        }
        fn permit(&self, _p: &Pod, _n: &str, _s: &crate::cycle_state::CycleState) -> Status {
            Status::wait(self.0, "waiting", self.1)
        }
    }
    struct OkPermit(&'static str);
    impl crate::extension_points::PermitPlugin for OkPermit {
        fn name(&self) -> &str {
            self.0
        }
        fn permit(&self, _: &Pod, _: &str, _: &crate::cycle_state::CycleState) -> Status {
            Status::success(self.0)
        }
    }
    struct DenyPermit(&'static str);
    impl crate::extension_points::PermitPlugin for DenyPermit {
        fn name(&self) -> &str {
            self.0
        }
        fn permit(&self, _: &Pod, _: &str, _: &crate::cycle_state::CycleState) -> Status {
            Status::unschedulable(self.0, "denied")
        }
    }

    #[test]
    fn run_permit_max_wait_wins() {
        let fw = Framework::new()
            .with_permit(Box::new(WaitPermit(
                "Fast",
                chrono::Duration::milliseconds(50),
            )))
            .with_permit(Box::new(WaitPermit(
                "Slow",
                chrono::Duration::milliseconds(500),
            )))
            .with_permit(Box::new(OkPermit("Ok")));
        let cs = crate::cycle_state::CycleState::new();
        let st = fw.run_permit(&Pod::new("t", "ns", "p"), "n", &cs);
        assert!(st.is_wait());
        assert_eq!(
            st.wait_duration.unwrap(),
            chrono::Duration::milliseconds(500)
        );
        assert_eq!(st.plugin, "Slow");
    }

    #[test]
    fn run_permit_no_waits_returns_success() {
        let fw = Framework::new()
            .with_permit(Box::new(OkPermit("A")))
            .with_permit(Box::new(OkPermit("B")));
        let cs = crate::cycle_state::CycleState::new();
        let st = fw.run_permit(&Pod::new("t", "ns", "p"), "n", &cs);
        assert!(st.is_success());
    }

    #[test]
    fn run_permit_deny_aborts() {
        let fw = Framework::new()
            .with_permit(Box::new(WaitPermit("W", chrono::Duration::seconds(1))))
            .with_permit(Box::new(DenyPermit("Deny")))
            .with_permit(Box::new(OkPermit("Never")));
        let cs = crate::cycle_state::CycleState::new();
        let st = fw.run_permit(&Pod::new("t", "ns", "p"), "n", &cs);
        assert!(st.is_rejected());
        assert_eq!(st.plugin, "Deny");
    }

    // ── Framework run_bind ────────────────────────────────────────────────

    #[test]
    fn run_bind_first_non_skip_wins() {
        use crate::bind::{DefaultBinder, SkipBinder};
        let binder = std::sync::Arc::new(DefaultBinder::new());
        let binder_ref = binder.clone();
        struct Wrapper(std::sync::Arc<DefaultBinder>);
        impl crate::extension_points::BindPlugin for Wrapper {
            fn name(&self) -> &str {
                "Wrapper"
            }
            fn bind(&self, p: &Pod, n: &str, s: &crate::cycle_state::CycleState) -> Status {
                self.0.bind(p, n, s)
            }
        }
        let fw = Framework::new()
            .with_bind(Box::new(SkipBinder::new("First")))
            .with_bind(Box::new(SkipBinder::new("Second")))
            .with_bind(Box::new(Wrapper(binder_ref)));
        let cs = crate::cycle_state::CycleState::new();
        let st = fw.run_bind(&Pod::new("t", "ns", "p"), "n1", &cs);
        assert!(st.is_success());
        assert_eq!(binder.count(), 1);
    }

    #[test]
    fn run_bind_all_skip_returns_error() {
        use crate::bind::SkipBinder;
        let fw = Framework::new()
            .with_bind(Box::new(SkipBinder::new("A")))
            .with_bind(Box::new(SkipBinder::new("B")));
        let cs = crate::cycle_state::CycleState::new();
        let st = fw.run_bind(&Pod::new("t", "ns", "p"), "n1", &cs);
        assert!(st.is_error());
    }

    // ── Framework run_pre_bind ───────────────────────────────────────────

    struct FailingPreBind(&'static str);
    impl crate::extension_points::PreBindPlugin for FailingPreBind {
        fn name(&self) -> &str {
            self.0
        }
        fn pre_bind(&self, _: &Pod, _: &str, _: &crate::cycle_state::CycleState) -> Status {
            Status::error(self.0, "boom")
        }
    }

    #[test]
    fn run_pre_bind_aborts_on_first_failure() {
        let fw = Framework::new()
            .with_pre_bind(Box::new(crate::bind::NoopPreBinder))
            .with_pre_bind(Box::new(FailingPreBind("Boom")))
            .with_pre_bind(Box::new(crate::bind::NoopPreBinder));
        let cs = crate::cycle_state::CycleState::new();
        let st = fw.run_pre_bind(&Pod::new("t", "ns", "p"), "n", &cs);
        assert!(st.is_error());
        assert_eq!(st.plugin, "Boom");
    }

    #[test]
    fn run_pre_bind_all_succeed() {
        let fw = Framework::new()
            .with_pre_bind(Box::new(crate::bind::NoopPreBinder))
            .with_pre_bind(Box::new(crate::bind::NoopPreBinder));
        let cs = crate::cycle_state::CycleState::new();
        let st = fw.run_pre_bind(&Pod::new("t", "ns", "p"), "n", &cs);
        assert!(st.is_success());
    }

    // ── Framework run_post_bind ───────────────────────────────────────────

    #[test]
    fn run_post_bind_invokes_every_plugin() {
        let logger = std::sync::Arc::new(crate::bind::PostBindLogger::new());
        let l1 = logger.clone();
        let l2 = logger.clone();
        struct Wrap(std::sync::Arc<crate::bind::PostBindLogger>, &'static str);
        impl crate::extension_points::PostBindPlugin for Wrap {
            fn name(&self) -> &str {
                self.1
            }
            fn post_bind(&self, p: &Pod, n: &str, s: &crate::cycle_state::CycleState) {
                self.0.post_bind(p, n, s);
            }
        }
        let fw = Framework::new()
            .with_post_bind(Box::new(Wrap(l1, "A")))
            .with_post_bind(Box::new(Wrap(l2, "B")));
        let cs = crate::cycle_state::CycleState::new();
        fw.run_post_bind(&Pod::new("t", "ns", "p"), "n1", &cs);
        assert_eq!(logger.events().len(), 2);
    }

    // ── Framework run_pre_enqueue ─────────────────────────────────────────

    #[test]
    fn run_pre_enqueue_passes_when_no_gates() {
        let fw = Framework::new().with_pre_enqueue(Box::new(crate::gates::SchedulingGates));
        let st = fw.run_pre_enqueue(&Pod::new("t", "ns", "p"));
        assert!(st.is_success());
    }

    #[test]
    fn run_pre_enqueue_pending_propagates() {
        let fw = Framework::new().with_pre_enqueue(Box::new(crate::gates::SchedulingGates));
        let mut p = Pod::new("t", "ns", "p");
        p.spec.scheduling_gates.push("acme.com/wait".into());
        let st = fw.run_pre_enqueue(&p);
        assert!(st.is_pending());
        assert_eq!(st.plugin, "SchedulingGates");
    }

    // ── Framework queue_sort ──────────────────────────────────────────────

    #[test]
    fn queue_sort_default_uses_priority() {
        let fw = Framework::new();
        let mut a = Pod::new("t", "ns", "a");
        a.spec.priority = 100;
        let mut b = Pod::new("t", "ns", "b");
        b.spec.priority = 50;
        // Higher priority first → a < b in queue order.
        assert_eq!(fw.queue_sort(&a, &b), std::cmp::Ordering::Less);
        assert_eq!(fw.queue_sort(&b, &a), std::cmp::Ordering::Greater);
    }

    #[test]
    fn queue_sort_custom_plugin_overrides_default() {
        struct Reversed;
        impl crate::extension_points::QueueSortPlugin for Reversed {
            fn name(&self) -> &str {
                "Reversed"
            }
            fn less(&self, a: &Pod, b: &Pod) -> std::cmp::Ordering {
                a.spec.priority.cmp(&b.spec.priority) // ascending
            }
        }
        let fw = Framework::new().with_queue_sort(Box::new(Reversed));
        let mut a = Pod::new("t", "ns", "a");
        a.spec.priority = 100;
        let mut b = Pod::new("t", "ns", "b");
        b.spec.priority = 50;
        // Ascending: low priority first → a > b.
        assert_eq!(fw.queue_sort(&a, &b), std::cmp::Ordering::Greater);
    }

    // ── Score normalisation ──────────────────────────────────────────────

    #[test]
    fn score_extension_registered_under_plugin_name() {
        struct Halve;
        impl crate::extension_points::ScoreExtensions for Halve {
            fn normalize_score(
                &self,
                _: &Pod,
                scores: &mut [(String, i64)],
                _: &crate::cycle_state::CycleState,
            ) -> Status {
                for (_, s) in scores.iter_mut() {
                    *s /= 2;
                }
                Status::success("Halve")
            }
        }
        let fw = Framework::new().with_score_extension("Half", Box::new(Halve));
        assert!(fw.score_extensions.contains_key("Half"));
        let mut data = vec![("a".into(), 100i64), ("b".into(), 40)];
        let cs = crate::cycle_state::CycleState::new();
        let ext = fw.score_extensions.get("Half").unwrap();
        let st = ext.normalize_score(&Pod::new("t", "ns", "p"), &mut data, &cs);
        assert!(st.is_success());
        assert_eq!(data[0].1, 50);
        assert_eq!(data[1].1, 20);
    }
}
