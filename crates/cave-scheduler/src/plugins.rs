//! Built-in Filter/Score plugins.
//!
//! Cite: kubernetes/kubernetes v1.36.0
//!   pkg/scheduler/framework/plugins/{noderesources,nodeaffinity,interpodaffinity,
//!   nodename,nodeunschedulable,tainttoleration,imagelocality,volumebinding,
//!   volumerestrictions,nodeports,nodevolumelimits}/

use crate::framework::*;
use crate::models::{Node, NodeStatus, ResourceCapacity, TaintEffect};
use std::collections::HashSet;

// ─────────────────────────────────────────────────────────────────────────────
// NodeName — pin pod to a specific node by spec.nodeName.
// Cite: pkg/scheduler/framework/plugins/nodename/node_name.go

pub struct NodeName;

impl FilterPlugin for NodeName {
    fn name(&self) -> &str { "NodeName" }
    fn filter(&self, pod: &Pod, node: &Node, _: &ClusterSnapshot) -> Status {
        match &pod.spec.node_name {
            None => Status::skip("NodeName"),
            Some(n) if n == &node.name => Status::success("NodeName"),
            Some(_) => Status::unresolvable("NodeName", "pod nodeName does not match"),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// NodeUnschedulable — reject Cordoned nodes unless pod tolerates the unschedulable taint.
// Cite: pkg/scheduler/framework/plugins/nodeunschedulable/node_unschedulable.go

pub struct NodeUnschedulable;

impl FilterPlugin for NodeUnschedulable {
    fn name(&self) -> &str { "NodeUnschedulable" }
    fn filter(&self, pod: &Pod, node: &Node, _: &ClusterSnapshot) -> Status {
        if node.status != NodeStatus::Cordoned { return Status::success("NodeUnschedulable"); }
        let tolerated = pod.spec.tolerations.iter().any(|t| {
            t.key.as_deref() == Some("node.kubernetes.io/unschedulable")
        });
        if tolerated { Status::success("NodeUnschedulable") }
        else { Status::unschedulable("NodeUnschedulable", "node is cordoned") }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Resources — fit by CPU/memory/pod count, also called "noderesources/Fit".
// Cite: pkg/scheduler/framework/plugins/noderesources/fit.go

pub struct Resources;

fn available(node: &Node) -> ResourceCapacity {
    ResourceCapacity {
        cpu_millicores: node.allocatable.cpu_millicores.saturating_sub(node.allocated.cpu_millicores),
        memory_bytes: node.allocatable.memory_bytes.saturating_sub(node.allocated.memory_bytes),
        pods: node.allocatable.pods.saturating_sub(node.allocated.pods),
        ephemeral_storage_bytes: 0,
    }
}

impl FilterPlugin for Resources {
    fn name(&self) -> &str { "Resources" }
    fn filter(&self, pod: &Pod, node: &Node, _: &ClusterSnapshot) -> Status {
        let avail = available(node);
        if avail.pods == 0 { return Status::unschedulable("Resources", "node pod capacity exhausted"); }
        if !avail.has_room_for(&pod.spec.resources) {
            return Status::unschedulable("Resources", "insufficient cpu or memory");
        }
        Status::success("Resources")
    }
}

impl ScorePlugin for Resources {
    fn name(&self) -> &str { "Resources" }
    fn score(&self, _: &Pod, node: &Node, _: &ClusterSnapshot) -> i64 {
        // LeastAllocated: prefer node with the most free CPU/memory.
        let cpu_total = node.allocatable.cpu_millicores.max(1);
        let mem_total = node.allocatable.memory_bytes.max(1);
        let cpu_free = (cpu_total - node.allocated.cpu_millicores.min(cpu_total)) as i64 * MAX_NODE_SCORE / cpu_total as i64;
        let mem_free = (mem_total - node.allocated.memory_bytes.min(mem_total)) as i64 * MAX_NODE_SCORE / mem_total as i64;
        (cpu_free + mem_free) / 2
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// NodeAffinity — match required nodeSelectorTerms (DNF over matchExpressions).
// Cite: pkg/scheduler/framework/plugins/nodeaffinity/node_affinity.go

pub struct NodeAffinity;

pub fn node_selector_term_matches(term: &NodeSelectorTerm, node: &Node) -> bool {
    term_matches(term, node)
}

fn term_matches(term: &NodeSelectorTerm, node: &Node) -> bool {
    term.match_expressions.iter().all(|req| match req.operator {
        NodeSelectorOp::In => node.labels.get(&req.key).map_or(false, |v| req.values.iter().any(|x| x == v)),
        NodeSelectorOp::NotIn => node.labels.get(&req.key).map_or(true, |v| !req.values.iter().any(|x| x == v)),
        NodeSelectorOp::Exists => node.labels.contains_key(&req.key),
        NodeSelectorOp::DoesNotExist => !node.labels.contains_key(&req.key),
        NodeSelectorOp::Gt => node.labels.get(&req.key)
            .and_then(|v| v.parse::<i64>().ok())
            .zip(req.values.first().and_then(|s| s.parse::<i64>().ok()))
            .map_or(false, |(a, b)| a > b),
        NodeSelectorOp::Lt => node.labels.get(&req.key)
            .and_then(|v| v.parse::<i64>().ok())
            .zip(req.values.first().and_then(|s| s.parse::<i64>().ok()))
            .map_or(false, |(a, b)| a < b),
    })
}

impl FilterPlugin for NodeAffinity {
    fn name(&self) -> &str { "NodeAffinity" }
    fn filter(&self, pod: &Pod, node: &Node, _: &ClusterSnapshot) -> Status {
        // Honor nodeSelector first (legacy required AND).
        for (k, v) in &pod.spec.node_selector {
            if node.labels.get(k) != Some(v) {
                return Status::unschedulable("NodeAffinity", format!("nodeSelector {}={} not matched", k, v));
            }
        }
        let Some(aff) = &pod.spec.node_affinity else { return Status::success("NodeAffinity"); };
        if aff.required.is_empty() { return Status::success("NodeAffinity"); }
        if aff.required.iter().any(|t| term_matches(t, node)) {
            Status::success("NodeAffinity")
        } else {
            Status::unschedulable("NodeAffinity", "no nodeSelectorTerm matched")
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TaintToleration — reject taints with NoSchedule/NoExecute unless tolerated.
// Cite: pkg/scheduler/framework/plugins/tainttoleration/taint_toleration.go

pub struct TaintToleration;

fn tolerates(pod: &Pod, taint: &crate::models::Taint) -> bool {
    pod.spec.tolerations.iter().any(|t| {
        let key_ok = match (t.operator.as_str(), t.key.as_deref()) {
            ("Exists", None) => true, // tolerates everything
            ("Exists", Some(k)) => k == taint.key,
            ("Equal", Some(k)) => k == taint.key && t.value.as_deref() == taint.value.as_deref(),
            _ => false,
        };
        let effect_ok = t.effect.is_none() || t.effect.as_ref() == Some(&taint.effect);
        key_ok && effect_ok
    })
}

impl FilterPlugin for TaintToleration {
    fn name(&self) -> &str { "TaintToleration" }
    fn filter(&self, pod: &Pod, node: &Node, _: &ClusterSnapshot) -> Status {
        for taint in &node.taints {
            if matches!(taint.effect, TaintEffect::NoSchedule | TaintEffect::NoExecute) && !tolerates(pod, taint) {
                return Status::unschedulable("TaintToleration", format!("taint {} not tolerated", taint.key));
            }
        }
        Status::success("TaintToleration")
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ImageLocality — score nodes higher when they already cache the pod's images.
//
// Cite: pkg/scheduler/framework/plugins/imagelocality/image_locality.go
//       (v1.36.0 — `Score` + `sumImageScores` + `calculatePriority`).
//
// Upstream's formula:
//   per_image_scaled = image_size_bytes * (num_nodes_with_image / total_nodes)
//   sum_scores       = Σ per_image_scaled for images on this node from the pod
//   node_score       = clamp_linear(sum_scores, min_threshold, max_threshold,
//                                   0, MAX_NODE_SCORE)
//   * min_threshold  = 23 MiB (tiny images are noise, treat as 0).
//   * max_threshold  = 1000 MiB × num_containers (cap so a single huge
//     image can't saturate the score with one container).
//
// Wider-spread images get *more* score: an image cached on every node
// already replicates the cost across the cluster, so picking the node
// that has it is high-value. A rare image (on 1 of N nodes) gets a
// fractional bonus because the system will still need to pull it
// elsewhere later.

/// Image-state summary the scheduler keeps per `(image_ref → state)`.
/// Mirrors `framework.ImageStateSummary`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageStateSummary {
    /// Compressed size on disk in bytes.
    pub size_bytes: u64,
    /// How many nodes in the cluster currently have this image cached.
    pub num_nodes: u32,
}

/// Per-node view of cached images. Each entry maps the canonical
/// image reference (`<registry>/<repo>:<tag>` or `@sha256:...`) to its
/// state summary.
pub type NodeImageStates = std::collections::HashMap<String, ImageStateSummary>;

/// 23 MiB — below this sum_scores the scaling treats the locality
/// benefit as nil. Matches upstream `mb23Threshold`.
pub const IMAGE_LOCALITY_MIN_THRESHOLD: u64 = 23 * 1024 * 1024;
/// 1000 MiB per container — saturates the cap. Matches upstream
/// `mb1000Threshold`.
pub const IMAGE_LOCALITY_MAX_THRESHOLD_PER_CONTAINER: u64 = 1000 * 1024 * 1024;

pub struct ImageLocality {
    /// `node_name` → cached-image state map. Populate from the
    /// kubelet's image GC report or via the CRI image API
    /// (`cave_cri::routes::list_images`); the scheduler does not
    /// fetch this itself.
    pub node_states: std::collections::HashMap<String, NodeImageStates>,
    /// Total number of schedulable nodes in the cluster — used to
    /// weight the "spread" component. Updated by the same routine
    /// that calls `update_node_images`.
    pub total_nodes: u32,
}

impl Default for ImageLocality {
    fn default() -> Self {
        Self {
            node_states: std::collections::HashMap::new(),
            total_nodes: 0,
        }
    }
}

impl ImageLocality {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the recorded image state for one node. Idempotent;
    /// the caller is responsible for keeping `total_nodes` honest.
    pub fn update_node_images(&mut self, node: &str, states: NodeImageStates) {
        self.node_states.insert(node.to_string(), states);
    }

    /// Bulk replace + recompute `total_nodes`. Use when feeding the
    /// scheduler from a fresh CRI dump on every scheduling cycle.
    pub fn set_cluster_state(&mut self, per_node: std::collections::HashMap<String, NodeImageStates>) {
        self.total_nodes = per_node.len() as u32;
        self.node_states = per_node;
    }

    /// Per-image scaled score. Public so the dispatcher can audit a
    /// single image's contribution + so tests can verify the
    /// upstream formula directly. `num_nodes_with_image` should
    /// match the entry's `state.num_nodes`; we accept it separately
    /// to support tests that override the cluster-wide spread.
    pub fn scaled_image_score(state: &ImageStateSummary, total_nodes: u32) -> i64 {
        if total_nodes == 0 {
            return 0;
        }
        // num_nodes / total — keep it integer-faithful so a one-of-one
        // cluster scores at full size, not 0 from rounding.
        let num = state.num_nodes.min(total_nodes) as i128;
        let total = total_nodes as i128;
        let size = state.size_bytes as i128;
        ((size * num) / total) as i64
    }

    /// Sum image scores for a pod on a given node.
    pub fn sum_image_scores(&self, pod: &Pod, node: &str) -> i64 {
        let Some(states) = self.node_states.get(node) else {
            return 0;
        };
        let mut sum: i128 = 0;
        for img in &pod.spec.container_images {
            if let Some(state) = states.get(img) {
                sum += Self::scaled_image_score(state, self.total_nodes) as i128;
            }
        }
        sum.min(i64::MAX as i128) as i64
    }

    /// Map a sum-of-image-sizes onto `[0, MAX_NODE_SCORE]` using
    /// upstream's linear-with-thresholds curve.
    pub fn calculate_priority(sum_scores: i64, num_containers: u32) -> i64 {
        let containers = num_containers.max(1) as u64;
        let min = IMAGE_LOCALITY_MIN_THRESHOLD as i128;
        let max = (IMAGE_LOCALITY_MAX_THRESHOLD_PER_CONTAINER as i128) * (containers as i128);
        let sum = (sum_scores as i128).max(0);
        let clamped = sum.clamp(min, max);
        let scaled = (MAX_NODE_SCORE as i128) * (clamped - min) / (max - min);
        scaled.clamp(0, MAX_NODE_SCORE as i128) as i64
    }
}

impl ScorePlugin for ImageLocality {
    fn name(&self) -> &str { "ImageLocality" }
    fn score(&self, pod: &Pod, node: &Node, _: &ClusterSnapshot) -> i64 {
        if pod.spec.container_images.is_empty() {
            return 0;
        }
        if self.total_nodes == 0 {
            return 0;
        }
        let sum = self.sum_image_scores(pod, &node.name);
        Self::calculate_priority(sum, pod.spec.container_images.len() as u32)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// InterPodAffinity / PodAntiAffinity
// Cite: pkg/scheduler/framework/plugins/interpodaffinity/plugin.go

pub struct InterPodAffinity;


/// Effective selector for a podAffinity term: literal `label_selector`
/// (HashMap) ANDed with optional `selector_v2` (rich `LabelSelector`),
/// plus values lifted from the *scheduling pod's* labels for every
/// `match_label_keys` entry, and `NotIn`-style exclusions for
/// `mismatch_label_keys`.
fn pod_term_matches_with_pod(p: &Pod, term: &PodAffinityTerm, scheduler_pod: &Pod) -> bool {
    // namespaces filter: literal list (empty → pod.namespace only) plus
    // optional namespace_selector by ns labels (we have no ns store, so
    // namespace_selector falls back to literal list semantics — match
    // upstream when no ns labels are stored).
    let ns_ok = if term.namespaces.is_empty() && term.namespace_selector.is_none() {
        p.namespace == scheduler_pod.namespace
    } else if !term.namespaces.is_empty() {
        term.namespaces.contains(&p.namespace)
    } else {
        // namespace_selector with no labels → match everything.
        true
    };
    if !ns_ok { return false; }

    // match_label_keys: lift scheduler-pod labels into matchLabels.
    let scheduler_labels: &std::collections::HashMap<String, String> = &scheduler_pod.spec.node_selector;
    for k in &term.match_label_keys {
        if let Some(v) = scheduler_labels.get(k) {
            if p.spec.node_selector.get(k) != Some(v) { return false; }
        }
    }
    // mismatch_label_keys: lifted value must NOT match.
    for k in &term.mismatch_label_keys {
        if let Some(v) = scheduler_labels.get(k) {
            if p.spec.node_selector.get(k) == Some(v) { return false; }
        }
    }

    // Legacy HashMap label_selector — ANDed entries.
    let legacy_ok = term.label_selector.iter().all(|(k, v)| p.spec.node_selector.get(k) == Some(v));
    if !legacy_ok { return false; }
    // Rich selector_v2 — when provided, ANDed on top of legacy.
    if let Some(sel) = &term.selector_v2 {
        if !sel.matches(&p.spec.node_selector) { return false; }
    }
    true
}

impl FilterPlugin for InterPodAffinity {
    fn name(&self) -> &str { "InterPodAffinity" }
    fn filter(&self, pod: &Pod, node: &Node, snap: &ClusterSnapshot) -> Status {
        // PodAffinity: each required term must have AT LEAST ONE matching pod
        // on a node sharing the same topology key value as `node`.
        for term in &pod.spec.pod_affinity {
            let Some(my_topo) = node.labels.get(&term.topology_key) else {
                return Status::unschedulable("InterPodAffinity", format!("node lacks topology key {}", term.topology_key));
            };
            let mut found = false;
            'outer: for n in &snap.nodes {
                if n.labels.get(&term.topology_key) != Some(my_topo) { continue; }
                for p in snap.pods_on(&n.name) {
                    if pod_term_matches_with_pod(p, term, pod) { found = true; break 'outer; }
                }
            }
            if !found {
                return Status::unschedulable("InterPodAffinity", "no pod satisfies podAffinity term");
            }
        }
        // PodAntiAffinity: each required term must have NO matching pod sharing topology key.
        for term in &pod.spec.pod_anti_affinity {
            let Some(my_topo) = node.labels.get(&term.topology_key) else { continue; };
            for n in &snap.nodes {
                if n.labels.get(&term.topology_key) != Some(my_topo) { continue; }
                for p in snap.pods_on(&n.name) {
                    if pod_term_matches_with_pod(p, term, pod) {
                        return Status::unschedulable("InterPodAffinity", "antiAffinity violated");
                    }
                }
            }
        }
        Status::success("InterPodAffinity")
    }
}

/// Soft pod (anti)affinity scoring. Adds the weight of every preferred
/// affinity term whose selector matches at least one pod on a node with the
/// same topology-key value as `node`; subtracts the weight of every
/// preferred anti-affinity term that matches. Sum is clamped to
/// `[0, MAX_NODE_SCORE]`.
pub struct InterPodAffinityScore {
    pub preferred_affinity: Vec<WeightedPodAffinityTerm>,
    pub preferred_anti_affinity: Vec<WeightedPodAffinityTerm>,
}

impl InterPodAffinityScore {
    pub fn new() -> Self {
        Self { preferred_affinity: vec![], preferred_anti_affinity: vec![] }
    }
    pub fn with_preferred_affinity(mut self, t: WeightedPodAffinityTerm) -> Self {
        self.preferred_affinity.push(t); self
    }
    pub fn with_preferred_anti_affinity(mut self, t: WeightedPodAffinityTerm) -> Self {
        self.preferred_anti_affinity.push(t); self
    }
}

impl Default for InterPodAffinityScore {
    fn default() -> Self { Self::new() }
}

impl ScorePlugin for InterPodAffinityScore {
    fn name(&self) -> &str { "InterPodAffinityScore" }
    fn score(&self, pod: &Pod, node: &Node, snap: &ClusterSnapshot) -> i64 {
        let mut s: i64 = 0;
        for w in &self.preferred_affinity {
            let Some(topo_v) = node.labels.get(&w.term.topology_key) else { continue; };
            let mut matched = false;
            'outer: for n in &snap.nodes {
                if n.labels.get(&w.term.topology_key) != Some(topo_v) { continue; }
                for p in snap.pods_on(&n.name) {
                    if pod_term_matches_with_pod(p, &w.term, pod) { matched = true; break 'outer; }
                }
            }
            if matched { s += w.weight as i64; }
        }
        for w in &self.preferred_anti_affinity {
            let Some(topo_v) = node.labels.get(&w.term.topology_key) else { continue; };
            'outer2: for n in &snap.nodes {
                if n.labels.get(&w.term.topology_key) != Some(topo_v) { continue; }
                for p in snap.pods_on(&n.name) {
                    if pod_term_matches_with_pod(p, &w.term, pod) {
                        s -= w.weight as i64;
                        break 'outer2;
                    }
                }
            }
        }
        s.clamp(0, MAX_NODE_SCORE)
    }
}

/// NodeAffinity Score plugin — sums the weights of every preferred
/// scheduling term whose preference matches the node, clamped to
/// `[0, MAX_NODE_SCORE]`. Required terms are handled by `NodeAffinity` Filter.
pub struct NodeAffinityScore;

impl ScorePlugin for NodeAffinityScore {
    fn name(&self) -> &str { "NodeAffinityScore" }
    fn score(&self, pod: &Pod, node: &Node, _: &ClusterSnapshot) -> i64 {
        let Some(aff) = &pod.spec.node_affinity else { return 0 };
        let mut s: i64 = 0;
        for pref in &aff.preferred {
            if term_matches(&pref.preference, node) {
                s += pref.weight as i64;
            }
        }
        s.clamp(0, MAX_NODE_SCORE)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// VolumeBinding — PVC bound to a specific node restricts scheduling to that node.
// Cite: pkg/scheduler/framework/plugins/volumebinding/volume_binding.go

pub struct VolumeBinding;

impl FilterPlugin for VolumeBinding {
    fn name(&self) -> &str { "VolumeBinding" }
    fn filter(&self, pod: &Pod, node: &Node, _: &ClusterSnapshot) -> Status {
        for v in &pod.spec.volumes {
            if let VolumeKind::PersistentVolumeClaim { bound_node: Some(bn), claim_name } = &v.kind {
                if bn != &node.name {
                    return Status::unschedulable("VolumeBinding", format!("PVC {} bound to {}", claim_name, bn));
                }
            }
        }
        Status::success("VolumeBinding")
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// VolumeRestrictions — same EBS/GCEPD volume cannot attach to two pods on different
// nodes; for HostPath, single-writer per host path.
// Cite: pkg/scheduler/framework/plugins/volumerestrictions/volume_restrictions.go

pub struct VolumeRestrictions;

impl FilterPlugin for VolumeRestrictions {
    fn name(&self) -> &str { "VolumeRestrictions" }
    fn filter(&self, pod: &Pod, node: &Node, snap: &ClusterSnapshot) -> Status {
        for v in &pod.spec.volumes {
            for other in snap.pods_on(&node.name) {
                if other.uid == pod.uid { continue; }
                for ov in &other.spec.volumes {
                    if same_exclusive_volume(&v.kind, &ov.kind) {
                        return Status::unschedulable("VolumeRestrictions", "exclusive volume already in use on node");
                    }
                }
            }
        }
        Status::success("VolumeRestrictions")
    }
}

fn same_exclusive_volume(a: &VolumeKind, b: &VolumeKind) -> bool {
    match (a, b) {
        (VolumeKind::EBS { volume_id: x }, VolumeKind::EBS { volume_id: y }) => x == y,
        (VolumeKind::GCEPD { pd_name: x }, VolumeKind::GCEPD { pd_name: y }) => x == y,
        (VolumeKind::AzureDisk { disk_name: x }, VolumeKind::AzureDisk { disk_name: y }) => x == y,
        (VolumeKind::HostPath { path: x }, VolumeKind::HostPath { path: y }) => x == y,
        _ => false,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// NodePorts — host ports must not collide with pods already on the node.
// Cite: pkg/scheduler/framework/plugins/nodeports/node_ports.go

pub struct NodePorts;

impl FilterPlugin for NodePorts {
    fn name(&self) -> &str { "NodePorts" }
    fn filter(&self, pod: &Pod, node: &Node, snap: &ClusterSnapshot) -> Status {
        if pod.spec.host_ports.is_empty() { return Status::success("NodePorts"); }
        for other in snap.pods_on(&node.name) {
            for op in &other.spec.host_ports {
                for p in &pod.spec.host_ports {
                    let ip_overlap = p.host_ip == op.host_ip || p.host_ip == "0.0.0.0" || op.host_ip == "0.0.0.0";
                    if ip_overlap && p.port == op.port && p.protocol == op.protocol {
                        return Status::unschedulable("NodePorts", format!("host port {}/{} in use", p.port, p.protocol));
                    }
                }
            }
        }
        Status::success("NodePorts")
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// NodeVolumeLimits — per-provider attached-volume cap (CSI / EBS / GCEPD / AzureDisk).
// Cite: pkg/scheduler/framework/plugins/nodevolumelimits/{csi,non_csi}.go

pub struct NodeVolumeLimits {
    pub provider: VolumeProvider,
    pub limit_per_node: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VolumeProvider { Ebs, GcePd, AzureDisk, Csi }

impl NodeVolumeLimits {
    pub fn ebs(limit: usize) -> Self { Self { provider: VolumeProvider::Ebs, limit_per_node: limit } }
    pub fn gce_pd(limit: usize) -> Self { Self { provider: VolumeProvider::GcePd, limit_per_node: limit } }
    pub fn azure_disk(limit: usize) -> Self { Self { provider: VolumeProvider::AzureDisk, limit_per_node: limit } }
    fn matches(&self, v: &VolumeKind) -> bool {
        matches!((&self.provider, v),
            (VolumeProvider::Ebs, VolumeKind::EBS { .. })
            | (VolumeProvider::GcePd, VolumeKind::GCEPD { .. })
            | (VolumeProvider::AzureDisk, VolumeKind::AzureDisk { .. }))
    }
}

impl FilterPlugin for NodeVolumeLimits {
    fn name(&self) -> &str {
        match self.provider {
            VolumeProvider::Ebs => "EBSLimits",
            VolumeProvider::GcePd => "GCEPDLimits",
            VolumeProvider::AzureDisk => "AzureDiskLimits",
            VolumeProvider::Csi => "NodeVolumeLimits",
        }
    }
    fn filter(&self, pod: &Pod, node: &Node, snap: &ClusterSnapshot) -> Status {
        let mine: usize = pod.spec.volumes.iter().filter(|v| self.matches(&v.kind)).count();
        if mine == 0 { return Status::success(self.name()); }
        let attached: usize = snap.pods_on(&node.name).iter()
            .flat_map(|p| p.spec.volumes.iter())
            .filter(|v| self.matches(&v.kind))
            .count();
        if attached + mine > self.limit_per_node {
            return Status::unschedulable(self.name(), format!("volume limit {} exceeded", self.limit_per_node));
        }
        Status::success(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ResourceCapacity, ResourceRequest, Taint, Toleration};
    use chrono::Utc;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn n(name: &str) -> Node {
        Node {
            name: name.into(), uid: Uuid::new_v4(), status: NodeStatus::Ready,
            capacity: ResourceCapacity { cpu_millicores: 4000, memory_bytes: 8_000_000_000, pods: 110, ephemeral_storage_bytes: 0 },
            allocatable: ResourceCapacity { cpu_millicores: 4000, memory_bytes: 8_000_000_000, pods: 110, ephemeral_storage_bytes: 0 },
            allocated: ResourceCapacity::default(),
            labels: HashMap::new(), taints: vec![], conditions: vec![],
            registered_at: Utc::now(), last_heartbeat: Utc::now(),
        }
    }

    fn empty_snap(nodes: Vec<Node>) -> ClusterSnapshot {
        ClusterSnapshot { nodes, pods_by_node: HashMap::new() }
    }

    // NodeName --------------------------------------------------------------
    #[test]
    fn node_name_skip_when_unset() {
        let p = Pod::new("t", "ns", "p");
        let s = NodeName.filter(&p, &n("a"), &empty_snap(vec![]));
        assert_eq!(s.code, Code::Skip);
    }
    #[test]
    fn node_name_passes_match() {
        let mut p = Pod::new("t", "ns", "p"); p.spec.node_name = Some("a".into());
        assert!(NodeName.filter(&p, &n("a"), &empty_snap(vec![])).is_success());
    }
    #[test]
    fn node_name_unresolvable_when_mismatch() {
        let mut p = Pod::new("t", "ns", "p"); p.spec.node_name = Some("a".into());
        let s = NodeName.filter(&p, &n("b"), &empty_snap(vec![]));
        assert_eq!(s.code, Code::UnschedulableAndUnresolvable);
    }

    // NodeUnschedulable ----------------------------------------------------
    #[test]
    fn node_unsched_passes_ready() {
        let p = Pod::new("t", "ns", "p");
        assert!(NodeUnschedulable.filter(&p, &n("a"), &empty_snap(vec![])).is_success());
    }
    #[test]
    fn node_unsched_blocks_cordoned() {
        let mut node = n("a"); node.status = NodeStatus::Cordoned;
        let p = Pod::new("t", "ns", "p");
        let s = NodeUnschedulable.filter(&p, &node, &empty_snap(vec![]));
        assert_eq!(s.code, Code::Unschedulable);
    }
    #[test]
    fn node_unsched_allows_cordoned_with_toleration() {
        let mut node = n("a"); node.status = NodeStatus::Cordoned;
        let mut p = Pod::new("t", "ns", "p");
        p.spec.tolerations.push(Toleration {
            key: Some("node.kubernetes.io/unschedulable".into()),
            operator: "Exists".into(), value: None, effect: None,
        });
        assert!(NodeUnschedulable.filter(&p, &node, &empty_snap(vec![])).is_success());
    }

    // Resources ------------------------------------------------------------
    #[test]
    fn resources_filter_rejects_when_insufficient() {
        let mut node = n("a"); node.allocated.cpu_millicores = 3900;
        let mut p = Pod::new("t", "ns", "p");
        p.spec.resources = ResourceRequest { cpu_millicores: 500, memory_bytes: 0, ..Default::default() };
        assert_eq!(Resources.filter(&p, &node, &empty_snap(vec![])).code, Code::Unschedulable);
    }
    #[test]
    fn resources_score_higher_when_more_free() {
        let mut a = n("a"); a.allocated.cpu_millicores = 3000; a.allocated.memory_bytes = 6_000_000_000;
        let b = n("b");
        let p = Pod::new("t", "ns", "p");
        let snap = empty_snap(vec![]);
        assert!(Resources.score(&p, &b, &snap) > Resources.score(&p, &a, &snap));
    }

    // NodeAffinity ---------------------------------------------------------
    #[test]
    fn node_affinity_in_operator_match() {
        let mut node = n("a"); node.labels.insert("zone".into(), "eu-west".into());
        let mut p = Pod::new("t", "ns", "p");
        p.spec.node_affinity = Some(NodeAffinitySpec { required: vec![NodeSelectorTerm {
            match_expressions: vec![NodeSelectorRequirement { key: "zone".into(), operator: NodeSelectorOp::In, values: vec!["eu-west".into(), "us-east".into()] }],
        }], ..Default::default() });
        assert!(NodeAffinity.filter(&p, &node, &empty_snap(vec![])).is_success());
    }
    #[test]
    fn node_affinity_does_not_exist() {
        let node = n("a");
        let mut p = Pod::new("t", "ns", "p");
        p.spec.node_affinity = Some(NodeAffinitySpec { required: vec![NodeSelectorTerm {
            match_expressions: vec![NodeSelectorRequirement { key: "gpu".into(), operator: NodeSelectorOp::DoesNotExist, values: vec![] }],
        }], ..Default::default() });
        assert!(NodeAffinity.filter(&p, &node, &empty_snap(vec![])).is_success());
    }
    #[test]
    fn node_affinity_gt_lt() {
        let mut node = n("a"); node.labels.insert("cores".into(), "16".into());
        let mut p = Pod::new("t", "ns", "p");
        p.spec.node_affinity = Some(NodeAffinitySpec { required: vec![NodeSelectorTerm {
            match_expressions: vec![NodeSelectorRequirement { key: "cores".into(), operator: NodeSelectorOp::Gt, values: vec!["8".into()] }],
        }], ..Default::default() });
        assert!(NodeAffinity.filter(&p, &node, &empty_snap(vec![])).is_success());

        p.spec.node_affinity = Some(NodeAffinitySpec { required: vec![NodeSelectorTerm {
            match_expressions: vec![NodeSelectorRequirement { key: "cores".into(), operator: NodeSelectorOp::Lt, values: vec!["8".into()] }],
        }], ..Default::default() });
        assert_eq!(NodeAffinity.filter(&p, &node, &empty_snap(vec![])).code, Code::Unschedulable);
    }
    #[test]
    fn node_selector_takes_precedence() {
        let node = n("a");
        let mut p = Pod::new("t", "ns", "p");
        p.spec.node_selector.insert("missing".into(), "v".into());
        assert_eq!(NodeAffinity.filter(&p, &node, &empty_snap(vec![])).code, Code::Unschedulable);
    }

    // TaintToleration -----------------------------------------------------
    #[test]
    fn taint_blocks_when_no_toleration() {
        let mut node = n("a");
        node.taints.push(Taint { key: "dedicated".into(), value: Some("gpu".into()), effect: TaintEffect::NoSchedule });
        let p = Pod::new("t", "ns", "p");
        assert_eq!(TaintToleration.filter(&p, &node, &empty_snap(vec![])).code, Code::Unschedulable);
    }
    #[test]
    fn taint_equal_toleration_passes() {
        let mut node = n("a");
        node.taints.push(Taint { key: "dedicated".into(), value: Some("gpu".into()), effect: TaintEffect::NoSchedule });
        let mut p = Pod::new("t", "ns", "p");
        p.spec.tolerations.push(Toleration { key: Some("dedicated".into()), operator: "Equal".into(), value: Some("gpu".into()), effect: Some(TaintEffect::NoSchedule) });
        assert!(TaintToleration.filter(&p, &node, &empty_snap(vec![])).is_success());
    }
    #[test]
    fn taint_exists_no_key_tolerates_all() {
        let mut node = n("a");
        node.taints.push(Taint { key: "any".into(), value: None, effect: TaintEffect::NoExecute });
        let mut p = Pod::new("t", "ns", "p");
        p.spec.tolerations.push(Toleration { key: None, operator: "Exists".into(), value: None, effect: None });
        assert!(TaintToleration.filter(&p, &node, &empty_snap(vec![])).is_success());
    }

    // ImageLocality -------------------------------------------------------

    fn states(entries: &[(&str, u64, u32)]) -> NodeImageStates {
        entries
            .iter()
            .map(|(name, size, nodes)| {
                (
                    (*name).to_string(),
                    ImageStateSummary {
                        size_bytes: *size,
                        num_nodes: *nodes,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn image_locality_zero_when_pod_has_no_images() {
        let p = Pod::new("t", "ns", "p");
        let mut il = ImageLocality::new();
        il.total_nodes = 1;
        assert_eq!(il.score(&p, &n("a"), &empty_snap(vec![])), 0);
    }

    #[test]
    fn image_locality_zero_when_no_total_nodes_recorded() {
        let mut p = Pod::new("t", "ns", "p");
        p.spec.container_images = vec!["nginx:1".into()];
        let il = ImageLocality::new(); // total_nodes = 0
        assert_eq!(il.score(&p, &n("a"), &empty_snap(vec![])), 0);
    }

    #[test]
    fn image_locality_zero_when_node_has_no_matching_image() {
        let mut p = Pod::new("t", "ns", "p");
        p.spec.container_images = vec!["nginx:1".into()];
        let mut il = ImageLocality::new();
        il.total_nodes = 3;
        il.update_node_images("a", states(&[("redis:7", 100 * 1024 * 1024, 1)]));
        assert_eq!(il.score(&p, &n("a"), &empty_snap(vec![])), 0);
    }

    #[test]
    fn image_locality_below_min_threshold_scores_zero() {
        // A tiny image (10 MiB) on the node with sum_scores below
        // the 23 MiB min threshold → score 0.
        let mut p = Pod::new("t", "ns", "p");
        p.spec.container_images = vec!["alpine:3".into()];
        let mut il = ImageLocality::new();
        il.total_nodes = 1;
        il.update_node_images("a", states(&[("alpine:3", 10 * 1024 * 1024, 1)]));
        assert_eq!(il.score(&p, &n("a"), &empty_snap(vec![])), 0);
    }

    #[test]
    fn image_locality_at_or_above_max_threshold_saturates_max_score() {
        // 1 GiB image (≥ max threshold for 1 container) on a 1-node
        // cluster → MAX_NODE_SCORE.
        let mut p = Pod::new("t", "ns", "p");
        p.spec.container_images = vec!["postgres:16".into()];
        let mut il = ImageLocality::new();
        il.total_nodes = 1;
        il.update_node_images("a", states(&[("postgres:16", 1000 * 1024 * 1024, 1)]));
        assert_eq!(il.score(&p, &n("a"), &empty_snap(vec![])), MAX_NODE_SCORE);
    }

    #[test]
    fn image_locality_spread_factor_reduces_score_when_image_is_rare() {
        // 1 GiB image present on only 1 of 4 nodes. Scaled score =
        // size * (1/4) = 256 MiB. The priority curve maps that
        // somewhere between min (23 MiB → 0) and max (1000 MiB →
        // 100). Verify the linear interpolation is honoured.
        let mut p = Pod::new("t", "ns", "p");
        p.spec.container_images = vec!["postgres:16".into()];
        let mut il = ImageLocality::new();
        il.total_nodes = 4;
        il.update_node_images("a", states(&[("postgres:16", 1000 * 1024 * 1024, 1)]));
        let got = il.score(&p, &n("a"), &empty_snap(vec![]));
        // Sanity bounds: strictly less than MAX_NODE_SCORE (because
        // spread < 1) and strictly greater than 0 (because sum
        // exceeds min threshold).
        assert!(got > 0 && got < MAX_NODE_SCORE, "got {got}");
    }

    #[test]
    fn image_locality_widely_replicated_image_outscores_rare_image() {
        // Same image size, but node 'a' has it on 4/4 nodes vs node
        // 'b' has it on 1/4 — 'a' must score higher.
        let mut p = Pod::new("t", "ns", "p");
        p.spec.container_images = vec!["postgres:16".into()];
        let mut il = ImageLocality::new();
        il.total_nodes = 4;
        il.update_node_images(
            "a",
            states(&[("postgres:16", 500 * 1024 * 1024, 4)]),
        );
        il.update_node_images(
            "b",
            states(&[("postgres:16", 500 * 1024 * 1024, 1)]),
        );
        let a = il.score(&p, &n("a"), &empty_snap(vec![]));
        let b = il.score(&p, &n("b"), &empty_snap(vec![]));
        assert!(a > b, "a={a} b={b}");
    }

    #[test]
    fn image_locality_multi_container_sums_scores_per_image() {
        // Two containers, two distinct images. Both on the node.
        // sum_scores ≈ 2 × image_size × spread.
        let mut p = Pod::new("t", "ns", "p");
        p.spec.container_images = vec!["nginx:1".into(), "redis:7".into()];
        let mut il = ImageLocality::new();
        il.total_nodes = 1;
        il.update_node_images(
            "a",
            states(&[
                ("nginx:1", 200 * 1024 * 1024, 1),
                ("redis:7", 400 * 1024 * 1024, 1),
            ]),
        );
        // sum = 200 + 400 = 600 MiB; max_threshold = 1000 × 2 = 2000 MiB.
        // priority = (600 - 23) / (2000 - 23) × 100 ≈ 29.
        let got = il.score(&p, &n("a"), &empty_snap(vec![]));
        assert!(got > 0 && got < MAX_NODE_SCORE);
        assert!((25..=35).contains(&got), "got {got}");
    }

    #[test]
    fn image_locality_partial_match_only_counts_present_images() {
        // Pod uses two images; node only has one.
        let mut p = Pod::new("t", "ns", "p");
        p.spec.container_images = vec!["nginx:1".into(), "redis:7".into()];
        let mut il = ImageLocality::new();
        il.total_nodes = 1;
        il.update_node_images("a", states(&[("nginx:1", 200 * 1024 * 1024, 1)]));
        let one = il.score(&p, &n("a"), &empty_snap(vec![]));
        // Adding the second image on the same node must monotonically
        // increase the score (or keep it pinned at MAX).
        il.update_node_images(
            "a",
            states(&[
                ("nginx:1", 200 * 1024 * 1024, 1),
                ("redis:7", 400 * 1024 * 1024, 1),
            ]),
        );
        let both = il.score(&p, &n("a"), &empty_snap(vec![]));
        assert!(both >= one, "both={both} one={one}");
    }

    #[test]
    fn scaled_image_score_clamps_num_nodes_to_total() {
        // Defensive: if state.num_nodes exceeds total_nodes (stale
        // metadata) we shouldn't return more than image size.
        let s = ImageStateSummary { size_bytes: 100, num_nodes: 50 };
        assert_eq!(ImageLocality::scaled_image_score(&s, 4), 100);
    }

    #[test]
    fn calculate_priority_clamps_below_min_to_zero() {
        let p = ImageLocality::calculate_priority(0, 1);
        assert_eq!(p, 0);
        let p = ImageLocality::calculate_priority(
            (IMAGE_LOCALITY_MIN_THRESHOLD as i64) - 1,
            1,
        );
        assert_eq!(p, 0);
    }

    #[test]
    fn calculate_priority_clamps_above_max_to_node_score() {
        let p = ImageLocality::calculate_priority(
            (IMAGE_LOCALITY_MAX_THRESHOLD_PER_CONTAINER as i64) * 10,
            1,
        );
        assert_eq!(p, MAX_NODE_SCORE);
    }

    #[test]
    fn calculate_priority_max_threshold_scales_with_container_count() {
        // Same sum, but 2 containers raises the max threshold and
        // therefore lowers the relative score (more headroom).
        let sum = (IMAGE_LOCALITY_MAX_THRESHOLD_PER_CONTAINER as i64) / 2;
        let one = ImageLocality::calculate_priority(sum, 1);
        let two = ImageLocality::calculate_priority(sum, 2);
        assert!(one > two, "one={one} two={two}");
    }

    #[test]
    fn set_cluster_state_replaces_total_nodes_count() {
        let mut il = ImageLocality::new();
        let mut per_node = std::collections::HashMap::new();
        per_node.insert("a".to_string(), states(&[("x", 1, 1)]));
        per_node.insert("b".to_string(), states(&[("x", 1, 1)]));
        per_node.insert("c".to_string(), states(&[("x", 1, 1)]));
        il.set_cluster_state(per_node);
        assert_eq!(il.total_nodes, 3);
        assert_eq!(il.node_states.len(), 3);
    }

    // InterPodAffinity ----------------------------------------------------
    #[test]
    fn pod_affinity_requires_matching_pod_in_topo() {
        let mut a = n("a"); a.labels.insert("zone".into(), "z1".into());
        let mut existing = Pod::new("t", "ns", "front"); existing.spec.node_selector.insert("app".into(), "web".into());
        let mut snap = empty_snap(vec![a.clone()]);
        snap.pods_by_node.insert("a".into(), vec![existing]);

        let mut p = Pod::new("t", "ns", "back");
        let mut sel = HashMap::new(); sel.insert("app".into(), "web".into());
        p.spec.pod_affinity.push(PodAffinityTerm { label_selector: sel, topology_key: "zone".into(), namespaces: vec![], ..Default::default() });
        assert!(InterPodAffinity.filter(&p, &a, &snap).is_success());
    }

    #[test]
    fn pod_anti_affinity_rejects_when_match_exists() {
        let mut a = n("a"); a.labels.insert("zone".into(), "z1".into());
        let mut existing = Pod::new("t", "ns", "x"); existing.spec.node_selector.insert("app".into(), "db".into());
        let mut snap = empty_snap(vec![a.clone()]);
        snap.pods_by_node.insert("a".into(), vec![existing]);

        let mut p = Pod::new("t", "ns", "y");
        let mut sel = HashMap::new(); sel.insert("app".into(), "db".into());
        p.spec.pod_anti_affinity.push(PodAffinityTerm { label_selector: sel, topology_key: "zone".into(), namespaces: vec![], ..Default::default() });
        assert_eq!(InterPodAffinity.filter(&p, &a, &snap).code, Code::Unschedulable);
    }

    #[test]
    fn pod_affinity_namespace_scope() {
        let mut a = n("a"); a.labels.insert("zone".into(), "z1".into());
        let mut existing = Pod::new("t", "other", "front"); existing.spec.node_selector.insert("app".into(), "web".into());
        let mut snap = empty_snap(vec![a.clone()]);
        snap.pods_by_node.insert("a".into(), vec![existing]);

        let mut p = Pod::new("t", "ns", "back");
        let mut sel = HashMap::new(); sel.insert("app".into(), "web".into());
        p.spec.pod_affinity.push(PodAffinityTerm { label_selector: sel, topology_key: "zone".into(), namespaces: vec!["ns".into()], ..Default::default() });
        assert_eq!(InterPodAffinity.filter(&p, &a, &snap).code, Code::Unschedulable);
    }

    // VolumeBinding -------------------------------------------------------
    #[test]
    fn volume_binding_passes_unbound() {
        let mut p = Pod::new("t", "ns", "p");
        p.spec.volumes.push(VolumeSpec { name: "v".into(), kind: VolumeKind::PersistentVolumeClaim { claim_name: "c".into(), bound_node: None } });
        assert!(VolumeBinding.filter(&p, &n("a"), &empty_snap(vec![])).is_success());
    }
    #[test]
    fn volume_binding_rejects_wrong_node() {
        let mut p = Pod::new("t", "ns", "p");
        p.spec.volumes.push(VolumeSpec { name: "v".into(), kind: VolumeKind::PersistentVolumeClaim { claim_name: "c".into(), bound_node: Some("b".into()) } });
        assert_eq!(VolumeBinding.filter(&p, &n("a"), &empty_snap(vec![])).code, Code::Unschedulable);
    }

    // VolumeRestrictions --------------------------------------------------
    #[test]
    fn volume_restrictions_blocks_duplicate_ebs() {
        let mut existing = Pod::new("t", "ns", "p1");
        existing.spec.volumes.push(VolumeSpec { name: "v".into(), kind: VolumeKind::EBS { volume_id: "vol-1".into() } });
        let mut snap = empty_snap(vec![n("a")]);
        snap.pods_by_node.insert("a".into(), vec![existing]);

        let mut p = Pod::new("t", "ns", "p2");
        p.spec.volumes.push(VolumeSpec { name: "v".into(), kind: VolumeKind::EBS { volume_id: "vol-1".into() } });
        assert_eq!(VolumeRestrictions.filter(&p, &n("a"), &snap).code, Code::Unschedulable);
    }
    #[test]
    fn volume_restrictions_allows_distinct_volumes() {
        let mut existing = Pod::new("t", "ns", "p1");
        existing.spec.volumes.push(VolumeSpec { name: "v".into(), kind: VolumeKind::EBS { volume_id: "vol-1".into() } });
        let mut snap = empty_snap(vec![n("a")]);
        snap.pods_by_node.insert("a".into(), vec![existing]);

        let mut p = Pod::new("t", "ns", "p2");
        p.spec.volumes.push(VolumeSpec { name: "v".into(), kind: VolumeKind::EBS { volume_id: "vol-2".into() } });
        assert!(VolumeRestrictions.filter(&p, &n("a"), &snap).is_success());
    }

    // NodePorts -----------------------------------------------------------
    #[test]
    fn node_ports_passes_when_empty() {
        let p = Pod::new("t", "ns", "p");
        assert!(NodePorts.filter(&p, &n("a"), &empty_snap(vec![])).is_success());
    }
    #[test]
    fn node_ports_blocks_collision() {
        let mut existing = Pod::new("t", "ns", "p1");
        existing.spec.host_ports.push(HostPort { host_ip: "0.0.0.0".into(), port: 80, protocol: "TCP".into() });
        let mut snap = empty_snap(vec![n("a")]);
        snap.pods_by_node.insert("a".into(), vec![existing]);

        let mut p = Pod::new("t", "ns", "p2");
        p.spec.host_ports.push(HostPort { host_ip: "10.0.0.5".into(), port: 80, protocol: "TCP".into() });
        assert_eq!(NodePorts.filter(&p, &n("a"), &snap).code, Code::Unschedulable);
    }

    // NodeVolumeLimits ----------------------------------------------------
    #[test]
    fn ebs_limits_rejects_over_cap() {
        let snap = empty_snap(vec![n("a")]);
        let mut p = Pod::new("t", "ns", "p");
        for i in 0..3 { p.spec.volumes.push(VolumeSpec { name: format!("v{}", i), kind: VolumeKind::EBS { volume_id: format!("v-{}", i) } }); }
        let plug = NodeVolumeLimits::ebs(2);
        assert_eq!(plug.filter(&p, &n("a"), &snap).code, Code::Unschedulable);
        assert_eq!(plug.name(), "EBSLimits");
    }
    #[test]
    fn gce_pd_limits_counts_attached() {
        let mut existing = Pod::new("t", "ns", "p1");
        existing.spec.volumes.push(VolumeSpec { name: "a".into(), kind: VolumeKind::GCEPD { pd_name: "pd-1".into() } });
        let mut snap = empty_snap(vec![n("a")]);
        snap.pods_by_node.insert("a".into(), vec![existing]);

        let mut p = Pod::new("t", "ns", "p2");
        p.spec.volumes.push(VolumeSpec { name: "b".into(), kind: VolumeKind::GCEPD { pd_name: "pd-2".into() } });
        let plug = NodeVolumeLimits::gce_pd(2);
        assert!(plug.filter(&p, &n("a"), &snap).is_success());
        assert_eq!(plug.name(), "GCEPDLimits");

        let plug2 = NodeVolumeLimits::gce_pd(1);
        assert_eq!(plug2.filter(&p, &n("a"), &snap).code, Code::Unschedulable);
    }
    #[test]
    fn azure_disk_limits_only_counts_azure() {
        let mut existing = Pod::new("t", "ns", "p1");
        existing.spec.volumes.push(VolumeSpec { name: "a".into(), kind: VolumeKind::EBS { volume_id: "vol-1".into() } });
        let mut snap = empty_snap(vec![n("a")]);
        snap.pods_by_node.insert("a".into(), vec![existing]);

        let mut p = Pod::new("t", "ns", "p2");
        p.spec.volumes.push(VolumeSpec { name: "b".into(), kind: VolumeKind::AzureDisk { disk_name: "az-1".into() } });
        let plug = NodeVolumeLimits::azure_disk(1);
        assert!(plug.filter(&p, &n("a"), &snap).is_success(), "EBS volumes don't count against Azure limit");
        assert_eq!(plug.name(), "AzureDiskLimits");
    }

    // ── LabelSelector ─────────────────────────────────────────────────────

    #[test]
    fn label_selector_match_labels_and() {
        let mut sel = LabelSelector::default();
        sel.match_labels.insert("env".into(), "prod".into());
        sel.match_labels.insert("tier".into(), "web".into());
        let mut labels = HashMap::new();
        labels.insert("env".into(), "prod".into());
        labels.insert("tier".into(), "web".into());
        labels.insert("extra".into(), "x".into());
        assert!(sel.matches(&labels));
        labels.remove("tier");
        assert!(!sel.matches(&labels));
    }

    #[test]
    fn label_selector_match_expressions_in() {
        let sel = LabelSelector {
            match_labels: HashMap::new(),
            match_expressions: vec![LabelSelectorRequirement {
                key: "tier".into(),
                operator: LabelSelectorOp::In,
                values: vec!["web".into(), "api".into()],
            }],
        };
        let mut labels = HashMap::new();
        labels.insert("tier".into(), "web".into());
        assert!(sel.matches(&labels));
        labels.insert("tier".into(), "db".into());
        assert!(!sel.matches(&labels));
    }

    #[test]
    fn label_selector_match_expressions_not_in() {
        let sel = LabelSelector {
            match_labels: HashMap::new(),
            match_expressions: vec![LabelSelectorRequirement {
                key: "tier".into(),
                operator: LabelSelectorOp::NotIn,
                values: vec!["db".into()],
            }],
        };
        let mut labels = HashMap::new();
        // Missing key → NotIn passes (upstream invariant).
        assert!(sel.matches(&labels));
        labels.insert("tier".into(), "db".into());
        assert!(!sel.matches(&labels));
        labels.insert("tier".into(), "web".into());
        assert!(sel.matches(&labels));
    }

    #[test]
    fn label_selector_match_expressions_exists_doesnotexist() {
        let exists = LabelSelector {
            match_labels: HashMap::new(),
            match_expressions: vec![LabelSelectorRequirement {
                key: "k".into(), operator: LabelSelectorOp::Exists, values: vec![],
            }],
        };
        let dne = LabelSelector {
            match_labels: HashMap::new(),
            match_expressions: vec![LabelSelectorRequirement {
                key: "k".into(), operator: LabelSelectorOp::DoesNotExist, values: vec![],
            }],
        };
        let mut labels = HashMap::new();
        assert!(!exists.matches(&labels));
        assert!(dne.matches(&labels));
        labels.insert("k".into(), "v".into());
        assert!(exists.matches(&labels));
        assert!(!dne.matches(&labels));
    }

    #[test]
    fn label_selector_empty_matches_everything() {
        let sel = LabelSelector::default();
        assert!(sel.is_empty());
        let labels = HashMap::new();
        assert!(sel.matches(&labels));
    }

    #[test]
    fn label_selector_from_match_labels_helper() {
        let mut m = HashMap::new();
        m.insert("env".into(), "prod".into());
        let sel = LabelSelector::from_match_labels(m);
        let mut labels = HashMap::new();
        labels.insert("env".into(), "prod".into());
        assert!(sel.matches(&labels));
    }

    // ── PodAffinityTerm: namespace_selector / match_label_keys / mismatch_label_keys / selector_v2 ──

    #[test]
    fn pod_affinity_namespace_selector_matches_all_when_present() {
        // namespace_selector with no labels → cave-scheduler treats as
        // "match every namespace" since we don't store ns labels.
        let mut a = n("a"); a.labels.insert("zone".into(), "z1".into());
        let mut existing = Pod::new("t", "other-ns", "front");
        existing.spec.node_selector.insert("app".into(), "web".into());
        let mut snap = empty_snap(vec![a.clone()]);
        snap.pods_by_node.insert("a".into(), vec![existing]);

        let mut p = Pod::new("t", "ns", "back");
        let mut sel = HashMap::new(); sel.insert("app".into(), "web".into());
        p.spec.pod_affinity.push(PodAffinityTerm {
            label_selector: sel,
            topology_key: "zone".into(),
            namespaces: vec![],
            namespace_selector: Some(LabelSelector::default()),
            ..Default::default()
        });
        // Other-ns pod is now visible (namespace_selector empty matches all).
        assert!(InterPodAffinity.filter(&p, &a, &snap).is_success());
    }

    #[test]
    fn pod_affinity_match_label_keys_lifts_scheduler_labels() {
        let mut a = n("a"); a.labels.insert("zone".into(), "z1".into());
        let mut existing1 = Pod::new("t", "ns", "front-rev1");
        existing1.spec.node_selector.insert("app".into(), "web".into());
        existing1.spec.node_selector.insert("rev".into(), "1".into());
        let mut existing2 = Pod::new("t", "ns", "front-rev2");
        existing2.spec.node_selector.insert("app".into(), "web".into());
        existing2.spec.node_selector.insert("rev".into(), "2".into());
        let mut snap = empty_snap(vec![a.clone()]);
        snap.pods_by_node.insert("a".into(), vec![existing1, existing2]);

        // Scheduler pod carries rev=1; matchLabelKeys=[rev] lifts that into
        // the selector → only rev=1 pod counts as a match.
        let mut p = Pod::new("t", "ns", "back");
        p.spec.node_selector.insert("rev".into(), "1".into());
        let mut sel = HashMap::new(); sel.insert("app".into(), "web".into());
        p.spec.pod_affinity.push(PodAffinityTerm {
            label_selector: sel.clone(),
            topology_key: "zone".into(),
            namespaces: vec![],
            match_label_keys: vec!["rev".into()],
            ..Default::default()
        });
        assert!(InterPodAffinity.filter(&p, &a, &snap).is_success());

        // Now scheduler pod has rev=3 — no existing pod matches.
        let mut p2 = Pod::new("t", "ns", "back2");
        p2.spec.node_selector.insert("rev".into(), "3".into());
        p2.spec.pod_affinity.push(PodAffinityTerm {
            label_selector: sel,
            topology_key: "zone".into(),
            namespaces: vec![],
            match_label_keys: vec!["rev".into()],
            ..Default::default()
        });
        assert!(InterPodAffinity.filter(&p2, &a, &snap).is_rejected());
    }

    #[test]
    fn pod_affinity_mismatch_label_keys_excludes_same_value() {
        let mut a = n("a"); a.labels.insert("zone".into(), "z1".into());
        let mut existing = Pod::new("t", "ns", "x");
        existing.spec.node_selector.insert("app".into(), "db".into());
        existing.spec.node_selector.insert("rev".into(), "1".into());
        let mut snap = empty_snap(vec![a.clone()]);
        snap.pods_by_node.insert("a".into(), vec![existing]);

        // PodAntiAffinity with mismatch_label_keys=[rev]: incoming pod's
        // rev=1 should NOT match a pod with rev=1 → no anti-violation.
        let mut p = Pod::new("t", "ns", "y");
        p.spec.node_selector.insert("rev".into(), "1".into());
        let mut sel = HashMap::new(); sel.insert("app".into(), "db".into());
        p.spec.pod_anti_affinity.push(PodAffinityTerm {
            label_selector: sel.clone(),
            topology_key: "zone".into(),
            namespaces: vec![],
            mismatch_label_keys: vec!["rev".into()],
            ..Default::default()
        });
        // mismatch_label_keys excludes rev=rev match → existing pod no longer matches.
        assert!(InterPodAffinity.filter(&p, &a, &snap).is_success());

        // Incoming with rev=2 → mismatch with existing rev=1 doesn't
        // exclude → existing matches → anti-affinity rejects.
        let mut p2 = Pod::new("t", "ns", "z");
        p2.spec.node_selector.insert("rev".into(), "2".into());
        p2.spec.pod_anti_affinity.push(PodAffinityTerm {
            label_selector: sel,
            topology_key: "zone".into(),
            namespaces: vec![],
            mismatch_label_keys: vec!["rev".into()],
            ..Default::default()
        });
        assert!(InterPodAffinity.filter(&p2, &a, &snap).is_rejected());
    }

    #[test]
    fn pod_affinity_selector_v2_anded_with_legacy() {
        let mut a = n("a"); a.labels.insert("zone".into(), "z1".into());
        let mut existing = Pod::new("t", "ns", "front");
        existing.spec.node_selector.insert("app".into(), "web".into());
        existing.spec.node_selector.insert("env".into(), "prod".into());
        let mut snap = empty_snap(vec![a.clone()]);
        snap.pods_by_node.insert("a".into(), vec![existing]);

        // Legacy hashmap: app=web. Rich selector_v2: env IN [prod].
        let mut sel_legacy = HashMap::new(); sel_legacy.insert("app".into(), "web".into());
        let sel_v2 = LabelSelector {
            match_labels: HashMap::new(),
            match_expressions: vec![LabelSelectorRequirement {
                key: "env".into(),
                operator: LabelSelectorOp::In,
                values: vec!["prod".into()],
            }],
        };
        let mut p = Pod::new("t", "ns", "back");
        p.spec.pod_affinity.push(PodAffinityTerm {
            label_selector: sel_legacy.clone(),
            topology_key: "zone".into(),
            namespaces: vec![],
            selector_v2: Some(sel_v2.clone()),
            ..Default::default()
        });
        assert!(InterPodAffinity.filter(&p, &a, &snap).is_success());

        // existing pod missing env=prod → no match → reject.
        let mut existing2 = Pod::new("t", "ns", "front");
        existing2.spec.node_selector.insert("app".into(), "web".into());
        let mut snap2 = empty_snap(vec![a.clone()]);
        snap2.pods_by_node.insert("a".into(), vec![existing2]);
        assert!(InterPodAffinity.filter(&p, &a, &snap2).is_rejected());
    }

    // ── NodeAffinityScore (preferred terms) ───────────────────────────────

    #[test]
    fn node_affinity_score_sums_matching_preferences() {
        let mut node = n("a");
        node.labels.insert("zone".into(), "us-east".into());
        node.labels.insert("rack".into(), "r1".into());
        let mut p = Pod::new("t", "ns", "p");
        p.spec.node_affinity = Some(NodeAffinitySpec {
            required: vec![],
            preferred: vec![
                PreferredSchedulingTerm {
                    weight: 30,
                    preference: NodeSelectorTerm { match_expressions: vec![NodeSelectorRequirement {
                        key: "zone".into(), operator: NodeSelectorOp::In, values: vec!["us-east".into()],
                    }]},
                },
                PreferredSchedulingTerm {
                    weight: 20,
                    preference: NodeSelectorTerm { match_expressions: vec![NodeSelectorRequirement {
                        key: "rack".into(), operator: NodeSelectorOp::In, values: vec!["r1".into()],
                    }]},
                },
                PreferredSchedulingTerm {
                    weight: 99,
                    preference: NodeSelectorTerm { match_expressions: vec![NodeSelectorRequirement {
                        key: "zone".into(), operator: NodeSelectorOp::In, values: vec!["eu-west".into()],
                    }]},
                },
            ],
        });
        // Two terms match (30 + 20 = 50); the third (eu-west) doesn't.
        assert_eq!(NodeAffinityScore.score(&p, &node, &empty_snap(vec![])), 50);
    }

    #[test]
    fn node_affinity_score_clamps_to_max() {
        let mut node = n("a");
        node.labels.insert("k".into(), "v".into());
        let mut p = Pod::new("t", "ns", "p");
        p.spec.node_affinity = Some(NodeAffinitySpec {
            required: vec![],
            preferred: vec![PreferredSchedulingTerm {
                weight: 200,
                preference: NodeSelectorTerm { match_expressions: vec![NodeSelectorRequirement {
                    key: "k".into(), operator: NodeSelectorOp::In, values: vec!["v".into()],
                }]},
            }],
        });
        assert_eq!(NodeAffinityScore.score(&p, &node, &empty_snap(vec![])), MAX_NODE_SCORE);
    }

    #[test]
    fn node_affinity_score_zero_when_no_affinity() {
        let p = Pod::new("t", "ns", "p");
        assert_eq!(NodeAffinityScore.score(&p, &n("a"), &empty_snap(vec![])), 0);
    }

    // ── InterPodAffinityScore (preferred terms) ───────────────────────────

    #[test]
    fn inter_pod_affinity_score_adds_preferred_match_weight() {
        let mut a = n("a"); a.labels.insert("zone".into(), "z1".into());
        let mut existing = Pod::new("t", "ns", "x");
        existing.spec.node_selector.insert("app".into(), "web".into());
        let mut snap = empty_snap(vec![a.clone()]);
        snap.pods_by_node.insert("a".into(), vec![existing]);

        let mut sel = HashMap::new(); sel.insert("app".into(), "web".into());
        let p = Pod::new("t", "ns", "y");
        let plug = InterPodAffinityScore::new()
            .with_preferred_affinity(WeightedPodAffinityTerm {
                weight: 40,
                term: PodAffinityTerm {
                    label_selector: sel,
                    topology_key: "zone".into(),
                    namespaces: vec![],
                    ..Default::default()
                },
            });
        assert_eq!(plug.score(&p, &a, &snap), 40);
    }

    #[test]
    fn inter_pod_affinity_score_subtracts_anti_affinity_weight() {
        let mut a = n("a"); a.labels.insert("zone".into(), "z1".into());
        let mut existing = Pod::new("t", "ns", "x");
        existing.spec.node_selector.insert("app".into(), "db".into());
        let mut snap = empty_snap(vec![a.clone()]);
        snap.pods_by_node.insert("a".into(), vec![existing]);

        let mut sel = HashMap::new(); sel.insert("app".into(), "db".into());
        let p = Pod::new("t", "ns", "y");
        let plug = InterPodAffinityScore::new()
            .with_preferred_anti_affinity(WeightedPodAffinityTerm {
                weight: 25,
                term: PodAffinityTerm {
                    label_selector: sel,
                    topology_key: "zone".into(),
                    namespaces: vec![],
                    ..Default::default()
                },
            });
        // 0 - 25 clamped to 0.
        assert_eq!(plug.score(&p, &a, &snap), 0);
    }

    #[test]
    fn inter_pod_affinity_score_combined_affinity_and_anti() {
        let mut a = n("a"); a.labels.insert("zone".into(), "z1".into());
        let mut web = Pod::new("t", "ns", "web1");
        web.spec.node_selector.insert("app".into(), "web".into());
        let mut db = Pod::new("t", "ns", "db1");
        db.spec.node_selector.insert("app".into(), "db".into());
        let mut snap = empty_snap(vec![a.clone()]);
        snap.pods_by_node.insert("a".into(), vec![web, db]);

        let mut sel_web = HashMap::new(); sel_web.insert("app".into(), "web".into());
        let mut sel_db = HashMap::new(); sel_db.insert("app".into(), "db".into());

        let plug = InterPodAffinityScore::new()
            .with_preferred_affinity(WeightedPodAffinityTerm {
                weight: 50,
                term: PodAffinityTerm {
                    label_selector: sel_web,
                    topology_key: "zone".into(),
                    namespaces: vec![],
                    ..Default::default()
                },
            })
            .with_preferred_anti_affinity(WeightedPodAffinityTerm {
                weight: 20,
                term: PodAffinityTerm {
                    label_selector: sel_db,
                    topology_key: "zone".into(),
                    namespaces: vec![],
                    ..Default::default()
                },
            });
        let p = Pod::new("t", "ns", "y");
        // 50 (web match) - 20 (db match) = 30.
        assert_eq!(plug.score(&p, &a, &snap), 30);
    }

    #[test]
    fn inter_pod_affinity_score_zero_when_no_topology_label() {
        let a = n("a"); // no zone label
        let snap = empty_snap(vec![a.clone()]);
        let mut sel = HashMap::new(); sel.insert("app".into(), "web".into());
        let p = Pod::new("t", "ns", "y");
        let plug = InterPodAffinityScore::new()
            .with_preferred_affinity(WeightedPodAffinityTerm {
                weight: 40,
                term: PodAffinityTerm {
                    label_selector: sel,
                    topology_key: "zone".into(),
                    namespaces: vec![],
                    ..Default::default()
                },
            });
        assert_eq!(plug.score(&p, &a, &snap), 0);
    }

    #[test]
    fn inter_pod_affinity_score_clamps_above_max() {
        let mut a = n("a"); a.labels.insert("zone".into(), "z1".into());
        let mut existing = Pod::new("t", "ns", "x");
        existing.spec.node_selector.insert("app".into(), "web".into());
        let mut snap = empty_snap(vec![a.clone()]);
        snap.pods_by_node.insert("a".into(), vec![existing]);

        let mut sel = HashMap::new(); sel.insert("app".into(), "web".into());
        let plug = InterPodAffinityScore::new()
            .with_preferred_affinity(WeightedPodAffinityTerm {
                weight: 200,
                term: PodAffinityTerm {
                    label_selector: sel,
                    topology_key: "zone".into(),
                    namespaces: vec![],
                    ..Default::default()
                },
            });
        let p = Pod::new("t", "ns", "y");
        assert_eq!(plug.score(&p, &a, &snap), MAX_NODE_SCORE);
    }
}
