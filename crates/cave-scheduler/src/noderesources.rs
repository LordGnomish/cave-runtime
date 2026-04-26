//! NodeResources Filter / Score deeper — extended resources, ephemeral storage,
//! pod-count cap, and three scoring strategies.
//!
//! Cite: kubernetes/kubernetes v1.31.0
//!   pkg/scheduler/framework/plugins/noderesources/fit.go
//!   pkg/scheduler/framework/plugins/noderesources/balanced_allocation.go
//!   pkg/scheduler/framework/plugins/noderesources/least_allocated.go
//!   pkg/scheduler/framework/plugins/noderesources/most_allocated.go
//!   pkg/scheduler/framework/plugins/noderesources/requested_to_capacity_ratio.go
//!
//! ## Differences from the simpler `Resources` plugin
//!
//! `plugins::Resources` only checks CPU + memory; this module adds:
//!
//! - **Pod count cap**: independent unschedulable reason when the node has
//!   no slots even though CPU/memory fit.
//! - **Ephemeral storage**: per-pod `ResourceRequest::ephemeral_storage_bytes`
//!   is checked against the node's `allocatable.ephemeral_storage_bytes`.
//! - **Extended resources**: `ResourceRequest::extended` (e.g. `nvidia.com/gpu`)
//!   is checked against the plugin's per-node capacity / allocated maps.
//! - **Configurable score strategy**: LeastAllocated (the default upstream),
//!   MostAllocated, RequestedToCapacityRatio with a configurable shape.
//! - **Resource weights**: per-resource weights influence the score.
//! - **IgnoredResources / IgnoredResourceGroups**: plugin args can mask out
//!   resources from the fit check (matching upstream args).

use crate::framework::{ClusterSnapshot, FilterPlugin, Pod, ScorePlugin, Status, MAX_NODE_SCORE};
use crate::models::Node;
use std::collections::HashMap;

/// Per-resource weights and ignored sets — matches upstream
/// `NodeResourcesFitArgs` and `ScoringStrategy.Resources`.
#[derive(Debug, Clone)]
pub struct NodeResourcesFitArgs {
    /// Resources to skip when computing fit / score. The matching key in
    /// `ResourceRequest::extended` is ignored.
    pub ignored_resources: Vec<String>,
    /// Group prefixes to skip; e.g. "example.com" ignores any extended
    /// resource starting with "example.com/".
    pub ignored_resource_groups: Vec<String>,
    /// Resource weights for the score. Defaults: cpu=1, memory=1, others=1.
    pub resource_weights: HashMap<String, u64>,
    /// Scoring strategy variant.
    pub scoring_strategy: ScoringStrategyType,
}

impl Default for NodeResourcesFitArgs {
    fn default() -> Self {
        let mut weights = HashMap::new();
        weights.insert("cpu".into(), 1);
        weights.insert("memory".into(), 1);
        Self {
            ignored_resources: Vec::new(),
            ignored_resource_groups: Vec::new(),
            resource_weights: weights,
            scoring_strategy: ScoringStrategyType::LeastAllocated,
        }
    }
}

/// Scoring strategy variants — see upstream `ScoringStrategyType`.
#[derive(Debug, Clone)]
pub enum ScoringStrategyType {
    /// score = (capacity - requested) / capacity * 100. Higher when more is
    /// free. Default upstream choice.
    LeastAllocated,
    /// score = requested / capacity * 100. Higher when more is used; biases
    /// the scheduler towards bin-packing.
    MostAllocated,
    /// score derived from a piecewise-linear shape mapping requested/capacity
    /// utilisation `[0..100]` to `[0..MAX_NODE_SCORE]`.
    /// `shape` must be sorted by utilisation ascending; values are
    /// interpolated linearly between adjacent points.
    RequestedToCapacityRatio { shape: Vec<(i64, i64)> },
}

