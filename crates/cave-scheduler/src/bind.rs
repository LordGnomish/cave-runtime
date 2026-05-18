// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bind plugins.
//!
//! Cite: kubernetes/kubernetes v1.31.0
//!   pkg/scheduler/framework/plugins/defaultbinder/default_binder.go

use crate::cycle_state::CycleState;
use crate::extension_points::{BindPlugin, PostBindPlugin, PreBindPlugin};
use crate::framework::{Pod, Status};
use std::sync::Mutex;

/// Default in-tree Binder — records the (pod_uid → node) decision so the
/// rest of cave-runtime can act on it. Upstream issues a Binding subresource
/// PATCH to apiserver; we accumulate decisions in memory and surface them via
/// `bound_decisions()`.
pub struct DefaultBinder {
    decisions: Mutex<Vec<BindDecision>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindDecision {
    pub pod_uid: String,
    pub pod_name: String,
    pub namespace: String,
    pub tenant_id: String,
    pub node_name: String,
}

impl Default for DefaultBinder {
    fn default() -> Self { Self::new() }
}

impl DefaultBinder {
    pub fn new() -> Self {
        Self { decisions: Mutex::new(Vec::new()) }
    }

    /// Snapshot every Bind decision recorded so far. Returned in registration
    /// order (chronological).
    pub fn bound_decisions(&self) -> Vec<BindDecision> {
        self.decisions.lock().expect("DefaultBinder poisoned").clone()
    }

    pub fn clear(&self) {
        self.decisions.lock().expect("DefaultBinder poisoned").clear();
    }

    pub fn count(&self) -> usize {
        self.decisions.lock().expect("DefaultBinder poisoned").len()
    }
}

impl BindPlugin for DefaultBinder {
    fn name(&self) -> &str { "DefaultBinder" }
    fn bind(&self, pod: &Pod, node: &str, _state: &CycleState) -> Status {
        if node.is_empty() {
            return Status::error("DefaultBinder", "empty node name");
        }
        let mut log = self.decisions.lock().expect("DefaultBinder poisoned");
        log.push(BindDecision {
            pod_uid: pod.uid.clone(),
            pod_name: pod.name.clone(),
            namespace: pod.namespace.clone(),
            tenant_id: pod.tenant_id.clone(),
            node_name: node.into(),
        });
        Status::success("DefaultBinder")
    }
}

/// Test-friendly NoOp binder that returns `Skip` so a later binder can claim
/// the pod (matches upstream pattern where extender binders run before the
/// default and can return Skip to fall through).
pub struct SkipBinder {
    pub plugin_name: String,
}

impl SkipBinder {
    pub fn new(name: impl Into<String>) -> Self { Self { plugin_name: name.into() } }
}

impl BindPlugin for SkipBinder {
    fn name(&self) -> &str { &self.plugin_name }
    fn bind(&self, _pod: &Pod, _node: &str, _state: &CycleState) -> Status {
        Status::skip(&self.plugin_name)
    }
}

/// Best-effort PostBind that records `(pod_uid, node)` events. Used by
/// observability hooks.
pub struct PostBindLogger {
    events: Mutex<Vec<(String, String)>>,
}

impl Default for PostBindLogger {
    fn default() -> Self { Self::new() }
}

impl PostBindLogger {
    pub fn new() -> Self { Self { events: Mutex::new(Vec::new()) } }
    pub fn events(&self) -> Vec<(String, String)> {
        self.events.lock().expect("PostBindLogger poisoned").clone()
    }
}

impl PostBindPlugin for PostBindLogger {
    fn name(&self) -> &str { "PostBindLogger" }
    fn post_bind(&self, pod: &Pod, node: &str, _state: &CycleState) {
        self.events.lock().expect("PostBindLogger poisoned")
            .push((pod.uid.clone(), node.into()));
    }
}

/// Identity PreBind — succeeds for every pod. Useful default when no PreBind
/// work is required (most clusters without volume binding).
pub struct NoopPreBinder;

impl PreBindPlugin for NoopPreBinder {
    fn name(&self) -> &str { "NoopPreBinder" }
    fn pre_bind(&self, _pod: &Pod, _node: &str, _state: &CycleState) -> Status {
        Status::success("NoopPreBinder")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::Pod;

    #[test]
    fn default_binder_records_decision() {
        let b = DefaultBinder::new();
        let s = b.bind(&Pod::new("t", "ns", "p"), "n1", &CycleState::new());
        assert!(s.is_success());
        let d = b.bound_decisions();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].node_name, "n1");
        assert_eq!(d[0].pod_name, "p");
        assert_eq!(d[0].tenant_id, "t");
    }

    #[test]
    fn default_binder_rejects_empty_node() {
        let b = DefaultBinder::new();
        let s = b.bind(&Pod::new("t", "ns", "p"), "", &CycleState::new());
        assert!(s.is_error());
        assert_eq!(b.count(), 0);
    }

    #[test]
    fn default_binder_clear_resets_log() {
        let b = DefaultBinder::new();
        b.bind(&Pod::new("t", "ns", "p1"), "n1", &CycleState::new());
        b.bind(&Pod::new("t", "ns", "p2"), "n2", &CycleState::new());
        assert_eq!(b.count(), 2);
        b.clear();
        assert_eq!(b.count(), 0);
    }

    #[test]
    fn default_binder_preserves_order() {
        let b = DefaultBinder::new();
        for i in 0..5 {
            b.bind(&Pod::new("t", "ns", &format!("p{}", i)), "n1", &CycleState::new());
        }
        let d = b.bound_decisions();
        for (i, dec) in d.iter().enumerate() {
            assert_eq!(dec.pod_name, format!("p{}", i));
        }
    }

    #[test]
    fn skip_binder_returns_skip() {
        let s = SkipBinder::new("X").bind(&Pod::new("t", "ns", "p"), "n1", &CycleState::new());
        assert!(s.is_skip());
        assert_eq!(s.plugin, "X");
    }

    #[test]
    fn post_bind_logger_records_events() {
        let l = PostBindLogger::new();
        l.post_bind(&Pod::new("t", "ns", "p"), "n1", &CycleState::new());
        let evts = l.events();
        assert_eq!(evts.len(), 1);
        assert_eq!(evts[0].1, "n1");
    }

    #[test]
    fn noop_pre_binder_succeeds() {
        let s = NoopPreBinder.pre_bind(&Pod::new("t", "ns", "p"), "n1", &CycleState::new());
        assert!(s.is_success());
    }

    #[test]
    fn default_binder_carries_tenant_id() {
        let b = DefaultBinder::new();
        b.bind(&Pod::new("acme", "default", "web"), "node-1", &CycleState::new());
        let d = b.bound_decisions();
        assert_eq!(d[0].tenant_id, "acme");
        assert_eq!(d[0].namespace, "default");
        assert_eq!(d[0].pod_uid, "acme-default-web");
    }
}
