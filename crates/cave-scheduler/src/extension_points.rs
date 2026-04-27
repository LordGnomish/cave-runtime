//! Plugin extension-point traits beyond `Filter` and `Score`.
//!
//! Cite: kubernetes/kubernetes v1.31.0
//!   pkg/scheduler/framework/interface.go
//!
//! cave-scheduler's `framework.rs` already defines `FilterPlugin` and
//! `ScorePlugin`. The remaining 11 extension points live here so existing
//! plugin authors don't have to touch `framework.rs` to find them, and so
//! every trait that takes [`CycleState`] sits next to the type it uses.
//!
//! Extension-point ordering inside one cycle:
//!
//! ```text
//! QueueSort → PreEnqueue → PreFilter → Filter → PostFilter → PreScore →
//! Score → NormalizeScore → Reserve → Permit → PreBind → Bind → PostBind
//! ```
//!
//! `Reserve` is paired with `Unreserve` for rollback when a later step fails.
//! `Bind` plugins run in registration order; the first that does not return
//! `Skip` wins.

use crate::cycle_state::CycleState;
use crate::framework::{ClusterSnapshot, Pod, Status};
use std::cmp::Ordering;

/// A bag of node names the framework should consider for this pod.
///
/// `None` means "every node" (identity element). Multiple PreFilter plugins'
/// results are intersected.
#[derive(Debug, Clone, Default)]
pub struct PreFilterResult {
    pub node_names: Option<std::collections::BTreeSet<String>>,
}

impl PreFilterResult {
    pub fn all_nodes() -> Self {
        Self { node_names: None }
    }

    pub fn restrict(nodes: impl IntoIterator<Item = String>) -> Self {
        Self { node_names: Some(nodes.into_iter().collect()) }
    }

    /// Intersect with another result. `None` (all nodes) is the identity.
    pub fn merge(&self, other: &PreFilterResult) -> PreFilterResult {
        match (&self.node_names, &other.node_names) {
            (None, None) => PreFilterResult::all_nodes(),
            (Some(a), None) => PreFilterResult { node_names: Some(a.clone()) },
            (None, Some(b)) => PreFilterResult { node_names: Some(b.clone()) },
            (Some(a), Some(b)) => {
                PreFilterResult { node_names: Some(a.intersection(b).cloned().collect()) }
            }
        }
    }
}

/// Per-node verdict accumulated during Filter — used as input to PostFilter.
#[derive(Debug, Clone, Default)]
pub struct NodeToStatusMap {
    inner: std::collections::HashMap<String, Status>,
}

impl NodeToStatusMap {
    pub fn new() -> Self { Self::default() }

    pub fn set(&mut self, node: impl Into<String>, status: Status) {
        self.inner.insert(node.into(), status);
    }

    pub fn get(&self, node: &str) -> Option<&Status> { self.inner.get(node) }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &Status)> { self.inner.iter() }

    pub fn len(&self) -> usize { self.inner.len() }
    pub fn is_empty(&self) -> bool { self.inner.is_empty() }

    pub fn rejected_nodes(&self) -> std::collections::BTreeSet<String> {
        self.inner.iter()
            .filter_map(|(n, s)| if s.is_rejected() { Some(n.clone()) } else { None })
            .collect()
    }
}

/// Outcome of a PostFilter run (preemption).
#[derive(Debug, Clone, Default)]
pub struct PostFilterResult {
    pub nominating_info: Option<NominatingInfo>,
}

