// SPDX-License-Identifier: AGPL-3.0-or-later
//! ClusterEvent + QueueingHint (KEP-4247) — event-driven re-queueing.
//!
//! Cite: kubernetes/kubernetes v1.31.0
//!   pkg/scheduler/eventhandlers.go
//!   pkg/scheduler/framework/types.go (ClusterEvent, ActionType)
//!   pkg/scheduler/framework/interface.go (QueueingHintFn)
//!
//! ## Concept
//!
//! When a pod hits the unschedulable subqueue, the scheduler waits for a
//! cluster event that *might* make it schedulable again. Plugins register
//! `QueueingHintFn`s — for a given (pod, event) pair, the hint returns
//! `Queue` (re-enqueue), `QueueImmediately` (skip backoff), or `QueueSkip`
//! (event is not relevant for this pod).
//!
//! The scheduler aggregates hints across plugins per event:
//!
//! - If any hint says `QueueImmediately`, the pod skips backoff.
//! - Else if any hint says `Queue`, the pod re-enters the active queue with
//!   the usual exponential backoff.
//! - Else (`QueueSkip` from every plugin), the pod stays unschedulable.

use crate::framework::Pod;
use std::collections::HashMap;

/// Resource that triggered the event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceType {
    Pod,
    Node,
    PersistentVolume,
    PersistentVolumeClaim,
    StorageClass,
    CSIStorageCapacity,
    CSINode,
    CSIDriver,
    ResourceClaim,
    Service,
    PodSchedulingContext,
    DeviceClass,
    Wildcard,
}

/// What happened to that resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActionType {
    Add,
    Update,
    Delete,
    /// Granular-update flavours used by Node events to keep hint dispatch tight.
    UpdateNodeLabel,
    UpdateNodeTaint,
    UpdateNodeAllocatable,
    UpdateNodeCondition,
    UpdateNodeAnnotation,
    All,
}

/// One cluster event — what happened to which resource.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClusterEvent {
    pub resource: ResourceType,
    pub action: ActionType,
    /// Opaque label of what changed; plugins may use it to tighten matches.
    pub note: Option<String>,
}

impl ClusterEvent {
    pub fn pod_added() -> Self { Self { resource: ResourceType::Pod, action: ActionType::Add, note: None } }
    pub fn pod_deleted() -> Self { Self { resource: ResourceType::Pod, action: ActionType::Delete, note: None } }
    pub fn node_added() -> Self { Self { resource: ResourceType::Node, action: ActionType::Add, note: None } }
    pub fn node_label_updated() -> Self {
        Self { resource: ResourceType::Node, action: ActionType::UpdateNodeLabel, note: None }
    }
    pub fn pv_added() -> Self {
        Self { resource: ResourceType::PersistentVolume, action: ActionType::Add, note: None }
    }
    pub fn resource_claim_updated() -> Self {
        Self { resource: ResourceType::ResourceClaim, action: ActionType::Update, note: None }
    }

    /// True when the event matches a hint registration's filter. The filter is
    /// a simpler `(resource, action_or_All)` pair — `All` covers every action
    /// of that resource type, and `Wildcard` covers every event.
    pub fn matches(&self, want_resource: ResourceType, want_action: ActionType) -> bool {
        if want_resource == ResourceType::Wildcard { return true; }
        if want_resource != self.resource { return false; }
        if want_action == ActionType::All { return true; }
        // Granular update flavours all roll up under Update.
        if want_action == ActionType::Update {
            return matches!(self.action,
                ActionType::Update
                | ActionType::UpdateNodeLabel
                | ActionType::UpdateNodeTaint
                | ActionType::UpdateNodeAllocatable
                | ActionType::UpdateNodeCondition
                | ActionType::UpdateNodeAnnotation
            );
        }
        self.action == want_action
    }
}

/// Per-(pod, event) hint result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueingHint {
    /// Event is not relevant to this pod's unschedulable reason.
    QueueSkip,
    /// Re-enqueue with the usual exponential backoff.
    Queue,
    /// Re-enqueue immediately, skipping backoff.
    QueueImmediately,
}

impl QueueingHint {
    /// Combine two hints — caller takes the most aggressive verdict.
    pub fn combine(self, other: QueueingHint) -> QueueingHint {
        use QueueingHint::*;
        match (self, other) {
            (QueueImmediately, _) | (_, QueueImmediately) => QueueImmediately,
            (Queue, _) | (_, Queue) => Queue,
            _ => QueueSkip,
        }
    }
}

/// Trait for per-plugin hint functions. The blanket impl on `Fn(&Pod, &ClusterEvent)`
/// means callers can register raw closures.
pub trait QueueingHintFn: Send + Sync {
    fn hint(&self, pod: &Pod, event: &ClusterEvent) -> QueueingHint;
}

