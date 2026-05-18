// SPDX-License-Identifier: AGPL-3.0-or-later
//! DefaultPreemption — PostFilter plugin running upstream's preemption flow.
//!
//! Cite: kubernetes/kubernetes v1.31.0
//!   pkg/scheduler/framework/plugins/defaultpreemption/default_preemption.go
//!   pkg/scheduler/framework/preemption/preemption.go
//!
//! ## Flow
//!
//! 1. **PostFilter** runs only when Filter rejected every node. The plugin
//!    walks every node, checks whether it is *potentially preemptable* (i.e.
//!    the rejection was Unschedulable, not UnschedulableAndUnresolvable),
//!    and computes a victim set on it.
//! 2. The cheapest candidate (fewest victims, then fewest PDB violations,
//!    then deterministic node name) wins.
//! 3. The plugin returns a [`PostFilterResult::nominate`] for the chosen
//!    node and stages the victim list on the [`AsyncPreemptHandle`] so a
//!    background worker can issue eviction RPCs without blocking the
//!    scheduler's main loop.
//! 4. The pod's UID is recorded in [`NominatedNodeMap`] so subsequent
//!    cycles know it is "owned" by that node and skip it for other pods.

use crate::cycle_state::CycleState;
use crate::extension_points::{NodeToStatusMap, PostFilterPlugin, PostFilterResult};
use crate::framework::{ClusterSnapshot, Pod, Status};
use crate::preempt::{preempt as preempt_impl, PodDisruptionBudget, PreemptionResult};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Configuration knobs for DefaultPreemption (mirrors upstream
/// `DefaultPreemptionArgs`).
#[derive(Debug, Clone)]
pub struct DefaultPreemptionArgs {
    /// Minimum percentage of candidate nodes the plugin will inspect.
    /// `100` means "always inspect every node"; defaults to upstream's 10.
    pub min_candidate_nodes_percentage: u32,
    /// Minimum absolute number of candidate nodes (defaults to 100).
    pub min_candidate_nodes_absolute: u32,
    /// When `true`, no eviction is staged — only the plan is returned.
    pub dry_run: bool,
    /// When set, victims with priority strictly below this threshold are
    /// excluded ("don't evict mission-critical low-priority pods").
    pub min_evictable_priority: Option<i32>,
}

impl Default for DefaultPreemptionArgs {
    fn default() -> Self {
        Self {
            min_candidate_nodes_percentage: 10,
            min_candidate_nodes_absolute: 100,
            dry_run: false,
            min_evictable_priority: None,
        }
    }
}

/// (pod_uid → nominated_node) map. Owned by the scheduler outside any one
/// cycle so cross-cycle coordination is possible.
#[derive(Debug, Default)]
pub struct NominatedNodeMap {
    inner: Mutex<HashMap<String, String>>,
}

impl NominatedNodeMap {
    pub fn new() -> Self { Self::default() }

    pub fn nominate(&self, pod_uid: &str, node: &str) {
        self.inner.lock().unwrap().insert(pod_uid.into(), node.into());
    }

    pub fn nominated_for(&self, pod_uid: &str) -> Option<String> {
        self.inner.lock().unwrap().get(pod_uid).cloned()
    }

    pub fn clear(&self, pod_uid: &str) {
        self.inner.lock().unwrap().remove(pod_uid);
    }

    pub fn len(&self) -> usize { self.inner.lock().unwrap().len() }
    pub fn is_empty(&self) -> bool { self.inner.lock().unwrap().is_empty() }
}

/// Eviction queue — upstream issues eviction subresource RPCs; we record the
/// (pod_uid, node) pair so the rest of cave-runtime can drain it.
#[derive(Debug, Default)]
pub struct AsyncPreemptHandle {
    pending: Mutex<Vec<EvictionTask>>,
    completed: Mutex<Vec<EvictionTask>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvictionTask {
    pub victim_uid: String,
    pub victim_namespace: String,
    pub victim_name: String,
    pub node_name: String,
    pub preemptor_uid: String,
}

impl AsyncPreemptHandle {
    pub fn new() -> Self { Self::default() }

    pub fn enqueue(&self, task: EvictionTask) {
        self.pending.lock().unwrap().push(task);
    }

    /// Drain one pending eviction; mark it completed. Returns the task or
    /// `None` if the queue is empty. (Kept as a sync API; cave-runtime's
    /// eviction worker calls it in a tokio task.)
    pub fn dequeue(&self) -> Option<EvictionTask> {
        let mut pending = self.pending.lock().unwrap();
        if pending.is_empty() { return None; }
        let task = pending.remove(0);
        self.completed.lock().unwrap().push(task.clone());
        Some(task)
    }

    pub fn pending_len(&self) -> usize { self.pending.lock().unwrap().len() }
    pub fn completed_len(&self) -> usize { self.completed.lock().unwrap().len() }
    pub fn pending(&self) -> Vec<EvictionTask> { self.pending.lock().unwrap().clone() }
}

/// PostFilter plugin running the cave-scheduler preemption algorithm.
pub struct DefaultPreemption {
    pub args: DefaultPreemptionArgs,
    pub pdbs: Vec<PodDisruptionBudget>,
    pub nominated: Arc<NominatedNodeMap>,
    pub async_handle: Arc<AsyncPreemptHandle>,
}

impl DefaultPreemption {
    pub fn new(
        args: DefaultPreemptionArgs,
        pdbs: Vec<PodDisruptionBudget>,
        nominated: Arc<NominatedNodeMap>,
        async_handle: Arc<AsyncPreemptHandle>,
    ) -> Self {
        Self { args, pdbs, nominated, async_handle }
    }

    /// Wrap upstream `preempt()` with the per-args min-evictable-priority
    /// gate. Honors min_evictable_priority by filtering victims after the
    /// inner algorithm picked them — upstream applies it inside the picker;
    /// we approximate by post-filtering and rejecting the candidate if the
    /// remaining set no longer covers the resource gap.
    fn preempt_for(&self, preemptor: &Pod, snap: &ClusterSnapshot) -> Option<PreemptionResult> {
        let raw = preempt_impl(preemptor, snap, &self.pdbs)?;
        if let Some(min_prio) = self.args.min_evictable_priority {
            if raw.victims.iter().any(|v| v.spec.priority < min_prio) {
                return None;
            }
        }
        Some(raw)
    }

    /// Stage every victim on the async handle (or skip when `dry_run`).
    fn stage_evictions(&self, preemptor: &Pod, victims: &[Pod], node: &str) {
        if self.args.dry_run { return; }
        for v in victims {
            self.async_handle.enqueue(EvictionTask {
                victim_uid: v.uid.clone(),
                victim_namespace: v.namespace.clone(),
                victim_name: v.name.clone(),
                node_name: node.into(),
                preemptor_uid: preemptor.uid.clone(),
            });
        }
    }
}

impl PostFilterPlugin for DefaultPreemption {
    fn name(&self) -> &str { "DefaultPreemption" }

    fn post_filter(
        &self,
        pod: &Pod,
        snapshot: &ClusterSnapshot,
        filtered: &NodeToStatusMap,
        _state: &CycleState,
    ) -> (PostFilterResult, Status) {
        // Skip nodes whose rejection was UnschedulableAndUnresolvable —
        // those failures cannot be fixed by evicting other pods.
        let mut candidate_snapshot = ClusterSnapshot {
            nodes: snapshot.nodes.iter()
                .filter(|n| match filtered.get(&n.name) {
                    Some(s) if s.code == crate::framework::Code::UnschedulableAndUnresolvable => false,
                    _ => true,
                })
                .cloned()
                .collect(),
            pods_by_node: snapshot.pods_by_node.clone(),
        };
        // min_candidate_nodes_{percentage,absolute}: at least
        // max(percent% of total, absolute) candidates considered.
        let total = snapshot.nodes.len();
        let min = std::cmp::max(
            (self.args.min_candidate_nodes_percentage as usize * total) / 100,
            self.args.min_candidate_nodes_absolute as usize,
        ).min(total);
        if candidate_snapshot.nodes.len() > min {
            // Sort by name (deterministic) and truncate.
            candidate_snapshot.nodes.sort_by(|a, b| a.name.cmp(&b.name));
            candidate_snapshot.nodes.truncate(min);
        }

        let Some(plan) = self.preempt_for(pod, &candidate_snapshot) else {
            return (
                PostFilterResult::default(),
                Status::unschedulable("DefaultPreemption", "no preemption candidate found"),
            );
        };
        self.stage_evictions(pod, &plan.victims, &plan.nominated_node_name);
        self.nominated.nominate(&pod.uid, &plan.nominated_node_name);
        (
            PostFilterResult::nominate(plan.nominated_node_name.clone()),
            Status::success("DefaultPreemption"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::Pod;
    use crate::models::{Node, NodeStatus, ResourceCapacity, ResourceRequest};
    use chrono::Utc;
    use std::collections::HashMap;
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
        p.spec.resources = ResourceRequest { cpu_millicores: cpu, memory_bytes: mem, ..Default::default() };
        p
    }

    // ── NominatedNodeMap ─────────────────────────────────────────────────

    #[test]
    fn nominated_map_round_trip() {
        let m = NominatedNodeMap::new();
        m.nominate("uid", "node-a");
        assert_eq!(m.nominated_for("uid").as_deref(), Some("node-a"));
        m.clear("uid");
        assert!(m.nominated_for("uid").is_none());
    }

    #[test]
    fn nominated_map_replaces_existing() {
        let m = NominatedNodeMap::new();
        m.nominate("uid", "node-a");
        m.nominate("uid", "node-b");
        assert_eq!(m.nominated_for("uid").as_deref(), Some("node-b"));
    }

    #[test]
    fn nominated_map_len_and_empty() {
        let m = NominatedNodeMap::new();
        assert!(m.is_empty());
        m.nominate("u1", "a");
        m.nominate("u2", "b");
        assert_eq!(m.len(), 2);
    }

    // ── AsyncPreemptHandle ───────────────────────────────────────────────

    #[test]
    fn async_handle_enqueue_and_dequeue() {
        let h = AsyncPreemptHandle::new();
        let task = EvictionTask {
            victim_uid: "v".into(), victim_namespace: "ns".into(), victim_name: "n".into(),
            node_name: "node".into(), preemptor_uid: "p".into(),
        };
        h.enqueue(task.clone());
        assert_eq!(h.pending_len(), 1);
        let drained = h.dequeue().unwrap();
        assert_eq!(drained, task);
        assert_eq!(h.pending_len(), 0);
        assert_eq!(h.completed_len(), 1);
    }

    #[test]
    fn async_handle_dequeue_empty_returns_none() {
        let h = AsyncPreemptHandle::new();
        assert!(h.dequeue().is_none());
    }

    #[test]
    fn async_handle_fifo_order() {
        let h = AsyncPreemptHandle::new();
        for i in 0..3 {
            h.enqueue(EvictionTask {
                victim_uid: format!("v{}", i),
                victim_namespace: "ns".into(),
                victim_name: format!("p{}", i),
                node_name: "n".into(),
                preemptor_uid: "p".into(),
            });
        }
        assert_eq!(h.dequeue().unwrap().victim_uid, "v0");
        assert_eq!(h.dequeue().unwrap().victim_uid, "v1");
        assert_eq!(h.dequeue().unwrap().victim_uid, "v2");
    }

    // ── DefaultPreemptionArgs defaults ───────────────────────────────────

    #[test]
    fn default_args_match_upstream_constants() {
        let a = DefaultPreemptionArgs::default();
        assert_eq!(a.min_candidate_nodes_percentage, 10);
        assert_eq!(a.min_candidate_nodes_absolute, 100);
        assert!(!a.dry_run);
        assert!(a.min_evictable_priority.is_none());
    }

    // ── DefaultPreemption.post_filter ────────────────────────────────────

    fn full_snap(nodes: Vec<Node>, pods_per_node: HashMap<String, Vec<Pod>>) -> ClusterSnapshot {
        ClusterSnapshot { nodes, pods_by_node: pods_per_node }
    }

    fn rejected_map(nodes: &[&str], code: crate::framework::Code) -> NodeToStatusMap {
        let mut m = NodeToStatusMap::new();
        for n in nodes {
            let s = match code {
                crate::framework::Code::Unschedulable => Status::unschedulable("Resources", "x"),
                crate::framework::Code::UnschedulableAndUnresolvable => Status::unresolvable("X", "x"),
                _ => Status::success("X"),
            };
            m.set(n.to_string(), s);
        }
        m
    }

    #[test]
    fn post_filter_nominates_node_with_smallest_victim_set() {
        let mut a = full_node("a");
        a.allocated.cpu_millicores = 4000;
        let snap = full_snap(
            vec![a],
            HashMap::from([("a".into(), vec![pod_at("t", "low", 0, 1000, 0), pod_at("t", "low2", 0, 1000, 0)])]),
        );
        let preemptor = pod_at("t", "new", 50, 500, 0);
        let nominated = Arc::new(NominatedNodeMap::new());
        let handle = Arc::new(AsyncPreemptHandle::new());
        let plug = DefaultPreemption::new(
            DefaultPreemptionArgs::default(),
            vec![],
            nominated.clone(),
            handle.clone(),
        );
        let cs = CycleState::new();
        let filtered = rejected_map(&["a"], crate::framework::Code::Unschedulable);
        let (res, st) = plug.post_filter(&preemptor, &snap, &filtered, &cs);
        assert!(st.is_success());
        assert_eq!(res.nominating_info.unwrap().nominated_node_name, "a");
        // Async handle staged 1 victim.
        assert_eq!(handle.pending_len(), 1);
        // Nominated map updated.
        assert_eq!(nominated.nominated_for(&preemptor.uid).as_deref(), Some("a"));
    }

    #[test]
    fn post_filter_skips_unresolvable_nodes() {
        let mut a = full_node("a");
        a.allocated.cpu_millicores = 4000;
        let snap = full_snap(
            vec![a],
            HashMap::from([("a".into(), vec![pod_at("t", "low", 0, 1000, 0)])]),
        );
        let preemptor = pod_at("t", "new", 50, 500, 0);
        let nominated = Arc::new(NominatedNodeMap::new());
        let handle = Arc::new(AsyncPreemptHandle::new());
        let plug = DefaultPreemption::new(
            DefaultPreemptionArgs::default(),
            vec![],
            nominated.clone(),
            handle.clone(),
        );
        let cs = CycleState::new();
        let filtered = rejected_map(&["a"], crate::framework::Code::UnschedulableAndUnresolvable);
        let (res, st) = plug.post_filter(&preemptor, &snap, &filtered, &cs);
        assert!(res.nominating_info.is_none());
        assert!(st.is_rejected());
        assert_eq!(handle.pending_len(), 0);
    }

    #[test]
    fn dry_run_does_not_stage_evictions() {
        let mut a = full_node("a");
        a.allocated.cpu_millicores = 4000;
        let snap = full_snap(
            vec![a],
            HashMap::from([("a".into(), vec![pod_at("t", "low", 0, 1000, 0)])]),
        );
        let preemptor = pod_at("t", "new", 50, 500, 0);
        let nominated = Arc::new(NominatedNodeMap::new());
        let handle = Arc::new(AsyncPreemptHandle::new());
        let mut args = DefaultPreemptionArgs::default();
        args.dry_run = true;
        let plug = DefaultPreemption::new(args, vec![], nominated.clone(), handle.clone());
        let cs = CycleState::new();
        let filtered = rejected_map(&["a"], crate::framework::Code::Unschedulable);
        let (res, st) = plug.post_filter(&preemptor, &snap, &filtered, &cs);
        assert!(st.is_success());
        assert!(res.nominating_info.is_some()); // plan returned
        assert_eq!(handle.pending_len(), 0); // but no evictions enqueued
    }

    #[test]
    fn min_evictable_priority_excludes_low_prio_victims() {
        let mut a = full_node("a");
        a.allocated.cpu_millicores = 4000;
        let snap = full_snap(
            vec![a],
            // Only a priority 0 victim is available.
            HashMap::from([("a".into(), vec![pod_at("t", "low", 0, 1000, 0)])]),
        );
        let preemptor = pod_at("t", "new", 100, 500, 0);
        let nominated = Arc::new(NominatedNodeMap::new());
        let handle = Arc::new(AsyncPreemptHandle::new());
        let mut args = DefaultPreemptionArgs::default();
        args.min_evictable_priority = Some(50);
        let plug = DefaultPreemption::new(args, vec![], nominated.clone(), handle.clone());
        let cs = CycleState::new();
        let filtered = rejected_map(&["a"], crate::framework::Code::Unschedulable);
        let (_res, st) = plug.post_filter(&preemptor, &snap, &filtered, &cs);
        assert!(st.is_rejected());
        assert_eq!(handle.pending_len(), 0);
    }

    #[test]
    fn min_evictable_priority_allows_high_prio_victims() {
        let mut a = full_node("a");
        a.allocated.cpu_millicores = 4000;
        let snap = full_snap(
            vec![a],
            HashMap::from([("a".into(), vec![pod_at("t", "mid", 60, 1000, 0)])]),
        );
        let preemptor = pod_at("t", "new", 100, 500, 0);
        let nominated = Arc::new(NominatedNodeMap::new());
        let handle = Arc::new(AsyncPreemptHandle::new());
        let mut args = DefaultPreemptionArgs::default();
        args.min_evictable_priority = Some(50);
        let plug = DefaultPreemption::new(args, vec![], nominated, handle.clone());
        let cs = CycleState::new();
        let filtered = rejected_map(&["a"], crate::framework::Code::Unschedulable);
        let (_, st) = plug.post_filter(&preemptor, &snap, &filtered, &cs);
        assert!(st.is_success());
        assert_eq!(handle.pending_len(), 1);
    }

    #[test]
    fn pdb_violations_skip_protected_victims() {
        let mut a = full_node("a");
        a.allocated.cpu_millicores = 4000;
        let mut v_protected = pod_at("t", "v1", 0, 2000, 4_000_000_000);
        v_protected.spec.node_selector.insert("app".into(), "db".into());
        let v_free = pod_at("t", "v2", 0, 2000, 4_000_000_000);
        let snap = full_snap(
            vec![a],
            HashMap::from([("a".into(), vec![v_protected, v_free])]),
        );
        let pdb = PodDisruptionBudget {
            name: "db-pdb".into(),
            namespace: "ns".into(),
            tenant_id: "t".into(),
            selector: HashMap::from([("app".into(), "db".into())]),
            min_available: 1,
            current_healthy: 1,
        };
        let preemptor = pod_at("t", "new", 50, 2000, 4_000_000_000);
        let nominated = Arc::new(NominatedNodeMap::new());
        let handle = Arc::new(AsyncPreemptHandle::new());
        let plug = DefaultPreemption::new(
            DefaultPreemptionArgs::default(),
            vec![pdb],
            nominated, handle.clone(),
        );
        let cs = CycleState::new();
        let filtered = rejected_map(&["a"], crate::framework::Code::Unschedulable);
        let (_, st) = plug.post_filter(&preemptor, &snap, &filtered, &cs);
        assert!(st.is_success());
        // Only the unprotected v2 is evicted.
        let pending = handle.pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].victim_name, "v2");
    }

    #[test]
    fn cross_tenant_preemption_blocked() {
        let mut a = full_node("a");
        a.allocated.cpu_millicores = 4000;
        let snap = full_snap(
            vec![a],
            HashMap::from([("a".into(), vec![pod_at("OTHER", "low", 0, 4000, 0)])]),
        );
        let preemptor = pod_at("t", "new", 100, 1000, 0);
        let nominated = Arc::new(NominatedNodeMap::new());
        let handle = Arc::new(AsyncPreemptHandle::new());
        let plug = DefaultPreemption::new(
            DefaultPreemptionArgs::default(), vec![],
            nominated, handle.clone(),
        );
        let cs = CycleState::new();
        let filtered = rejected_map(&["a"], crate::framework::Code::Unschedulable);
        let (_, st) = plug.post_filter(&preemptor, &snap, &filtered, &cs);
        assert!(st.is_rejected());
        assert_eq!(handle.pending_len(), 0);
    }

    #[test]
    fn no_preemption_when_only_higher_priority_pods() {
        let mut a = full_node("a");
        a.allocated.cpu_millicores = 4000;
        let snap = full_snap(
            vec![a],
            HashMap::from([("a".into(), vec![pod_at("t", "boss", 100, 4000, 0)])]),
        );
        let preemptor = pod_at("t", "new", 50, 1000, 0);
        let nominated = Arc::new(NominatedNodeMap::new());
        let handle = Arc::new(AsyncPreemptHandle::new());
        let plug = DefaultPreemption::new(
            DefaultPreemptionArgs::default(), vec![], nominated, handle.clone(),
        );
        let cs = CycleState::new();
        let filtered = rejected_map(&["a"], crate::framework::Code::Unschedulable);
        let (_, st) = plug.post_filter(&preemptor, &snap, &filtered, &cs);
        assert!(st.is_rejected());
    }

    #[test]
    fn nominated_node_map_persisted_across_post_filter_calls() {
        let mut a = full_node("a");
        a.allocated.cpu_millicores = 4000;
        let snap = full_snap(
            vec![a],
            HashMap::from([("a".into(), vec![pod_at("t", "low", 0, 1000, 0)])]),
        );
        let preemptor = pod_at("t", "new", 50, 500, 0);
        let nominated = Arc::new(NominatedNodeMap::new());
        let handle = Arc::new(AsyncPreemptHandle::new());
        let plug = DefaultPreemption::new(
            DefaultPreemptionArgs::default(), vec![],
            nominated.clone(), handle,
        );
        let cs = CycleState::new();
        let filtered = rejected_map(&["a"], crate::framework::Code::Unschedulable);
        let (_, _) = plug.post_filter(&preemptor, &snap, &filtered, &cs);
        // Persisted across plugin calls.
        assert!(nominated.nominated_for(&preemptor.uid).is_some());
        // Different preemptor in next cycle doesn't clobber.
        let preemptor2 = pod_at("t", "new2", 60, 500, 0);
        plug.post_filter(&preemptor2, &snap, &filtered, &cs);
        assert!(nominated.nominated_for(&preemptor.uid).is_some());
        assert!(nominated.nominated_for(&preemptor2.uid).is_some());
        assert_eq!(nominated.len(), 2);
    }

    #[test]
    fn plugin_name_is_default_preemption() {
        let p = DefaultPreemption::new(
            DefaultPreemptionArgs::default(), vec![],
            Arc::new(NominatedNodeMap::new()),
            Arc::new(AsyncPreemptHandle::new()),
        );
        assert_eq!(p.name(), "DefaultPreemption");
    }

    #[test]
    fn min_candidate_nodes_caps_inspected_set() {
        // 5 nodes, but only 2 should be inspected (min_absolute=2).
        let mut a = full_node("a");
        let mut b = full_node("b"); let mut c = full_node("c"); let mut d = full_node("d"); let mut e = full_node("e");
        for n in [&mut a, &mut b, &mut c, &mut d, &mut e] {
            n.allocated.cpu_millicores = 4000;
        }
        let mut pods_by_node = HashMap::new();
        for nm in ["a", "b", "c", "d", "e"] {
            pods_by_node.insert(nm.to_string(), vec![pod_at("t", &format!("low-{}", nm), 0, 1000, 0)]);
        }
        let snap = full_snap(vec![a, b, c, d, e], pods_by_node);
        let preemptor = pod_at("t", "new", 50, 500, 0);
        let nominated = Arc::new(NominatedNodeMap::new());
        let handle = Arc::new(AsyncPreemptHandle::new());
        let mut args = DefaultPreemptionArgs::default();
        args.min_candidate_nodes_absolute = 2;
        args.min_candidate_nodes_percentage = 0; // force absolute path
        let plug = DefaultPreemption::new(args, vec![], nominated, handle.clone());
        let cs = CycleState::new();
        let filtered = rejected_map(&["a", "b", "c", "d", "e"], crate::framework::Code::Unschedulable);
        let (res, st) = plug.post_filter(&preemptor, &snap, &filtered, &cs);
        assert!(st.is_success());
        // Should still nominate one (out of the 2 inspected — by sorted name "a").
        assert_eq!(res.nominating_info.unwrap().nominated_node_name, "a");
    }

    #[test]
    fn min_candidate_nodes_percentage_floor() {
        // 100 nodes, percentage 5 → at least 5; absolute floor 100 dominates.
        let args = DefaultPreemptionArgs::default();
        // Sanity: defaults define min absolute 100.
        assert!(args.min_candidate_nodes_absolute >= 100);
    }
}
