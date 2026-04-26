//! Manager loop wiring.
//!
//! Mirrors `cmd/kube-controller-manager/app/controllermanager.go` plus the
//! `client-go/tools/workqueue` and `client-go/informers` plumbing the
//! controllers actually consume:
//!
//! * `EventSource` — generates `Event::{Add, Update, Delete}` records
//!   tagged with the originating tenant.
//! * `Workqueue` — rate-limited, deduplicated `(kind, namespace, name)`
//!   keys; idempotent `add`, `get` returns at most once until `done`.
//! * `SyncController` — stitches a source to a queue and drains it via a
//!   `Reconciler::reconcile` callback, tracking per-tenant counters.
//!
//! Multi-tenancy is preserved end-to-end: every `Event` carries a
//! [`TenantId`], and `SyncController::run_one_pass` refuses to dispatch a
//! key whose tenant doesn't match the controller's owner.

use crate::types::{Cite, ControllerError, Reconcile, TenantId};
use serde::Serialize;
use std::collections::{HashSet, VecDeque};

/// Stable key identifying a Kubernetes object across an event stream.
#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize)]
pub struct ObjectKey {
    pub tenant: TenantId,
    pub kind: &'static str,
    pub namespace: String,
    pub name: String,
}

impl ObjectKey {
    pub fn new(
        tenant: TenantId,
        kind: &'static str,
        namespace: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self { tenant, kind, namespace: namespace.into(), name: name.into() }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub enum Event {
    Add(ObjectKey),
    Update(ObjectKey),
    Delete(ObjectKey),
}

impl Event {
    pub fn key(&self) -> &ObjectKey {
        match self {
            Event::Add(k) | Event::Update(k) | Event::Delete(k) => k,
        }
    }
}

/// In-memory event source — the test harness substitute for
/// `cache.Reflector`. Real informers populate the same queue.
#[derive(Debug, Default)]
pub struct EventSource {
    buffer: VecDeque<Event>,
}

impl EventSource {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn push(&mut self, e: Event) {
        self.buffer.push_back(e);
    }
    pub fn drain_into(&mut self, q: &mut Workqueue, owner: &TenantId) {
        while let Some(e) = self.buffer.pop_front() {
            if e.key().tenant == *owner {
                q.add(e.key().clone());
            }
        }
    }
    pub fn pending(&self) -> usize {
        self.buffer.len()
    }
}

/// Rate-limited deduplicating queue. Mirrors
/// `client-go/util/workqueue.RateLimitingInterface`.
///
/// Semantics:
/// * `add(k)` is idempotent — re-adding while `k` is queued or processing
///   is a no-op.
/// * `get()` returns the next key and marks it as processing.
/// * `done(k)` is required before `add(k)` will re-enqueue.
/// * If a key is `add`ed again *while* it's processing, it gets requeued
///   exactly once when `done(k)` is called (the "dirty bit" pattern).
#[derive(Debug, Default)]
pub struct Workqueue {
    queue: VecDeque<ObjectKey>,
    queued: HashSet<ObjectKey>,
    processing: HashSet<ObjectKey>,
    dirty: HashSet<ObjectKey>,
    requeues: u64,
}

impl Workqueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, k: ObjectKey) {
        if self.processing.contains(&k) {
            self.dirty.insert(k);
            return;
        }
        if self.queued.insert(k.clone()) {
            self.queue.push_back(k);
        }
    }

    pub fn get(&mut self) -> Option<ObjectKey> {
        let k = self.queue.pop_front()?;
        self.queued.remove(&k);
        self.processing.insert(k.clone());
        Some(k)
    }

    pub fn done(&mut self, k: &ObjectKey) {
        if !self.processing.remove(k) {
            return;
        }
        if self.dirty.remove(k) {
            // Re-enqueue once.
            self.queued.insert(k.clone());
            self.queue.push_back(k.clone());
            self.requeues += 1;
        }
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
    pub fn processing_count(&self) -> usize {
        self.processing.len()
    }
    pub fn requeue_count(&self) -> u64 {
        self.requeues
    }
}

/// Sync callback invoked once per drained key.
pub trait Reconciler {
    fn reconcile(&mut self, key: &ObjectKey) -> Result<Reconcile, ControllerError>;
}

/// Stateless test reconciler that always returns the configured outcome.
#[derive(Debug)]
pub struct ConstReconciler {
    pub outcome: Reconcile,
    pub calls: u64,
}

impl ConstReconciler {
    pub fn new(outcome: Reconcile) -> Self {
        Self { outcome, calls: 0 }
    }
}

impl Reconciler for ConstReconciler {
    fn reconcile(&mut self, _key: &ObjectKey) -> Result<Reconcile, ControllerError> {
        self.calls += 1;
        Ok(self.outcome.clone())
    }
}

/// Tenant-bound controller. Drains a queue against a [`Reconciler`].
#[derive(Debug)]
pub struct SyncController<R: Reconciler> {
    pub owner: TenantId,
    pub reconciler: R,
    pub processed: u64,
    pub denied_cross_tenant: u64,
    pub errored: u64,
}

impl<R: Reconciler> SyncController<R> {
    pub fn new(owner: TenantId, reconciler: R) -> Self {
        Self { owner, reconciler, processed: 0, denied_cross_tenant: 0, errored: 0 }
    }