impl PostFilterResult {
    pub fn nominate(node: impl Into<String>) -> Self {
        Self {
            nominating_info: Some(NominatingInfo {
                nominated_node_name: node.into(),
                nominating_mode: NominatingMode::Override,
            }),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NominatingInfo {
    pub nominated_node_name: String,
    pub nominating_mode: NominatingMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NominatingMode {
    Noop,
    Override,
}

// ─── trait surfaces ────────────────────────────────────────────────────────

/// Order pods in the active queue. The first registered QueueSort plugin wins
/// (kube-scheduler validates "exactly one" at config time; we mirror by using
/// the first one).
pub trait QueueSortPlugin: Send + Sync {
    fn name(&self) -> &str;
    fn less(&self, a: &Pod, b: &Pod) -> Ordering;
}

/// Gate pod admission to the active queue. Returning `Pending` keeps the pod
/// in the unschedulable subqueue until a relevant cluster event re-enqueues it.
pub trait PreEnqueuePlugin: Send + Sync {
    fn name(&self) -> &str;
    fn pre_enqueue(&self, pod: &Pod) -> Status;
}

/// Compute per-pod metadata once before per-node Filter runs. May return a
/// [`PreFilterResult`] that restricts the candidate node set, or [`Status::skip`]
/// to short-circuit the matching Filter plugin.
pub trait PreFilterPlugin: Send + Sync {
    fn name(&self) -> &str;
    fn pre_filter(
        &self,
        pod: &Pod,
        snapshot: &ClusterSnapshot,
        state: &CycleState,
    ) -> (PreFilterResult, Status);
}

/// Runs only when Filter rejected every node — preemption belongs here.
pub trait PostFilterPlugin: Send + Sync {
    fn name(&self) -> &str;
    fn post_filter(
        &self,
        pod: &Pod,
        snapshot: &ClusterSnapshot,
        filtered: &NodeToStatusMap,
        state: &CycleState,
    ) -> (PostFilterResult, Status);
}

/// Compute per-pod metadata for Score. Returning `Skip` signals the matching
/// Score plugin to be skipped (kube-scheduler optimisation).
pub trait PreScorePlugin: Send + Sync {
    fn name(&self) -> &str;
    fn pre_score(&self, pod: &Pod, snapshot: &ClusterSnapshot, state: &CycleState) -> Status;
}

/// NormalizeScore — rescale a plugin's per-node score map. `[0, 100]` final
/// values are required.
pub trait ScoreExtensions: Send + Sync {
    fn normalize_score(
        &self,
        pod: &Pod,
        scores: &mut [(String, i64)],
        state: &CycleState,
    ) -> Status;
}

/// Reserve resources on the chosen node before Bind. Paired with `unreserve`:
/// if any later extension point fails, every Reserve plugin's `unreserve` is
/// called in reverse order to roll back.
pub trait ReservePlugin: Send + Sync {
    fn name(&self) -> &str;
    fn reserve(&self, pod: &Pod, node: &str, state: &CycleState) -> Status;
    fn unreserve(&self, pod: &Pod, node: &str, state: &CycleState);
}

/// Gate the transition from Reserve to Bind.
///
/// Return values:
/// - `Success` — proceed immediately.
/// - `Wait` carrying a non-zero duration — block bind for at most that long;
///   the framework picks the *largest* wait across plugins.
/// - `Unschedulable` / `Error` — abort the cycle and run Unreserve.
pub trait PermitPlugin: Send + Sync {
    fn name(&self) -> &str;
    fn permit(&self, pod: &Pod, node: &str, state: &CycleState) -> Status;
}

/// Work that must complete before Bind — typically volume provisioning.
pub trait PreBindPlugin: Send + Sync {
    fn name(&self) -> &str;
    fn pre_bind(&self, pod: &Pod, node: &str, state: &CycleState) -> Status;
}

/// Actually bind the pod to the node. Only one Bind plugin runs per pod —
/// the framework picks the first plugin that does not return `Skip`.
pub trait BindPlugin: Send + Sync {
    fn name(&self) -> &str;
    fn bind(&self, pod: &Pod, node: &str, state: &CycleState) -> Status;
}

/// Informational hook after a successful Bind. Best-effort, no status.
pub trait PostBindPlugin: Send + Sync {
    fn name(&self) -> &str;
    fn post_bind(&self, pod: &Pod, node: &str, state: &CycleState);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_filter_result_intersects() {
        let a = PreFilterResult::restrict(["n1".into(), "n2".into(), "n3".into()]);
        let b = PreFilterResult::restrict(["n2".into(), "n3".into(), "n4".into()]);
        let m = a.merge(&b);
        let names = m.node_names.unwrap();
        assert_eq!(names.len(), 2);
        assert!(names.contains("n2"));
        assert!(names.contains("n3"));
    }

    #[test]
    fn pre_filter_result_all_is_identity() {
        let a = PreFilterResult::all_nodes();
        let b = PreFilterResult::restrict(["n2".into()]);
        let m = a.merge(&b);
        assert_eq!(m.node_names.as_ref().map(|s| s.len()), Some(1));
        assert!(m.node_names.unwrap().contains("n2"));
    }

    #[test]
    fn pre_filter_result_both_all_stays_all() {
        let a = PreFilterResult::all_nodes();
        let m = a.merge(&PreFilterResult::all_nodes());
        assert!(m.node_names.is_none());
    }

    #[test]
    fn node_to_status_map_tracks_rejections() {
        let mut m = NodeToStatusMap::new();
        m.set("a", Status::success("p"));
        m.set("b", Status::unschedulable("p", "nope"));
        m.set("c", Status::unresolvable("p", "hard"));
        let r = m.rejected_nodes();
        assert!(!r.contains("a"));
        assert!(r.contains("b"));
        assert!(r.contains("c"));
        assert_eq!(r.len(), 2);
        assert_eq!(m.len(), 3);
    }

    #[test]
    fn post_filter_nominate_carries_node() {
        let r = PostFilterResult::nominate("n5");
        let info = r.nominating_info.unwrap();
        assert_eq!(info.nominated_node_name, "n5");
        assert_eq!(info.nominating_mode, NominatingMode::Override);
    }

    #[test]
    fn post_filter_default_is_no_nominator() {
        let r = PostFilterResult::default();
        assert!(r.nominating_info.is_none());
    }

    #[test]
    fn nominating_mode_default_unset() {
        let r = PostFilterResult::default();
        assert!(r.nominating_info.is_none());
    }

    /// Compile-time check: every trait is object-safe.
    #[test]
    fn extension_point_traits_are_object_safe() {
        fn _accept(
            _: Box<dyn QueueSortPlugin>,
            _: Box<dyn PreEnqueuePlugin>,
            _: Box<dyn PreFilterPlugin>,
            _: Box<dyn PostFilterPlugin>,
            _: Box<dyn PreScorePlugin>,
            _: Box<dyn ScoreExtensions>,
            _: Box<dyn ReservePlugin>,
            _: Box<dyn PermitPlugin>,
            _: Box<dyn PreBindPlugin>,
            _: Box<dyn BindPlugin>,
            _: Box<dyn PostBindPlugin>,
        ) {
        }
    }
}
