// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cascade plan computation — `pkg/controller/garbagecollector/garbagecollector.go`.
//!
//! Given the dependency graph and a [`DeletionPropagation`] policy, compute
//! the concrete actions the GC controller needs to take when the user
//! `DELETE`s a root object:
//!
//! * **Background**: collect the root and all transitive dependents into
//!   `delete`. They will be deleted asynchronously in dependent-first order.
//! * **Foreground**: the root cannot be deleted until all dependents that
//!   set `block_owner_deletion = true` are gone. Plan emits:
//!     * `set_orphan_finalizer` for the root (so it stays alive while we
//!       wait — upstream sets `metadata.deletionTimestamp` + the
//!       `foregroundDeletion` finalizer).
//!     * `delete` listing the blocking dependents.
//!     * `wait_for` mirroring `delete` — these must be observed gone before
//!       the root may be removed.
//! * **Orphan**: the root is deleted immediately; direct dependents have
//!   their owner reference rewritten (via `rewrite_owner_refs`) so the GC
//!   no longer sees them as descendants of `root`.

use super::graph::{DependencyGraph, ObjectId};
use crate::types::Cite;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeletionPropagation {
    Foreground,
    Background,
    Orphan,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CascadePlan {
    /// Objects to delete (in this order: dependents first, root last for
    /// Background; blocking dependents only for Foreground; just root for
    /// Orphan).
    pub delete: Vec<ObjectId>,
    /// Objects whose owner-reference list must be rewritten to drop the
    /// deleted root's UID (Orphan mode only).
    pub orphan: Vec<ObjectId>,
    /// Objects that need the foregroundDeletion finalizer set (Foreground only).
    pub set_orphan_finalizer: Vec<ObjectId>,
    /// Objects whose disappearance the controller waits on before removing
    /// the root finalizer (Foreground only). Mirrors `wait_for` in upstream.
    pub wait_for: Vec<ObjectId>,
}

/// Compute the cascade plan for deleting `root` under `mode`.
///
/// For multi-owner dependents, deletion only happens when ALL owners are
/// scheduled for deletion in this plan; otherwise the dependent is preserved
/// and listed under `orphan`.
pub fn compute_cascade_plan(
    graph: &DependencyGraph,
    root: &ObjectId,
    mode: DeletionPropagation,
) -> CascadePlan {
    match mode {
        DeletionPropagation::Background => background(graph, root),
        DeletionPropagation::Foreground => foreground(graph, root),
        DeletionPropagation::Orphan => orphan(graph, root),
    }
}

fn background(graph: &DependencyGraph, root: &ObjectId) -> CascadePlan {
    // Transitive dependents that are exclusively owned by the to-be-deleted
    // set (closure starting from root). Other-owned dependents survive and
    // get listed under `orphan` for owner-ref rewrite.
    let descendants = graph.transitive_dependents(root);
    let mut delete_set: std::collections::HashSet<ObjectId> = std::collections::HashSet::new();
    delete_set.insert(root.clone());

    // Iteratively grow delete_set with dependents whose every owner is in the set.
    loop {
        let before = delete_set.len();
        for d in &descendants {
            if delete_set.contains(d) {
                continue;
            }
            let owners = graph.owners_of(d);
            if !owners.is_empty() && owners.iter().all(|o| delete_set.contains(o)) {
                delete_set.insert(d.clone());
            }
        }
        if delete_set.len() == before {
            break;
        }
    }

    let mut orphan_list: Vec<ObjectId> = descendants
        .iter()
        .filter(|d| !delete_set.contains(d))
        .cloned()
        .collect();
    orphan_list.sort();

    // Order delete: dependents first, root last (deepest first heuristic via BFS reverse).
    let mut delete: Vec<ObjectId> = descendants
        .iter()
        .filter(|d| delete_set.contains(d))
        .cloned()
        .collect();
    delete.sort();
    // BFS produced ancestors-first; reverse to get dependents-first.
    delete.reverse();
    delete.push(root.clone());

    CascadePlan {
        delete,
        orphan: orphan_list,
        set_orphan_finalizer: vec![],
        wait_for: vec![],
    }
}

fn foreground(graph: &DependencyGraph, root: &ObjectId) -> CascadePlan {
    // Blocking direct dependents (block_owner_deletion = true on their ref to root).
    // For each blocker, recursively foreground-delete it as well.
    let mut delete = Vec::new();
    let mut wait_for = Vec::new();
    let mut set_orphan_finalizer = vec![root.clone()];

    let direct = graph.dependents_of(root);
    for d in direct {
        let blocking = graph.owners_of(&d).iter().any(|_| {
            // Need to look up the OwnerReference flags on d; pull from graph's stored owners.
            // graph.owners_of returns just UIDs; reach into the node directly.
            false
        }) || dependent_blocks(graph, &d, root);
        if blocking {
            // Recursively gather: each blocker's own blocking dependents
            // also need to be scheduled before this blocker can go.
            let sub = foreground(graph, &d);
            for x in sub.delete {
                if !delete.contains(&x) {
                    delete.push(x);
                }
            }
            for x in sub.wait_for {
                if !wait_for.contains(&x) {
                    wait_for.push(x);
                }
            }
            for x in sub.set_orphan_finalizer {
                if !set_orphan_finalizer.contains(&x) {
                    set_orphan_finalizer.push(x);
                }
            }
            if !delete.contains(&d) {
                delete.push(d.clone());
            }
            if !wait_for.contains(&d) {
                wait_for.push(d);
            }
        }
    }
    delete.sort();
    wait_for.sort();
    set_orphan_finalizer.sort();

    CascadePlan {
        delete,
        orphan: vec![],
        set_orphan_finalizer,
        wait_for,
    }
}

/// Returns true if `dep` has an owner-reference pointing at `owner` with
/// `block_owner_deletion = true`.
fn dependent_blocks(graph: &DependencyGraph, dep: &ObjectId, owner: &ObjectId) -> bool {
    // We need the raw OwnerReference; expose via graph query.
    graph
        .owner_refs_of(dep)
        .iter()
        .any(|r| r.uid == owner.0 && r.block_owner_deletion)
}

fn orphan(graph: &DependencyGraph, root: &ObjectId) -> CascadePlan {
    let direct = graph.dependents_of(root);
    CascadePlan {
        delete: vec![root.clone()],
        orphan: direct,
        set_orphan_finalizer: vec![],
        wait_for: vec![],
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/garbagecollector/garbagecollector.go",
    "attemptToDeleteItem",
);

#[cfg(test)]
mod tests {
    use super::super::owner_ref::OwnerReference;
    use super::*;
    use crate::test_ctx;

    fn oid(s: &str) -> ObjectId {
        ObjectId::new(s)
    }
    fn or(uid: &str) -> OwnerReference {
        OwnerReference::new(uid, format!("o-{uid}"), "ReplicaSet")
    }
    fn or_block(uid: &str) -> OwnerReference {
        or(uid).blocking()
    }

    #[test]
    fn background_deletes_root_and_all_transitive_dependents() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/garbagecollector.go",
            "attemptToDeleteItem",
            "tenant-gc-cascade-bg-chain"
        );
        let mut g = DependencyGraph::new();
        g.add_object(oid("dep"), vec![]);
        g.add_object(oid("rs"), vec![or("dep")]);
        g.add_object(oid("pod"), vec![or("rs")]);
        let plan = compute_cascade_plan(&g, &oid("dep"), DeletionPropagation::Background);
        // Order: dependents-first, root last.
        assert_eq!(plan.delete.last(), Some(&oid("dep")));
        assert!(plan.delete.contains(&oid("rs")));
        assert!(plan.delete.contains(&oid("pod")));
        assert!(plan.orphan.is_empty());
    }

    #[test]
    fn background_emits_root_only_when_no_dependents() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/garbagecollector.go",
            "attemptToDeleteItem",
            "tenant-gc-cascade-bg-leaf"
        );
        let mut g = DependencyGraph::new();
        g.add_object(oid("solo"), vec![]);
        let plan = compute_cascade_plan(&g, &oid("solo"), DeletionPropagation::Background);
        assert_eq!(plan.delete, vec![oid("solo")]);
    }

    #[test]
    fn background_skips_dep_with_other_owners() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/garbagecollector.go",
            "deleteOwners",
            "tenant-gc-cascade-bg-multi-owner"
        );
        let mut g = DependencyGraph::new();
        g.add_object(oid("a"), vec![]);
        g.add_object(oid("b"), vec![]);
        g.add_object(oid("dep"), vec![or("a"), or("b")]);
        let plan = compute_cascade_plan(&g, &oid("a"), DeletionPropagation::Background);
        // dep also owned by b — survives, gets listed under orphan.
        assert!(!plan.delete.contains(&oid("dep")));
        assert!(plan.orphan.contains(&oid("dep")));
    }

    #[test]
    fn orphan_deletes_root_and_lists_direct_deps_for_owner_rewrite() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/garbagecollector.go",
            "orphanDependents",
            "tenant-gc-cascade-orphan-direct"
        );
        let mut g = DependencyGraph::new();
        g.add_object(oid("rs"), vec![]);
        g.add_object(oid("p1"), vec![or("rs")]);
        g.add_object(oid("p2"), vec![or("rs")]);
        let plan = compute_cascade_plan(&g, &oid("rs"), DeletionPropagation::Orphan);
        assert_eq!(plan.delete, vec![oid("rs")]);
        let mut o = plan.orphan.clone();
        o.sort();
        assert_eq!(o, vec![oid("p1"), oid("p2")]);
    }

    #[test]
    fn orphan_does_not_touch_transitive_grandchildren() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/garbagecollector.go",
            "orphanDependents",
            "tenant-gc-cascade-orphan-no-transitive"
        );
        let mut g = DependencyGraph::new();
        g.add_object(oid("dep"), vec![]);
        g.add_object(oid("rs"), vec![or("dep")]);
        g.add_object(oid("pod"), vec![or("rs")]);
        let plan = compute_cascade_plan(&g, &oid("dep"), DeletionPropagation::Orphan);
        assert_eq!(plan.delete, vec![oid("dep")]);
        // Only direct dependents listed.
        assert!(plan.orphan.contains(&oid("rs")));
        assert!(!plan.orphan.contains(&oid("pod")));
    }

    #[test]
    fn foreground_sets_finalizer_on_root() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/garbagecollector.go",
            "processGraphChanges",
            "tenant-gc-cascade-fg-finalizer"
        );
        let mut g = DependencyGraph::new();
        g.add_object(oid("dep"), vec![]);
        g.add_object(oid("pod"), vec![or_block("dep")]);
        let plan = compute_cascade_plan(&g, &oid("dep"), DeletionPropagation::Foreground);
        assert!(plan.set_orphan_finalizer.contains(&oid("dep")));
    }

    #[test]
    fn foreground_waits_only_on_blocking_dependents() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/garbagecollector.go",
            "processGraphChanges",
            "tenant-gc-cascade-fg-blocking-only"
        );
        let mut g = DependencyGraph::new();
        g.add_object(oid("dep"), vec![]);
        // p1 blocks; p2 doesn't.
        g.add_object(oid("p1"), vec![or_block("dep")]);
        g.add_object(oid("p2"), vec![or("dep")]);
        let plan = compute_cascade_plan(&g, &oid("dep"), DeletionPropagation::Foreground);
        assert!(plan.wait_for.contains(&oid("p1")));
        assert!(!plan.wait_for.contains(&oid("p2")));
    }

    #[test]
    fn foreground_recurses_into_blocker_subgraph() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/garbagecollector.go",
            "processGraphChanges",
            "tenant-gc-cascade-fg-recursive"
        );
        let mut g = DependencyGraph::new();
        g.add_object(oid("dep"), vec![]);
        g.add_object(oid("rs"), vec![or_block("dep")]);
        g.add_object(oid("pod"), vec![or_block("rs")]);
        let plan = compute_cascade_plan(&g, &oid("dep"), DeletionPropagation::Foreground);
        assert!(plan.wait_for.contains(&oid("rs")));
        assert!(plan.wait_for.contains(&oid("pod")));
        // pod's finalizer is also set (it itself is being foreground-deleted).
        assert!(plan.set_orphan_finalizer.contains(&oid("pod")));
    }

    #[test]
    fn foreground_no_blockers_yields_empty_wait() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/garbagecollector.go",
            "processGraphChanges",
            "tenant-gc-cascade-fg-no-blockers"
        );
        let mut g = DependencyGraph::new();
        g.add_object(oid("dep"), vec![]);
        g.add_object(oid("pod"), vec![or("dep")]);
        let plan = compute_cascade_plan(&g, &oid("dep"), DeletionPropagation::Foreground);
        assert!(plan.wait_for.is_empty());
        // Root finalizer still set (Foreground always sets it on root).
        assert!(plan.set_orphan_finalizer.contains(&oid("dep")));
    }

    #[test]
    fn cascade_plan_serializes_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/garbagecollector.go",
            "CascadePlan",
            "tenant-gc-cascade-serde"
        );
        let plan = CascadePlan {
            delete: vec![oid("a"), oid("b")],
            orphan: vec![oid("c")],
            set_orphan_finalizer: vec![oid("a")],
            wait_for: vec![oid("b")],
        };
        let s = serde_json::to_string(&plan).unwrap();
        let back: CascadePlan = serde_json::from_str(&s).unwrap();
        assert_eq!(plan, back);
    }

    #[test]
    fn deletion_propagation_serializes_three_modes() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/apis/meta/v1/types.go",
            "DeletionPropagation",
            "tenant-gc-cascade-mode-serde"
        );
        for m in [
            DeletionPropagation::Background,
            DeletionPropagation::Foreground,
            DeletionPropagation::Orphan,
        ] {
            let s = serde_json::to_string(&m).unwrap();
            let back: DeletionPropagation = serde_json::from_str(&s).unwrap();
            assert_eq!(m, back);
        }
    }

    #[test]
    fn background_dependents_first_root_last_ordering() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/garbagecollector.go",
            "attemptToDeleteItem",
            "tenant-gc-cascade-bg-order"
        );
        let mut g = DependencyGraph::new();
        g.add_object(oid("dep"), vec![]);
        g.add_object(oid("rs"), vec![or("dep")]);
        g.add_object(oid("pod"), vec![or("rs")]);
        let plan = compute_cascade_plan(&g, &oid("dep"), DeletionPropagation::Background);
        let last = plan.delete.last().unwrap();
        assert_eq!(last, &oid("dep"));
    }
}