    /// Drain everything currently in the queue. Returns the number of keys
    /// that were actually dispatched (not the number processed — keys
    /// belonging to a foreign tenant are silently dropped after marking
    /// `done` to keep the workqueue ledger consistent).
    pub fn run_until_idle(&mut self, q: &mut Workqueue) -> usize {
        let mut dispatched = 0;
        while let Some(k) = q.get() {
            if k.tenant != self.owner {
                self.denied_cross_tenant += 1;
                q.done(&k);
                continue;
            }
            match self.reconciler.reconcile(&k) {
                Ok(_) => {
                    self.processed += 1;
                    dispatched += 1;
                }
                Err(_) => {
                    self.errored += 1;
                }
            }
            q.done(&k);
        }
        dispatched
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "cmd/kube-controller-manager/app/controllermanager.go",
    "Run",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn key(tenant: &str, name: &str) -> ObjectKey {
        ObjectKey::new(TenantId::new(tenant), "Deployment", "default", name)
    }

    #[test]
    fn workqueue_dedups_repeated_adds_before_get() {
        let (_cite, _t) = test_ctx!(
            "client-go/util/workqueue/queue.go",
            "Type.Add",
            "tenant-mgr-dedup"
        );
        let mut q = Workqueue::new();
        q.add(key("acme", "web"));
        q.add(key("acme", "web"));
        q.add(key("acme", "web"));
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn workqueue_get_marks_processing_and_blocks_re_enqueue_until_done() {
        let (_cite, _t) = test_ctx!(
            "client-go/util/workqueue/queue.go",
            "Type.Done",
            "tenant-mgr-dirty"
        );
        let mut q = Workqueue::new();
        let k = key("acme", "web");
        q.add(k.clone());
        let got = q.get().unwrap();
        assert_eq!(got, k);
        // While processing, an add becomes a dirty bit, NOT a re-enqueue.
        q.add(k.clone());
        assert_eq!(q.len(), 0);
        assert_eq!(q.processing_count(), 1);
        // done() flushes the dirty bit and re-enqueues exactly once.
        q.done(&k);
        assert_eq!(q.len(), 1);
        assert_eq!(q.processing_count(), 0);
        assert_eq!(q.requeue_count(), 1);
    }

    #[test]
    fn workqueue_done_without_processing_is_noop() {
        let (_cite, _t) = test_ctx!(
            "client-go/util/workqueue/queue.go",
            "Type.Done",
            "tenant-mgr-done-noop"
        );
        let mut q = Workqueue::new();
        q.done(&key("acme", "ghost"));
        assert_eq!(q.requeue_count(), 0);
    }

    #[test]
    fn event_source_drain_filters_to_owner_tenant() {
        let (_cite, _t) = test_ctx!(
            "client-go/tools/cache/reflector.go",
            "Reflector.ListAndWatch",
            "tenant-mgr-source"
        );
        let mut src = EventSource::new();
        src.push(Event::Add(key("acme", "a")));
        src.push(Event::Add(key("evil", "b")));
        src.push(Event::Update(key("acme", "c")));
        let mut q = Workqueue::new();
        src.drain_into(&mut q, &TenantId::new("acme"));
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn sync_controller_processes_owner_keys_and_drops_cross_tenant() {
        let (_cite, _t) = test_ctx!(
            "cmd/kube-controller-manager/app/controllermanager.go",
            "ControllerLoop",
            "tenant-mgr-sync"
        );
        let mut q = Workqueue::new();
        q.add(key("acme", "a"));
        q.add(key("evil", "b"));
        q.add(key("acme", "c"));
        let mut ctrl = SyncController::new(TenantId::new("acme"), ConstReconciler::new(Reconcile::NoOp));
        let dispatched = ctrl.run_until_idle(&mut q);
        assert_eq!(dispatched, 2);
        assert_eq!(ctrl.processed, 2);
        assert_eq!(ctrl.denied_cross_tenant, 1);
        assert_eq!(q.len(), 0);
        assert_eq!(q.processing_count(), 0);
    }

    #[test]
    fn reconciler_error_is_counted_separately_from_processed() {
        let (_cite, _t) = test_ctx!(
            "cmd/kube-controller-manager/app/controllermanager.go",
            "ControllerLoop",
            "tenant-mgr-err"
        );
        struct ErrorReconciler;
        impl Reconciler for ErrorReconciler {
            fn reconcile(&mut self, _: &ObjectKey) -> Result<Reconcile, ControllerError> {
                Err(ControllerError::Unimplemented("test"))
            }
        }
        let mut q = Workqueue::new();
        q.add(key("acme", "a"));
        let mut ctrl = SyncController::new(TenantId::new("acme"), ErrorReconciler);
        ctrl.run_until_idle(&mut q);
        assert_eq!(ctrl.processed, 0);
        assert_eq!(ctrl.errored, 1);
    }

    #[test]
    fn end_to_end_pipeline_drains_event_source_to_reconciler() {
        let (_cite, _t) = test_ctx!(
            "cmd/kube-controller-manager/app/controllermanager.go",
            "Run",
            "tenant-mgr-e2e"
        );
        let owner = TenantId::new("acme");
        let mut src = EventSource::new();
        for n in ["a", "b", "c", "a"] {
            src.push(Event::Add(key("acme", n)));
        }
        src.push(Event::Update(key("evil", "x")));
        let mut q = Workqueue::new();
        src.drain_into(&mut q, &owner);
        assert_eq!(q.len(), 3); // dedup ate the second "a", and "evil" was filtered
        let mut ctrl = SyncController::new(owner, ConstReconciler::new(Reconcile::Update(1)));
        ctrl.run_until_idle(&mut q);
        assert_eq!(ctrl.processed, 3);
        assert_eq!(ctrl.reconciler.calls, 3);
    }
}
