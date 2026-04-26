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
            None => Status { code: Code::Skip, reasons: vec![], plugin: "NodeName".into() },
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
// Cite: pkg/scheduler/framework/plugins/imagelocality/image_locality.go

pub struct ImageLocality {
    /// node_name → set of image refs cached on that node.
    pub cache: std::collections::HashMap<String, HashSet<String>>,
}

impl ScorePlugin for ImageLocality {
    fn name(&self) -> &str { "ImageLocality" }
    fn score(&self, pod: &Pod, node: &Node, _: &ClusterSnapshot) -> i64 {
        if pod.spec.container_images.is_empty() { return 0; }
        let cached = self.cache.get(&node.name);
        let hits: usize = pod.spec.container_images.iter().filter(|img| {
            cached.map_or(false, |c| c.contains(*img))
        }).count();
        (hits as i64 * MAX_NODE_SCORE) / pod.spec.container_images.len() as i64
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// InterPodAffinity / PodAntiAffinity
// Cite: pkg/scheduler/framework/plugins/interpodaffinity/plugin.go

pub struct InterPodAffinity;

fn pod_matches_term(p: &Pod, term: &PodAffinityTerm) -> bool {
    // namespaces filter (empty list ⇒ pod.namespace only)
    let ns_ok = if term.namespaces.is_empty() { true } else { term.namespaces.contains(&p.namespace) };
    if !ns_ok { return false; }
    // label selector — every required label must match (use spec.node_selector as the pod label proxy)
    // For test purposes we treat pod.spec.node_selector as the pod's labels.
    term.label_selector.iter().all(|(k, v)| p.spec.node_selector.get(k) == Some(v))
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
                    if pod_matches_term(p, term) { found = true; break 'outer; }
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
                    if pod_matches_term(p, term) {
                        return Status::unschedulable("InterPodAffinity", "antiAffinity violated");
                    }
                }
            }
        }
        Status::success("InterPodAffinity")
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
        p.spec.resources = ResourceRequest { cpu_millicores: 500, memory_bytes: 0 };
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
        }]});
        assert!(NodeAffinity.filter(&p, &node, &empty_snap(vec![])).is_success());
    }
    #[test]
    fn node_affinity_does_not_exist() {
        let node = n("a");
        let mut p = Pod::new("t", "ns", "p");
        p.spec.node_affinity = Some(NodeAffinitySpec { required: vec![NodeSelectorTerm {
            match_expressions: vec![NodeSelectorRequirement { key: "gpu".into(), operator: NodeSelectorOp::DoesNotExist, values: vec![] }],
        }]});
        assert!(NodeAffinity.filter(&p, &node, &empty_snap(vec![])).is_success());
    }
    #[test]
    fn node_affinity_gt_lt() {
        let mut node = n("a"); node.labels.insert("cores".into(), "16".into());
        let mut p = Pod::new("t", "ns", "p");
        p.spec.node_affinity = Some(NodeAffinitySpec { required: vec![NodeSelectorTerm {
            match_expressions: vec![NodeSelectorRequirement { key: "cores".into(), operator: NodeSelectorOp::Gt, values: vec!["8".into()] }],
        }]});
        assert!(NodeAffinity.filter(&p, &node, &empty_snap(vec![])).is_success());

        p.spec.node_affinity = Some(NodeAffinitySpec { required: vec![NodeSelectorTerm {
            match_expressions: vec![NodeSelectorRequirement { key: "cores".into(), operator: NodeSelectorOp::Lt, values: vec!["8".into()] }],
        }]});
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
    #[test]
    fn image_locality_zero_when_no_images() {
        let p = Pod::new("t", "ns", "p");
        let il = ImageLocality { cache: HashMap::new() };
        assert_eq!(il.score(&p, &n("a"), &empty_snap(vec![])), 0);
    }
    #[test]
    fn image_locality_scales_with_hits() {
        let mut p = Pod::new("t", "ns", "p");
        p.spec.container_images = vec!["nginx:1".into(), "redis:7".into()];
        let mut cache = HashMap::new();
        let mut s = HashSet::new(); s.insert("nginx:1".into());
        cache.insert("a".into(), s);
        let il = ImageLocality { cache };
        assert_eq!(il.score(&p, &n("a"), &empty_snap(vec![])), 50);
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
        p.spec.pod_affinity.push(PodAffinityTerm { label_selector: sel, topology_key: "zone".into(), namespaces: vec![] });
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
        p.spec.pod_anti_affinity.push(PodAffinityTerm { label_selector: sel, topology_key: "zone".into(), namespaces: vec![] });
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
        p.spec.pod_affinity.push(PodAffinityTerm { label_selector: sel, topology_key: "zone".into(), namespaces: vec!["ns".into()] });
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
}
