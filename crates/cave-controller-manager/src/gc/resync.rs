// SPDX-License-Identifier: AGPL-3.0-or-later
//! GC graph resync — `pkg/controller/garbagecollector/garbagecollector.go::Sync`.
//!
//! Periodically rebuilds the dependency graph by listing every monitored
//! resource and reconciling it with the prior graph state.
//!
//! The resync also distinguishes "real" nodes (objects that appeared in a
//! live snapshot) from "virtual" nodes (UIDs only seen as owner references
//! by some dependent). Virtual nodes can later be upgraded to real when
//! their object materializes.

use super::graph::{DependencyGraph, ObjectId};
use super::owner_ref::OwnerReference;
use crate::types::Cite;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveObject {
    pub uid: ObjectId,
    pub owners: Vec<OwnerReference>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ResyncDiff {
    /// UIDs that were not previously real but now are (newly observed).
    pub added: Vec<ObjectId>,
    /// UIDs that were previously real but no longer are.
    pub removed: Vec<ObjectId>,
    /// UIDs present in the graph as side-effect of being someone's owner
    /// but never observed as a real object.
    pub virtual_nodes: Vec<ObjectId>,
}

/// Stateful resyncer — tracks which UIDs have been observed as real
/// objects across resyncs.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Resyncer {
    real_uids: HashSet<ObjectId>,
}

impl Resyncer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn resync(&mut self, graph: &mut DependencyGraph, live: &[LiveObject]) -> ResyncDiff {
        let new_real: HashSet<ObjectId> = live.iter().map(|o| o.uid.clone()).collect();

        let mut added: Vec<ObjectId> =
            new_real.difference(&self.real_uids).cloned().collect();
        added.sort();
        let mut removed: Vec<ObjectId> =
            self.real_uids.difference(&new_real).cloned().collect();
        removed.sort();

        // Apply removals first.
        for id in &removed {
            graph.remove_object(id);
        }
        // Insert / refresh each live object.
        for o in live {
            graph.add_object(o.uid.clone(), o.owners.clone());
        }
        // Anything in the graph that isn't real → virtual.
        let mut virtual_nodes: Vec<ObjectId> = graph
            .all_node_ids()
            .into_iter()
            .filter(|id| !new_real.contains(id))
            .collect();
        virtual_nodes.sort();

        self.real_uids = new_real;
        ResyncDiff { added, removed, virtual_nodes }
    }
}

