//! Orphan / foreground finalizer mechanics — `pkg/controller/garbagecollector/garbagecollector.go::orphanDependents`.
//!
//! Two finalizer-driven flows:
//!
//! * **`orphanDependents`** rewrites each dependent's `metadata.ownerReferences[]`
//!   to drop the deleted owner's UID. When the rewritten slice is empty, the
//!   `orphan` finalizer on the owner can be removed and the owner GC'd.
//! * **Stale owner reference detection** — references whose UID no longer
//!   appears in the cluster cache must also be scrubbed. Mirrors
//!   `pkg/controller/garbagecollector/garbagecollector.go::generateVirtualNode`.

use super::owner_ref::OwnerReference;
use crate::types::Cite;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Rewrite a dependent's owner-references list, removing every entry whose
/// `uid` matches one of `dropped_uids`. Returns the new owner-refs list.
pub fn rewrite_owner_refs(
    refs: &[OwnerReference],
    dropped_uids: &HashSet<String>,
) -> Vec<OwnerReference> {
    refs.iter()
        .filter(|r| !dropped_uids.contains(&r.uid))
        .cloned()
        .collect()
}

/// Filter out owner references whose UID is not present in the live cluster.
/// `live_uids` is the set of every UID currently observed in the GC graph.
/// Mirrors the `markStaleOwnerReferences` pass.
pub fn drop_stale_refs(
    refs: &[OwnerReference],
    live_uids: &HashSet<String>,
) -> Vec<OwnerReference> {
    refs.iter()
        .filter(|r| live_uids.contains(&r.uid))
        .cloned()
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrphanFinalizerAction {
    /// Owner-refs slice still non-empty after rewrite — patch the dependent.
    Patch,
    /// Owner-refs slice is now empty — patch + the dependent can become
    /// "ownerless" (its own deletion stays under user control).
    PatchAndOrphan,
    /// Nothing to drop — no change to apply.
    NoOp,
}

/// Compute what to do for one dependent after the orphan flow rewrites its
/// owner references.
pub fn orphan_action(
    refs: &[OwnerReference],
    dropped_uids: &HashSet<String>,
) -> OrphanFinalizerAction {
    let new_refs = rewrite_owner_refs(refs, dropped_uids);
    if new_refs.len() == refs.len() {
        OrphanFinalizerAction::NoOp
    } else if new_refs.is_empty() {
        OrphanFinalizerAction::PatchAndOrphan
    } else {
        OrphanFinalizerAction::Patch
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct WorkQueue {
    items: Vec<String>,
    seen: HashSet<String>,
}

impl WorkQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mirrors workqueue.RateLimitingInterface.Add — duplicates squashed.
    pub fn enqueue(&mut self, key: impl Into<String>) -> bool {
        let s = key.into();
        if self.seen.insert(s.clone()) {
            self.items.push(s);
            true
        } else {
            false
        }
    }

    pub fn pop(&mut self) -> Option<String> {
        if self.items.is_empty() {
            return None;
        }
        let s = self.items.remove(0);
        self.seen.remove(&s);
        Some(s)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/garbagecollector/garbagecollector.go",
    "orphanDependents",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn r(uid: &str) -> OwnerReference {
        OwnerReference::new(uid, format!("o-{uid}"), "Pod")
    }
    fn set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn rewrite_drops_specified_uids() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/garbagecollector.go",
            "orphanDependents",
            "tenant-gc-orph-rewrite"
        );
        let refs = vec![r("a"), r("b"), r("c")];
        let dropped = set(&["b"]);
        let got = rewrite_owner_refs(&refs, &dropped);
        assert_eq!(got.len(), 2);
        assert!(got.iter().all(|r| r.uid != "b"));
    }

    #[test]
    fn rewrite_no_op_when_no_match() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/garbagecollector.go",
            "orphanDependents",
            "tenant-gc-orph-rewrite-no-match"
        );
        let refs = vec![r("a"), r("b")];
        let dropped = set(&["x"]);
        let got = rewrite_owner_refs(&refs, &dropped);
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn drop_stale_keeps_only_live_uids() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/garbagecollector.go",
            "markStaleOwnerReferences",
            "tenant-gc-orph-stale"
        );
        let refs = vec![r("alive"), r("dead"), r("alive2")];
        let live = set(&["alive", "alive2"]);
        let got = drop_stale_refs(&refs, &live);
        assert_eq!(got.len(), 2);
        assert!(got.iter().all(|r| r.uid != "dead"));
    }

    #[test]
    fn orphan_action_patch_when_partial_drop() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/garbagecollector.go",
            "orphanDependents",
            "tenant-gc-orph-action-patch"
        );
        let refs = vec![r("a"), r("b")];
        assert_eq!(
            orphan_action(&refs, &set(&["a"])),
            OrphanFinalizerAction::Patch
        );
    }

    #[test]
    fn orphan_action_patch_and_orphan_when_all_dropped() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/garbagecollector.go",
            "orphanDependents",
            "tenant-gc-orph-action-empty"
        );
        let refs = vec![r("a"), r("b")];
        assert_eq!(
            orphan_action(&refs, &set(&["a", "b"])),
            OrphanFinalizerAction::PatchAndOrphan
        );
    }

    #[test]
    fn orphan_action_noop_when_nothing_to_drop() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/garbagecollector.go",
            "orphanDependents",
            "tenant-gc-orph-action-noop"
        );
        let refs = vec![r("a"), r("b")];
        assert_eq!(
            orphan_action(&refs, &set(&["x"])),
            OrphanFinalizerAction::NoOp
        );
    }

    #[test]
    fn workqueue_dedupes_repeat_enqueues() {
        let (_cite, _tenant) = test_ctx!(
            "staging/src/k8s.io/client-go/util/workqueue/rate_limiting_queue.go",
            "RateLimitingInterface",
            "tenant-gc-orph-workqueue-dedup"
        );
        let mut q = WorkQueue::new();
        assert!(q.enqueue("uid-1"));
        assert!(!q.enqueue("uid-1"));
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn workqueue_pops_in_fifo_order() {
        let (_cite, _tenant) = test_ctx!(
            "staging/src/k8s.io/client-go/util/workqueue/rate_limiting_queue.go",
            "Queue",
            "tenant-gc-orph-workqueue-fifo"
        );
        let mut q = WorkQueue::new();
        q.enqueue("a");
        q.enqueue("b");
        q.enqueue("c");
        assert_eq!(q.pop(), Some("a".into()));
        assert_eq!(q.pop(), Some("b".into()));
        assert_eq!(q.pop(), Some("c".into()));
        assert!(q.pop().is_none());
    }

    #[test]
    fn workqueue_re_admits_after_pop() {
        let (_cite, _tenant) = test_ctx!(
            "staging/src/k8s.io/client-go/util/workqueue/rate_limiting_queue.go",
            "Done",
            "tenant-gc-orph-workqueue-readd"
        );
        let mut q = WorkQueue::new();
        q.enqueue("a");
        q.pop();
        // After Done, the item can be re-added.
        assert!(q.enqueue("a"));
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn rewrite_then_drop_stale_composable() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/garbagecollector.go",
            "orphanDependents",
            "tenant-gc-orph-compose"
        );
        // Pipeline: drop deleted owner; then drop any stale-by-cache ones.
        let refs = vec![r("alive"), r("deleting"), r("ghost")];
        let after_orphan = rewrite_owner_refs(&refs, &set(&["deleting"]));
        let after_stale = drop_stale_refs(&after_orphan, &set(&["alive"]));
        assert_eq!(after_stale.len(), 1);
        assert_eq!(after_stale[0].uid, "alive");
    }

    #[test]
    fn orphan_finalizer_action_serde_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/garbagecollector.go",
            "OrphanFinalizerAction",
            "tenant-gc-orph-action-serde"
        );
        for a in [
            OrphanFinalizerAction::Patch,
            OrphanFinalizerAction::PatchAndOrphan,
            OrphanFinalizerAction::NoOp,
        ] {
            let s = serde_json::to_string(&a).unwrap();
            let back: OrphanFinalizerAction = serde_json::from_str(&s).unwrap();
            assert_eq!(a, back);
        }
    }
}