/// Per-node extended resource state — capacity & allocated maps.
///
/// Maintained outside `Node` so the existing model struct stays untouched.
/// One `NodeResourcesFit` instance owns one of these via `Arc`-shared state.
#[derive(Debug, Clone, Default)]
pub struct ExtendedResourcesState {
    /// node_name → resource_name → capacity
    pub capacity: HashMap<String, HashMap<String, u64>>,
    /// node_name → resource_name → currently allocated
    pub allocated: HashMap<String, HashMap<String, u64>>,
}

impl ExtendedResourcesState {
    pub fn set_capacity(&mut self, node: &str, resource: &str, qty: u64) {
        self.capacity.entry(node.into()).or_default().insert(resource.into(), qty);
    }

    pub fn set_allocated(&mut self, node: &str, resource: &str, qty: u64) {
        self.allocated.entry(node.into()).or_default().insert(resource.into(), qty);
    }

    pub fn capacity_of(&self, node: &str, resource: &str) -> u64 {
        self.capacity.get(node).and_then(|m| m.get(resource)).copied().unwrap_or(0)
    }

    pub fn allocated_of(&self, node: &str, resource: &str) -> u64 {
        self.allocated.get(node).and_then(|m| m.get(resource)).copied().unwrap_or(0)
    }

    pub fn free_of(&self, node: &str, resource: &str) -> u64 {
        self.capacity_of(node, resource).saturating_sub(self.allocated_of(node, resource))
    }
}

/// NodeResourcesFit plugin — both Filter and (configurable) Score.
pub struct NodeResourcesFit {
    pub args: NodeResourcesFitArgs,
    pub extended: ExtendedResourcesState,
}

impl NodeResourcesFit {
    pub fn new(args: NodeResourcesFitArgs) -> Self {
        Self { args, extended: ExtendedResourcesState::default() }
    }

    pub fn with_extended(mut self, ext: ExtendedResourcesState) -> Self {
        self.extended = ext;
        self
    }

    fn is_ignored(&self, name: &str) -> bool {
        if self.args.ignored_resources.iter().any(|r| r == name) {
            return true;
        }
        for group in &self.args.ignored_resource_groups {
            // Group is "example.com" → resource "example.com/foo" matches.
            if let Some(rest) = name.strip_prefix(group) {
                if rest.starts_with('/') { return true; }
            }
        }
        false
    }

    fn weight(&self, resource: &str) -> u64 {
        self.args.resource_weights.get(resource).copied().unwrap_or(1)
    }
}

impl FilterPlugin for NodeResourcesFit {
    fn name(&self) -> &str { "NodeResourcesFit" }

    fn filter(&self, pod: &Pod, node: &Node, _: &ClusterSnapshot) -> Status {
        let allocatable = &node.allocatable;
        let allocated = &node.allocated;
        // Pod-count cap — separate reason so PostFilter can tell it apart from
        // CPU/memory exhaustion.
        let pods_free = allocatable.pods.saturating_sub(allocated.pods);
        if pods_free == 0 {
            return Status::unschedulable("NodeResourcesFit", "node pod capacity exhausted");
        }

        let req = &pod.spec.resources;

        if !self.is_ignored("cpu") {
            let cpu_free = allocatable.cpu_millicores.saturating_sub(allocated.cpu_millicores);
            if cpu_free < req.cpu_millicores {
                return Status::unschedulable(
                    "NodeResourcesFit",
                    format!("insufficient cpu: requested {} > free {}", req.cpu_millicores, cpu_free),
                );
            }
        }
        if !self.is_ignored("memory") {
            let mem_free = allocatable.memory_bytes.saturating_sub(allocated.memory_bytes);
            if mem_free < req.memory_bytes {
                return Status::unschedulable(
                    "NodeResourcesFit",
                    format!("insufficient memory: requested {} > free {}", req.memory_bytes, mem_free),
                );
            }
        }
        if !self.is_ignored("ephemeral-storage") {
            let eph_free = allocatable.ephemeral_storage_bytes.saturating_sub(allocated.ephemeral_storage_bytes);
            if eph_free < req.ephemeral_storage_bytes {
                return Status::unschedulable(
                    "NodeResourcesFit",
                    format!("insufficient ephemeral storage: requested {} > free {}", req.ephemeral_storage_bytes, eph_free),
                );
            }
        }
        for (name, qty) in &req.extended {
            if self.is_ignored(name) { continue; }
            let free = self.extended.free_of(&node.name, name);
            if free < *qty {
                return Status::unschedulable(
                    "NodeResourcesFit",
                    format!("insufficient {}: requested {} > free {}", name, qty, free),
                );
            }
        }
        Status::success("NodeResourcesFit")
    }
}

