//! Garbage collector — deletes resources whose owner_uid no longer exists.
//!
//! Upstream: pkg/controller/garbagecollector/garbagecollector.go::RunOnce

use crate::models::OwnedResource;
use std::collections::HashSet;
use uuid::Uuid;

/// Run one pass of garbage collection. Returns the uids of resources that
/// should be deleted (owners are no longer present in `live_uids`).
///
/// Resources without an owner are not garbage-collected (mirrors upstream
/// behavior: an empty owner_uid means root resource).
pub fn reconcile_garbage_collector(
    resources: &[OwnedResource],
    live_uids: &HashSet<Uuid>,
) -> Vec<Uuid> {
    resources
        .iter()
        .filter(|r| match r.owner_uid {
            Some(owner) => !live_uids.contains(&owner),
            None => false,
        })
        .map(|r| r.uid)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resource(name: &str, owner: Option<Uuid>) -> OwnedResource {
        OwnedResource {
            uid: Uuid::new_v4(),
            kind: "Pod".into(),
            name: name.into(),
            namespace: "default".into(),
            owner_uid: owner,
        }
    }

    /// Port of `TestGarbageCollectorOrphan` — a resource whose owner uid is
    /// not in the live set is queued for deletion; one whose owner is alive
    /// is left untouched; orphans (no owner) are not collected.
    #[test]
    fn test_garbage_collector_orphan() {
        let alive = Uuid::new_v4();
        let dead = Uuid::new_v4();

        let with_alive_owner = resource("a", Some(alive));
        let with_dead_owner = resource("b", Some(dead));
        let no_owner = resource("c", None);

        let live: HashSet<Uuid> = [alive].into_iter().collect();
        let to_delete = reconcile_garbage_collector(
            &[with_alive_owner.clone(), with_dead_owner.clone(), no_owner.clone()],
            &live,
        );

        assert_eq!(to_delete.len(), 1);
        assert_eq!(to_delete[0], with_dead_owner.uid);
    }

    #[test]
    fn test_garbage_collector_all_alive() {
        let owner = Uuid::new_v4();
        let r = resource("a", Some(owner));
        let live: HashSet<Uuid> = [owner].into_iter().collect();
        assert!(reconcile_garbage_collector(&[r], &live).is_empty());
    }
}