impl<F: Fn(&Pod, &ClusterEvent) -> QueueingHint + Send + Sync> QueueingHintFn for F {
    fn hint(&self, pod: &Pod, event: &ClusterEvent) -> QueueingHint {
        (self)(pod, event)
    }
}

/// One registration: a plugin name + the (resource, action) filter it cares
/// about + the hint function itself.
pub struct HintRegistration {
    pub plugin: String,
    pub want_resource: ResourceType,
    pub want_action: ActionType,
    pub hint: Box<dyn QueueingHintFn>,
}

/// Registry indexed by `(resource, action)` for fast event dispatch.
#[derive(Default)]
pub struct HintRegistry {
    entries: Vec<HintRegistration>,
}

impl HintRegistry {
    pub fn new() -> Self { Self::default() }

    /// Register a hint. The same plugin may register multiple hints under
    /// different filters.
    pub fn register(&mut self, reg: HintRegistration) {
        self.entries.push(reg);
    }

    pub fn len(&self) -> usize { self.entries.len() }
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }

    /// Aggregate verdict for `(pod, event)` across every registration whose
    /// filter matches the event.
    pub fn aggregate_hint(&self, pod: &Pod, event: &ClusterEvent) -> QueueingHint {
        let mut acc = QueueingHint::QueueSkip;
        for reg in &self.entries {
            if !event.matches(reg.want_resource, reg.want_action) { continue; }
            acc = acc.combine(reg.hint.hint(pod, event));
            if acc == QueueingHint::QueueImmediately { break; }
        }
        acc
    }

    /// Per-plugin verdict map: `plugin → hint`. Used for diagnostics.
    pub fn per_plugin_hints(&self, pod: &Pod, event: &ClusterEvent) -> HashMap<String, QueueingHint> {
        let mut map: HashMap<String, QueueingHint> = HashMap::new();
        for reg in &self.entries {
            if !event.matches(reg.want_resource, reg.want_action) { continue; }
            let h = reg.hint.hint(pod, event);
            map.entry(reg.plugin.clone())
                .and_modify(|cur| *cur = cur.combine(h))
                .or_insert(h);
        }
        map
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pod(name: &str) -> Pod { Pod::new("t", "ns", name) }

    // ── ClusterEvent.matches ──────────────────────────────────────────────

    #[test]
    fn matches_exact_resource_and_action() {
        let e = ClusterEvent::pod_added();
        assert!(e.matches(ResourceType::Pod, ActionType::Add));
        assert!(!e.matches(ResourceType::Pod, ActionType::Delete));
        assert!(!e.matches(ResourceType::Node, ActionType::Add));
    }

    #[test]
    fn matches_wildcard_resource() {
        let e = ClusterEvent::pod_added();
        assert!(e.matches(ResourceType::Wildcard, ActionType::Add));
        assert!(e.matches(ResourceType::Wildcard, ActionType::All));
    }

    #[test]
    fn matches_action_all() {
        let e = ClusterEvent::pod_added();
        assert!(e.matches(ResourceType::Pod, ActionType::All));
    }

    #[test]
    fn matches_node_label_update_via_update() {
        let e = ClusterEvent::node_label_updated();
        // Generic Update hint covers UpdateNodeLabel events.
        assert!(e.matches(ResourceType::Node, ActionType::Update));
        // Specific UpdateNodeLabel hint also covers them.
        assert!(e.matches(ResourceType::Node, ActionType::UpdateNodeLabel));
        // But UpdateNodeTaint hint does not.
        assert!(!e.matches(ResourceType::Node, ActionType::UpdateNodeTaint));
    }

    #[test]
    fn matches_action_update_does_not_match_add() {
        let e = ClusterEvent::pod_added();
        assert!(!e.matches(ResourceType::Pod, ActionType::Update));
    }

    // ── QueueingHint.combine ──────────────────────────────────────────────

    #[test]
    fn combine_immediately_dominates() {
        use QueueingHint::*;
        assert_eq!(QueueImmediately.combine(QueueSkip), QueueImmediately);
        assert_eq!(QueueSkip.combine(QueueImmediately), QueueImmediately);
        assert_eq!(QueueImmediately.combine(Queue), QueueImmediately);
    }

    #[test]
    fn combine_queue_beats_skip() {
        use QueueingHint::*;
        assert_eq!(Queue.combine(QueueSkip), Queue);
        assert_eq!(QueueSkip.combine(Queue), Queue);
    }

    #[test]
    fn combine_skip_skip_is_skip() {
        assert_eq!(QueueingHint::QueueSkip.combine(QueueingHint::QueueSkip), QueueingHint::QueueSkip);
    }

    // ── HintRegistry.aggregate_hint ──────────────────────────────────────

    #[test]
    fn aggregate_returns_skip_when_no_registrations() {
        let r = HintRegistry::new();
        assert_eq!(r.aggregate_hint(&pod("p"), &ClusterEvent::pod_added()), QueueingHint::QueueSkip);
    }

    #[test]
    fn aggregate_picks_most_aggressive_hint() {
        let mut r = HintRegistry::new();
        r.register(HintRegistration {
            plugin: "A".into(),
            want_resource: ResourceType::Node,
            want_action: ActionType::Add,
            hint: Box::new(|_p: &Pod, _e: &ClusterEvent| QueueingHint::Queue),
        });
        r.register(HintRegistration {
            plugin: "B".into(),
            want_resource: ResourceType::Node,
            want_action: ActionType::Add,
            hint: Box::new(|_p: &Pod, _e: &ClusterEvent| QueueingHint::QueueImmediately),
        });
        r.register(HintRegistration {
            plugin: "C".into(),
            want_resource: ResourceType::Node,
            want_action: ActionType::Add,
            hint: Box::new(|_p: &Pod, _e: &ClusterEvent| QueueingHint::QueueSkip),
        });
        assert_eq!(r.aggregate_hint(&pod("p"), &ClusterEvent::node_added()),
                   QueueingHint::QueueImmediately);
    }

    #[test]
    fn aggregate_filters_by_resource_and_action() {
        let mut r = HintRegistry::new();
        r.register(HintRegistration {
            plugin: "OnlyPod".into(),
            want_resource: ResourceType::Pod,
            want_action: ActionType::Add,
            hint: Box::new(|_p: &Pod, _e: &ClusterEvent| QueueingHint::Queue),
        });
        // Node event → no hint matches → QueueSkip.
        assert_eq!(r.aggregate_hint(&pod("p"), &ClusterEvent::node_added()), QueueingHint::QueueSkip);
        // Pod event → hint runs → Queue.
        assert_eq!(r.aggregate_hint(&pod("p"), &ClusterEvent::pod_added()), QueueingHint::Queue);
    }

    #[test]
    fn aggregate_short_circuits_on_immediately() {
        let mut r = HintRegistry::new();
        r.register(HintRegistration {
            plugin: "A".into(),
            want_resource: ResourceType::Wildcard,
            want_action: ActionType::All,
            hint: Box::new(|_p: &Pod, _e: &ClusterEvent| QueueingHint::QueueImmediately),
        });
        r.register(HintRegistration {
            plugin: "B-should-not-run".into(),
            want_resource: ResourceType::Wildcard,
            want_action: ActionType::All,
            hint: Box::new(|_p: &Pod, _e: &ClusterEvent| panic!("must not run after QueueImmediately")),
        });
        // No panic → second hint was skipped.
        assert_eq!(r.aggregate_hint(&pod("p"), &ClusterEvent::pod_added()),
                   QueueingHint::QueueImmediately);
    }

    // ── Per-plugin hint map ──────────────────────────────────────────────

    #[test]
    fn per_plugin_hints_records_each_plugin() {
        let mut r = HintRegistry::new();
        r.register(HintRegistration {
            plugin: "NodeAffinity".into(),
            want_resource: ResourceType::Node,
            want_action: ActionType::UpdateNodeLabel,
            hint: Box::new(|_p: &Pod, _e: &ClusterEvent| QueueingHint::Queue),
        });
        r.register(HintRegistration {
            plugin: "VolumeBinding".into(),
            want_resource: ResourceType::PersistentVolume,
            want_action: ActionType::Add,
            hint: Box::new(|_p: &Pod, _e: &ClusterEvent| QueueingHint::Queue),
        });
        let m = r.per_plugin_hints(&pod("p"), &ClusterEvent::node_label_updated());
        assert_eq!(m.get("NodeAffinity"), Some(&QueueingHint::Queue));
        assert!(!m.contains_key("VolumeBinding"));
    }

    #[test]
    fn per_plugin_hint_combines_when_same_plugin_registers_twice() {
        let mut r = HintRegistry::new();
        r.register(HintRegistration {
            plugin: "X".into(),
            want_resource: ResourceType::Wildcard,
            want_action: ActionType::All,
            hint: Box::new(|_p: &Pod, _e: &ClusterEvent| QueueingHint::Queue),
        });
        r.register(HintRegistration {
            plugin: "X".into(),
            want_resource: ResourceType::Wildcard,
            want_action: ActionType::All,
            hint: Box::new(|_p: &Pod, _e: &ClusterEvent| QueueingHint::QueueImmediately),
        });
        let m = r.per_plugin_hints(&pod("p"), &ClusterEvent::pod_added());
        assert_eq!(m.get("X"), Some(&QueueingHint::QueueImmediately));
    }

    // ── End-to-end requeue scenario ──────────────────────────────────────

    #[test]
    fn pv_added_queues_pod_blocked_on_volume() {
        let mut r = HintRegistry::new();
        r.register(HintRegistration {
            plugin: "VolumeBinding".into(),
            want_resource: ResourceType::PersistentVolume,
            want_action: ActionType::Add,
            hint: Box::new(|_p: &Pod, _e: &ClusterEvent| QueueingHint::Queue),
        });
        let h = r.aggregate_hint(&pod("p"), &ClusterEvent::pv_added());
        assert_eq!(h, QueueingHint::Queue);
    }

    #[test]
    fn node_taint_update_can_unblock_pod() {
        let mut r = HintRegistry::new();
        // Pod is unschedulable due to TaintToleration; only NodeTaint updates
        // can unblock it. Other Node updates should not.
        r.register(HintRegistration {
            plugin: "TaintToleration".into(),
            want_resource: ResourceType::Node,
            want_action: ActionType::UpdateNodeTaint,
            hint: Box::new(|_p: &Pod, _e: &ClusterEvent| QueueingHint::Queue),
        });
        let label_evt = ClusterEvent::node_label_updated();
        assert_eq!(r.aggregate_hint(&pod("p"), &label_evt), QueueingHint::QueueSkip);
        let taint_evt = ClusterEvent {
            resource: ResourceType::Node,
            action: ActionType::UpdateNodeTaint,
            note: None,
        };
        assert_eq!(r.aggregate_hint(&pod("p"), &taint_evt), QueueingHint::Queue);
    }

    #[test]
    fn closure_hint_function_is_object_safe() {
        let mut r = HintRegistry::new();
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count2 = count.clone();
        r.register(HintRegistration {
            plugin: "Counter".into(),
            want_resource: ResourceType::Pod,
            want_action: ActionType::Add,
            hint: Box::new(move |_p: &Pod, _e: &ClusterEvent| {
                count2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                QueueingHint::Queue
            }),
        });
        r.aggregate_hint(&pod("p"), &ClusterEvent::pod_added());
        r.aggregate_hint(&pod("p"), &ClusterEvent::pod_added());
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    // ── Registry bookkeeping ─────────────────────────────────────────────

    #[test]
    fn registry_len_and_empty() {
        let r = HintRegistry::new();
        assert!(r.is_empty());
        let mut r = HintRegistry::new();
        r.register(HintRegistration {
            plugin: "A".into(),
            want_resource: ResourceType::Pod,
            want_action: ActionType::Add,
            hint: Box::new(|_p: &Pod, _e: &ClusterEvent| QueueingHint::Queue),
        });
        assert_eq!(r.len(), 1);
        assert!(!r.is_empty());
    }

    // ── ClusterEvent helper constructors ─────────────────────────────────

    #[test]
    fn helpers_construct_correct_event() {
        assert_eq!(ClusterEvent::pod_added().action, ActionType::Add);
        assert_eq!(ClusterEvent::pod_deleted().action, ActionType::Delete);
        assert_eq!(ClusterEvent::node_added().resource, ResourceType::Node);
        assert_eq!(ClusterEvent::pv_added().resource, ResourceType::PersistentVolume);
        assert_eq!(ClusterEvent::resource_claim_updated().action, ActionType::Update);
    }

    #[test]
    fn cluster_event_equality_used_for_hashing() {
        let mut s: std::collections::HashSet<ClusterEvent> = std::collections::HashSet::new();
        s.insert(ClusterEvent::pod_added());
        s.insert(ClusterEvent::pod_added());
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn note_distinguishes_otherwise_identical_events() {
        let mut a = ClusterEvent::pod_added();
        let mut b = ClusterEvent::pod_added();
        a.note = Some("foo".into());
        b.note = Some("bar".into());
        assert_ne!(a, b);
    }

    #[test]
    fn aggregate_with_wildcard_registration() {
        let mut r = HintRegistry::new();
        r.register(HintRegistration {
            plugin: "Catch-all".into(),
            want_resource: ResourceType::Wildcard,
            want_action: ActionType::All,
            hint: Box::new(|_p: &Pod, _e: &ClusterEvent| QueueingHint::Queue),
        });
        // Any event matches.
        assert_eq!(r.aggregate_hint(&pod("p"), &ClusterEvent::pv_added()), QueueingHint::Queue);
        assert_eq!(r.aggregate_hint(&pod("p"), &ClusterEvent::pod_deleted()), QueueingHint::Queue);
    }
}