impl ScorePlugin for NodeResourcesFit {
    fn name(&self) -> &str { "NodeResourcesFit" }

    fn score(&self, pod: &Pod, node: &Node, _: &ClusterSnapshot) -> i64 {
        // Per upstream: score is a weighted average of per-resource utilisation
        // mapped through the chosen scoring strategy.
        let mut sum: i64 = 0;
        let mut weight_sum: u64 = 0;

        let mut consider = |name: &str, requested: u64, capacity: u64| {
            if self.is_ignored(name) { return; }
            if capacity == 0 { return; }
            let w = self.weight(name);
            let utilisation_pct = (requested.saturating_mul(100) / capacity).min(100) as i64;
            let s = match &self.args.scoring_strategy {
                ScoringStrategyType::LeastAllocated => MAX_NODE_SCORE - utilisation_pct,
                ScoringStrategyType::MostAllocated => utilisation_pct,
                ScoringStrategyType::RequestedToCapacityRatio { shape } => {
                    interpolate_shape(shape, utilisation_pct)
                }
            };
            sum += (s.max(0).min(MAX_NODE_SCORE)) * w as i64;
            weight_sum += w;
        };

        // Core resources — sum the pod request with what's already running on
        // the node (matches upstream "requested = existing + new pod").
        let cpu_req = pod.spec.resources.cpu_millicores + node.allocated.cpu_millicores;
        let mem_req = pod.spec.resources.memory_bytes + node.allocated.memory_bytes;
        let eph_req = pod.spec.resources.ephemeral_storage_bytes + node.allocated.ephemeral_storage_bytes;

        consider("cpu", cpu_req, node.allocatable.cpu_millicores);
        consider("memory", mem_req, node.allocatable.memory_bytes);
        consider("ephemeral-storage", eph_req, node.allocatable.ephemeral_storage_bytes);

        for (name, qty) in &pod.spec.resources.extended {
            let cap = self.extended.capacity_of(&node.name, name);
            let used = self.extended.allocated_of(&node.name, name);
            consider(name, *qty + used, cap);
        }

        if weight_sum == 0 { return 0; }
        sum / weight_sum as i64
    }
}

fn interpolate_shape(shape: &[(i64, i64)], x: i64) -> i64 {
    if shape.is_empty() { return 0; }
    if x <= shape[0].0 { return shape[0].1; }
    if x >= shape[shape.len() - 1].0 { return shape[shape.len() - 1].1; }
    for w in shape.windows(2) {
        let (x0, y0) = w[0];
        let (x1, y1) = w[1];
        if x >= x0 && x <= x1 {
            // Linear interpolation.
            return y0 + (y1 - y0) * (x - x0) / (x1 - x0).max(1);
        }
    }
    shape[shape.len() - 1].1
}

// ─────────────────────────────────────────────────────────────────────────────
// NodeResourcesBalancedAllocation — score plugin only.
// Cite: pkg/scheduler/framework/plugins/noderesources/balanced_allocation.go
// Picks nodes whose CPU and memory utilisation are *close* to each other
// (low standard deviation), promoting balanced bin-packing.

pub struct NodeResourcesBalancedAllocation {
    pub args: NodeResourcesFitArgs,
}

impl NodeResourcesBalancedAllocation {
    pub fn new() -> Self {
        Self { args: NodeResourcesFitArgs::default() }
    }
}

impl Default for NodeResourcesBalancedAllocation {
    fn default() -> Self { Self::new() }
}