/// Stateless one-shot resync helper for callers that don't keep a Resyncer.
pub fn resync(graph: &mut DependencyGraph, live: &[LiveObject]) -> ResyncDiff {
    let mut r = Resyncer::new();
    // Seed with current graph nodes so "removed" semantics still work for
    // a single call.
    r.real_uids = graph.all_node_ids().into_iter().collect();
    r.resync(graph, live)
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/garbagecollector/garbagecollector.go",
    "Sync",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn or(uid: &str) -> OwnerReference {
        OwnerReference::new(uid, format!("o-{uid}"), "Pod")
    }
    fn live(uid: &str, owners: &[&str]) -> LiveObject {
        LiveObject {
            uid: ObjectId::new(uid),
            owners: owners.iter().map(|u| or(u)).collect(),
        }
    }

    #[test]
    fn empty_graph_resync_inserts_every_live_object() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/garbagecollector.go",
            "Sync",
            "tenant-gc-resync-empty"
        );
        let mut g = DependencyGraph::new();
        let mut r = Resyncer::new();
        let diff = r.resync(&mut g, &[live("a", &[]), live("b", &["a"])]);
        assert_eq!(diff.added.len(), 2);
        assert!(diff.removed.is_empty());
    }

    #[test]
    fn resync_removes_objects_not_in_live() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/garbagecollector.go",
            "Sync",
            "tenant-gc-resync-remove"
        );
        let mut g = DependencyGraph::new();
        let mut r = Resyncer::new();
        r.resync(&mut g, &[live("a", &[]), live("b", &[])]);
        let diff = r.resync(&mut g, &[live("a", &[])]);
        assert!(diff.removed.contains(&ObjectId::new("b")));
    }

    #[test]
    fn resync_idempotent_when_unchanged() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/garbagecollector.go",
            "Sync",
            "tenant-gc-resync-idemp"
        );
        let mut g = DependencyGraph::new();
        let mut r = Resyncer::new();
        r.resync(&mut g, &[live("a", &[])]);
        let diff = r.resync(&mut g, &[live("a", &[])]);
        assert!(diff.added.is_empty());
        assert!(diff.removed.is_empty());
    }

    #[test]
    fn resync_flags_virtual_nodes_referenced_by_owners() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/garbagecollector.go",
            "generateVirtualNode",
            "tenant-gc-resync-virtual"
        );
        let mut g = DependencyGraph::new();
        let mut r = Resyncer::new();
        let diff = r.resync(&mut g, &[live("dep", &["phantom"])]);
        assert!(diff.virtual_nodes.contains(&ObjectId::new("phantom")));
    }

    #[test]
    fn resync_picks_up_object_after_appearing() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/garbagecollector.go",
            "Sync",
            "tenant-gc-resync-late-arrival"
        );
        let mut g = DependencyGraph::new();
        let mut r = Resyncer::new();
        r.resync(&mut g, &[live("dep", &["owner"])]);
        let diff = r.resync(&mut g, &[live("dep", &["owner"]), live("owner", &[])]);
        assert!(diff.added.contains(&ObjectId::new("owner")));
        // No longer in the virtual set after upgrade.
        assert!(!diff.virtual_nodes.contains(&ObjectId::new("owner")));
    }

    #[test]
    fn resync_preserves_dependent_edges_when_owner_added() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/garbagecollector.go",
            "Sync",
            "tenant-gc-resync-edges"
        );
        let mut g = DependencyGraph::new();
        let mut r = Resyncer::new();
        r.resync(&mut g, &[live("dep", &["owner"]), live("owner", &[])]);
        r.resync(&mut g, &[live("dep", &["owner"]), live("owner", &[])]);
        assert!(g
            .dependents_of(&ObjectId::new("owner"))
            .contains(&ObjectId::new("dep")));
    }

    #[test]
    fn resync_diff_serializes_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/garbagecollector.go",
            "ResyncDiff",
            "tenant-gc-resync-serde"
        );
        let d = ResyncDiff {
            added: vec![ObjectId::new("a")],
            removed: vec![ObjectId::new("b")],
            virtual_nodes: vec![],
        };
        let s = serde_json::to_string(&d).unwrap();
        let back: ResyncDiff = serde_json::from_str(&s).unwrap();
        assert_eq!(d, back);
    }

    #[test]
    fn resync_handles_owner_ref_change_in_place() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/garbagecollector.go",
            "Sync",
            "tenant-gc-resync-reparent"
        );
        let mut g = DependencyGraph::new();
        let mut r = Resyncer::new();
        r.resync(&mut g, &[live("dep", &["o1"]), live("o1", &[]), live("o2", &[])]);
        r.resync(&mut g, &[live("dep", &["o2"]), live("o1", &[]), live("o2", &[])]);
        assert!(g.dependents_of(&ObjectId::new("o1")).is_empty());
        assert!(g
            .dependents_of(&ObjectId::new("o2"))
            .contains(&ObjectId::new("dep")));
    }

    #[test]
    fn stateless_resync_seeds_from_graph() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/garbagecollector.go",
            "Sync",
            "tenant-gc-resync-stateless"
        );
        let mut g = DependencyGraph::new();
        // First populate the graph manually.
        g.add_object(ObjectId::new("a"), vec![]);
        g.add_object(ObjectId::new("b"), vec![]);
        // Live drops "b" → stateless resync removes it (uses graph as seed).
        let diff = resync(&mut g, &[live("a", &[])]);
        assert!(diff.removed.contains(&ObjectId::new("b")));
    }
}