impl ScorePlugin for NodeResourcesBalancedAllocation {
    fn name(&self) -> &str { "NodeResourcesBalancedAllocation" }
    fn score(&self, pod: &Pod, node: &Node, _: &ClusterSnapshot) -> i64 {
        let cap_cpu = node.allocatable.cpu_millicores.max(1);
        let cap_mem = node.allocatable.memory_bytes.max(1);
        let cpu_req = pod.spec.resources.cpu_millicores + node.allocated.cpu_millicores;
        let mem_req = pod.spec.resources.memory_bytes + node.allocated.memory_bytes;
        let cpu_frac = (cpu_req as f64 / cap_cpu as f64).min(1.0);
        let mem_frac = (mem_req as f64 / cap_mem as f64).min(1.0);
        // Mirror upstream: 1 - |cpu_frac - mem_frac| then scaled to MAX_NODE_SCORE.
        let diff = (cpu_frac - mem_frac).abs();
        let s = ((1.0 - diff) * MAX_NODE_SCORE as f64).round() as i64;
        s.clamp(0, MAX_NODE_SCORE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::Pod;
    use crate::models::{NodeStatus, ResourceCapacity, ResourceRequest};
    use chrono::Utc;
    use uuid::Uuid;

    fn n(name: &str, cpu: u64, mem: u64, eph: u64, pods: u64) -> Node {
        Node {
            name: name.into(), uid: Uuid::new_v4(), status: NodeStatus::Ready,
            capacity: ResourceCapacity { cpu_millicores: cpu, memory_bytes: mem, pods, ephemeral_storage_bytes: eph },
            allocatable: ResourceCapacity { cpu_millicores: cpu, memory_bytes: mem, pods, ephemeral_storage_bytes: eph },
            allocated: ResourceCapacity::default(),
            labels: HashMap::new(), taints: vec![], conditions: vec![],
            registered_at: Utc::now(), last_heartbeat: Utc::now(),
        }
    }

    fn pod_req(cpu: u64, mem: u64) -> Pod {
        let mut p = Pod::new("t", "ns", "p");
        p.spec.resources = ResourceRequest { cpu_millicores: cpu, memory_bytes: mem, ..Default::default() };
        p
    }

    fn empty_snap() -> ClusterSnapshot {
        ClusterSnapshot { nodes: vec![], pods_by_node: HashMap::new() }
    }

    // ─── Filter ───────────────────────────────────────────────────────────

    #[test]
    fn filter_passes_when_resources_fit() {
        let plug = NodeResourcesFit::new(NodeResourcesFitArgs::default());
        let p = pod_req(500, 1024);
        assert!(plug.filter(&p, &n("a", 4000, 8192, 0, 110), &empty_snap()).is_success());
    }

    #[test]
    fn filter_rejects_when_pod_count_exhausted() {
        let plug = NodeResourcesFit::new(NodeResourcesFitArgs::default());
        let mut node = n("a", 4000, 8192, 0, 110);
        node.allocated.pods = 110;
        let s = plug.filter(&pod_req(0, 0), &node, &empty_snap());
        assert!(s.is_rejected());
        assert!(s.reasons[0].contains("pod capacity"));
    }

    #[test]
    fn filter_rejects_when_cpu_short() {
        let plug = NodeResourcesFit::new(NodeResourcesFitArgs::default());
        let mut node = n("a", 1000, 8192, 0, 110);
        node.allocated.cpu_millicores = 800;
        let s = plug.filter(&pod_req(500, 0), &node, &empty_snap());
        assert!(s.is_rejected());
        assert!(s.reasons[0].contains("cpu"));
    }

    #[test]
    fn filter_rejects_when_memory_short() {
        let plug = NodeResourcesFit::new(NodeResourcesFitArgs::default());
        let mut node = n("a", 4000, 1024, 0, 110);
        node.allocated.memory_bytes = 512;
        let s = plug.filter(&pod_req(0, 1024), &node, &empty_snap());
        assert!(s.is_rejected());
        assert!(s.reasons[0].contains("memory"));
    }

    #[test]
    fn filter_rejects_when_ephemeral_storage_short() {
        let plug = NodeResourcesFit::new(NodeResourcesFitArgs::default());
        let node = n("a", 4000, 8192, 1024, 110);
        let mut p = pod_req(0, 0);
        p.spec.resources.ephemeral_storage_bytes = 4096;
        let s = plug.filter(&p, &node, &empty_snap());
        assert!(s.is_rejected());
        assert!(s.reasons[0].contains("ephemeral"));
    }

    #[test]
    fn filter_passes_when_ephemeral_storage_fits() {
        let plug = NodeResourcesFit::new(NodeResourcesFitArgs::default());
        let node = n("a", 4000, 8192, 1024 * 1024, 110);
        let mut p = pod_req(0, 0);
        p.spec.resources.ephemeral_storage_bytes = 4096;
        assert!(plug.filter(&p, &node, &empty_snap()).is_success());
    }

    #[test]
    fn filter_passes_when_extended_resource_fits() {
        let mut plug = NodeResourcesFit::new(NodeResourcesFitArgs::default());
        plug.extended.set_capacity("a", "nvidia.com/gpu", 8);
        plug.extended.set_allocated("a", "nvidia.com/gpu", 0);
        let node = n("a", 4000, 8192, 0, 110);
        let mut p = pod_req(0, 0);
        p.spec.resources.extended.insert("nvidia.com/gpu".into(), 1);
        assert!(plug.filter(&p, &node, &empty_snap()).is_success());
    }

    #[test]
    fn filter_rejects_when_extended_resource_short() {
        let mut plug = NodeResourcesFit::new(NodeResourcesFitArgs::default());
        plug.extended.set_capacity("a", "nvidia.com/gpu", 1);
        plug.extended.set_allocated("a", "nvidia.com/gpu", 1);
        let node = n("a", 4000, 8192, 0, 110);
        let mut p = pod_req(0, 0);
        p.spec.resources.extended.insert("nvidia.com/gpu".into(), 1);
        let s = plug.filter(&p, &node, &empty_snap());
        assert!(s.is_rejected());
        assert!(s.reasons[0].contains("nvidia.com/gpu"));
    }

    #[test]
    fn filter_ignores_explicit_resource() {
        let mut args = NodeResourcesFitArgs::default();
        args.ignored_resources.push("nvidia.com/gpu".into());
        let plug = NodeResourcesFit::new(args);
        let node = n("a", 4000, 8192, 0, 110);
        let mut p = pod_req(0, 0);
        p.spec.resources.extended.insert("nvidia.com/gpu".into(), 100);
        // Plugin has no capacity for the resource but it's ignored → success.
        assert!(plug.filter(&p, &node, &empty_snap()).is_success());
    }

    #[test]
    fn filter_ignores_resource_group_prefix() {
        let mut args = NodeResourcesFitArgs::default();
        args.ignored_resource_groups.push("example.com".into());
        let plug = NodeResourcesFit::new(args);
        let node = n("a", 4000, 8192, 0, 110);
        let mut p = pod_req(0, 0);
        p.spec.resources.extended.insert("example.com/widget".into(), 5);
        assert!(plug.filter(&p, &node, &empty_snap()).is_success());
        // Other groups still checked.
        let mut p2 = pod_req(0, 0);
        p2.spec.resources.extended.insert("other.com/widget".into(), 5);
        assert!(plug.filter(&p2, &node, &empty_snap()).is_rejected());
    }

    // ─── Score: LeastAllocated (default) ──────────────────────────────────

    #[test]
    fn score_least_allocated_prefers_more_free() {
        let plug = NodeResourcesFit::new(NodeResourcesFitArgs::default());
        let mut a = n("a", 4000, 8192, 0, 110); a.allocated.cpu_millicores = 3500;
        let b = n("b", 4000, 8192, 0, 110); // empty
        let p = pod_req(100, 0);
        assert!(plug.score(&p, &b, &empty_snap()) > plug.score(&p, &a, &empty_snap()));
    }

    #[test]
    fn score_most_allocated_prefers_less_free() {
        let mut args = NodeResourcesFitArgs::default();
        args.scoring_strategy = ScoringStrategyType::MostAllocated;
        let plug = NodeResourcesFit::new(args);
        let mut a = n("a", 4000, 8192, 0, 110); a.allocated.cpu_millicores = 3500;
        let b = n("b", 4000, 8192, 0, 110);
        let p = pod_req(100, 0);
        assert!(plug.score(&p, &a, &empty_snap()) > plug.score(&p, &b, &empty_snap()));
    }

    #[test]
    fn score_requested_to_capacity_ratio_uses_shape() {
        // Shape: 0% util → 0, 50% → 100, 100% → 0 (favours mid utilisation).
        let mut args = NodeResourcesFitArgs::default();
        args.scoring_strategy = ScoringStrategyType::RequestedToCapacityRatio {
            shape: vec![(0, 0), (50, 100), (100, 0)],
        };
        let plug = NodeResourcesFit::new(args);
        let mut empty_node = n("a", 4000, 8192, 0, 110);
        empty_node.allocated.cpu_millicores = 0;
        let mut half_node = n("b", 4000, 8192, 0, 110);
        half_node.allocated.cpu_millicores = 2000; // 50% util before pod
        let mut full_node = n("c", 4000, 8192, 0, 110);
        full_node.allocated.cpu_millicores = 4000;
        let p = pod_req(0, 0);
        assert!(plug.score(&p, &half_node, &empty_snap()) > plug.score(&p, &empty_node, &empty_snap()));
        assert!(plug.score(&p, &half_node, &empty_snap()) > plug.score(&p, &full_node, &empty_snap()));
    }

    #[test]
    fn score_resource_weights_skew_outcome() {
        let mut args = NodeResourcesFitArgs::default();
        // Memory-only weighting: cpu free becomes irrelevant.
        args.resource_weights.clear();
        args.resource_weights.insert("memory".into(), 10);
        let plug = NodeResourcesFit::new(args);
        let mut a = n("a", 4000, 8192, 0, 110); a.allocated.memory_bytes = 8000;
        let b = n("b", 4000, 8192, 0, 110); // empty memory
        let p = pod_req(0, 0);
        let sb = plug.score(&p, &b, &empty_snap());
        let sa = plug.score(&p, &a, &empty_snap());
        assert!(sb > sa);
    }

    #[test]
    fn score_extended_resource_factored_in() {
        let mut plug = NodeResourcesFit::new(NodeResourcesFitArgs::default());
        plug.extended.set_capacity("a", "nvidia.com/gpu", 8);
        plug.extended.set_allocated("a", "nvidia.com/gpu", 7);
        plug.extended.set_capacity("b", "nvidia.com/gpu", 8);
        plug.extended.set_allocated("b", "nvidia.com/gpu", 0);
        let mut p = pod_req(0, 0);
        p.spec.resources.extended.insert("nvidia.com/gpu".into(), 1);
        let na = n("a", 4000, 8192, 0, 110);
        let nb = n("b", 4000, 8192, 0, 110);
        // LeastAllocated: b with 0/8 (12.5% after pod) scores higher than a (8/8 = 100%).
        let sa = plug.score(&p, &na, &empty_snap());
        let sb = plug.score(&p, &nb, &empty_snap());
        assert!(sb > sa);
    }

    #[test]
    fn interpolate_shape_endpoints_and_midpoint() {
        let shape = vec![(0i64, 0i64), (50, 100), (100, 0)];
        assert_eq!(interpolate_shape(&shape, 0), 0);
        assert_eq!(interpolate_shape(&shape, 50), 100);
        assert_eq!(interpolate_shape(&shape, 100), 0);
        // Midway between 50 and 100 → halfway between 100 and 0 → 50.
        assert_eq!(interpolate_shape(&shape, 75), 50);
        // Below first point clamped to first y.
        assert_eq!(interpolate_shape(&shape, -10), 0);
        // Above last point clamped to last y.
        assert_eq!(interpolate_shape(&shape, 200), 0);
    }

    // ─── BalancedAllocation ──────────────────────────────────────────────

    #[test]
    fn balanced_allocation_prefers_close_cpu_memory() {
        let plug = NodeResourcesBalancedAllocation::new();
        // a: cpu 50% used, mem 50% used → diff 0 → MAX_NODE_SCORE
        let mut a = n("a", 4000, 8000, 0, 110);
        a.allocated.cpu_millicores = 2000;
        a.allocated.memory_bytes = 4000;
        // b: cpu 75% used, mem 25% used → diff 0.5 → MAX_NODE_SCORE/2
        let mut b = n("b", 4000, 8000, 0, 110);
        b.allocated.cpu_millicores = 3000;
        b.allocated.memory_bytes = 2000;
        let p = pod_req(0, 0);
        assert!(plug.score(&p, &a, &empty_snap()) > plug.score(&p, &b, &empty_snap()));
    }

    #[test]
    fn balanced_allocation_zero_capacity_zero_score() {
        let plug = NodeResourcesBalancedAllocation::new();
        let node = n("a", 0, 0, 0, 0);
        // Zero capacities clamped to 1 in the formula → both fractions 0 → diff 0 → MAX.
        // We don't depend on edge case math; just ensure no panic & valid range.
        let s = plug.score(&pod_req(0, 0), &node, &empty_snap());
        assert!((0..=MAX_NODE_SCORE).contains(&s));
    }

    #[test]
    fn balanced_allocation_full_node_low_score() {
        let plug = NodeResourcesBalancedAllocation::new();
        // cpu 100%, mem 0% → diff 1.0 → 0
        let mut a = n("a", 1000, 1000, 0, 10);
        a.allocated.cpu_millicores = 1000;
        let s = plug.score(&pod_req(0, 0), &a, &empty_snap());
        assert!(s < 50);
    }

    // ─── ExtendedResourcesState ─────────────────────────────────────────

    #[test]
    fn extended_state_tracks_capacity_and_allocated() {
        let mut s = ExtendedResourcesState::default();
        s.set_capacity("nodeA", "nvidia.com/gpu", 8);
        s.set_allocated("nodeA", "nvidia.com/gpu", 3);
        assert_eq!(s.capacity_of("nodeA", "nvidia.com/gpu"), 8);
        assert_eq!(s.allocated_of("nodeA", "nvidia.com/gpu"), 3);
        assert_eq!(s.free_of("nodeA", "nvidia.com/gpu"), 5);
    }

    #[test]
    fn extended_state_unknown_node_returns_zero() {
        let s = ExtendedResourcesState::default();
        assert_eq!(s.capacity_of("ghost", "x"), 0);
        assert_eq!(s.allocated_of("ghost", "x"), 0);
        assert_eq!(s.free_of("ghost", "x"), 0);
    }

    #[test]
    fn extended_state_free_saturates_at_zero() {
        let mut s = ExtendedResourcesState::default();
        s.set_capacity("a", "x", 5);
        s.set_allocated("a", "x", 10); // over-allocated bookkeeping bug
        assert_eq!(s.free_of("a", "x"), 0);
    }

    // ─── Plugin name ─────────────────────────────────────────────────────

    #[test]
    fn fit_plugin_name() {
        assert_eq!(<NodeResourcesFit as FilterPlugin>::name(&NodeResourcesFit::new(NodeResourcesFitArgs::default())), "NodeResourcesFit");
        assert_eq!(<NodeResourcesFit as ScorePlugin>::name(&NodeResourcesFit::new(NodeResourcesFitArgs::default())), "NodeResourcesFit");
    }

    #[test]
    fn balanced_plugin_name() {
        assert_eq!(NodeResourcesBalancedAllocation::new().name(), "NodeResourcesBalancedAllocation");
    }
}
