// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! GC dependency graph — `pkg/controller/garbagecollector/dependent_graph.go`.
//!
//! Each cluster object is a node keyed by UID. Edges go from owner to
//! dependent. The graph supports:
//!
//! * insert/remove of objects with arbitrary owner references.
//! * traversal of direct dependents and transitive dependents (BFS).
//! * lookup of all owners of a node.
//! * detection of cycles (rare, possible when admission lets a buggy CRD
//!   create them; upstream tolerates by visiting at most once).

use super::owner_ref::OwnerReference;
use crate::types::Cite;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

/// A cluster object's UID. Mirrors `apimachinery/pkg/types.UID`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ObjectId(pub String);

impl ObjectId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GraphNode {
    /// Owner references stored verbatim — the GC needs the `controller` and
    /// `block_owner_deletion` flags during cascade planning.
    pub owners: Vec<OwnerReference>,
    /// Direct dependents (objects whose `ownerReferences[]` contains this
    /// node's UID). Maintained as a derived index.
    pub dependents: HashSet<ObjectId>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct DependencyGraph {
    nodes: HashMap<ObjectId, GraphNode>,
}

impl DependencyGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn contains(&self, id: &ObjectId) -> bool {
        self.nodes.contains_key(id)
    }

    /// All node IDs currently in the graph (sorted for determinism).
    pub fn all_node_ids(&self) -> Vec<ObjectId> {
        let mut v: Vec<_> = self.nodes.keys().cloned().collect();
        v.sort();
        v
    }

    /// Insert (or overwrite) a node with its owner references. Updates the
    /// derived `dependents` index on the owner nodes — owner nodes that don't
    /// yet exist in the graph are auto-created (they may show up later via
    /// their own `add_object` call).
    pub fn add_object(&mut self, id: ObjectId, owners: Vec<OwnerReference>) {
        // Compute owner UID set for index maintenance.
        let owner_uids: Vec<ObjectId> = owners
            .iter()
            .map(|o| ObjectId::new(o.uid.clone()))
            .collect();

        // First, scrub stale dependent entries if this node existed before.
        if let Some(prev) = self.nodes.get(&id).cloned() {
            for prev_owner in &prev.owners {
                let oid = ObjectId::new(prev_owner.uid.clone());
                if !owner_uids.contains(&oid) {
                    if let Some(parent) = self.nodes.get_mut(&oid) {
                        parent.dependents.remove(&id);
                    }
                }
            }
        }

        // Insert/update this node, preserving its existing dependents.
        let existing_dependents = self
            .nodes
            .get(&id)
            .map(|n| n.dependents.clone())
            .unwrap_or_default();
        self.nodes.insert(
            id.clone(),
            GraphNode {
                owners: owners.clone(),
                dependents: existing_dependents,
            },
        );

        // Auto-create owner placeholder nodes and wire forward edges.
        for owner_id in owner_uids {
            let parent = self.nodes.entry(owner_id.clone()).or_default();
            parent.dependents.insert(id.clone());
        }
    }

    /// Remove an object. Owners' dependent-sets are updated; dependents'
    /// owner lists are *not* mutated (caller decides whether to orphan or
    /// transitively delete them — see `cascade.rs`).
    pub fn remove_object(&mut self, id: &ObjectId) {
        let Some(node) = self.nodes.remove(id) else {
            return;
        };
        for owner in &node.owners {
            let oid = ObjectId::new(owner.uid.clone());
            if let Some(parent) = self.nodes.get_mut(&oid) {
                parent.dependents.remove(id);
            }
        }
    }

    /// Direct dependents of `id`.
    pub fn dependents_of(&self, id: &ObjectId) -> Vec<ObjectId> {
        self.nodes
            .get(id)
            .map(|n| {
                let mut v: Vec<_> = n.dependents.iter().cloned().collect();
                v.sort();
                v
            })
            .unwrap_or_default()
    }

    /// Raw owner references on `id`. Returns empty if the node doesn't exist.
    pub fn owner_refs_of(&self, id: &ObjectId) -> &[OwnerReference] {
        self.nodes
            .get(id)
            .map(|n| n.owners.as_slice())
            .unwrap_or(&[])
    }

    /// Owners of `id` (UIDs).
    pub fn owners_of(&self, id: &ObjectId) -> Vec<ObjectId> {
        self.nodes
            .get(id)
            .map(|n| {
                let mut v: Vec<_> = n
                    .owners
                    .iter()
                    .map(|o| ObjectId::new(o.uid.clone()))
                    .collect();
                v.sort();
                v
            })
            .unwrap_or_default()
    }

    /// Transitive dependents of `id`, BFS, excluding `id` itself.
    /// Cycle-safe: each node visited at most once.
    pub fn transitive_dependents(&self, id: &ObjectId) -> Vec<ObjectId> {
        let mut out = Vec::new();
        let mut seen: HashSet<ObjectId> = HashSet::new();
        seen.insert(id.clone());
        let mut q: VecDeque<ObjectId> = VecDeque::new();
        q.push_back(id.clone());
        while let Some(curr) = q.pop_front() {
            if let Some(node) = self.nodes.get(&curr) {
                let mut deps: Vec<_> = node.dependents.iter().cloned().collect();
                deps.sort();
                for d in deps {
                    if seen.insert(d.clone()) {
                        out.push(d.clone());
                        q.push_back(d);
                    }
                }
            }
        }
        out
    }

    /// Returns true if there is a directed cycle reachable from `id`.
    /// Upstream `attemptToDeleteItem` tolerates cycles; this is a diagnostic.
    pub fn has_cycle_from(&self, id: &ObjectId) -> bool {
        let mut stack: Vec<(ObjectId, Vec<ObjectId>)> = Vec::new();
        stack.push((id.clone(), vec![id.clone()]));
        while let Some((curr, mut path)) = stack.pop() {
            if let Some(node) = self.nodes.get(&curr) {
                for d in &node.dependents {
                    if path.contains(d) {
                        return true;
                    }
                    let mut next_path = path.clone();
                    next_path.push(d.clone());
                    stack.push((d.clone(), next_path));
                }
            }
            path.clear();
        }
        false
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/garbagecollector/dependent_graph.go",
    "concurrentUIDToNode",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn oid(s: &str) -> ObjectId {
        ObjectId::new(s)
    }
    fn or(uid: &str) -> OwnerReference {
        OwnerReference::new(uid, format!("o-{uid}"), "ReplicaSet")
    }

    #[test]
    fn empty_graph_has_no_nodes() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/dependent_graph.go",
            "graphBuilder",
            "tenant-gc-graph-empty"
        );
        let g = DependencyGraph::new();
        assert!(g.is_empty());
        assert_eq!(g.len(), 0);
    }

    #[test]
    fn add_object_inserts_node() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/dependent_graph.go",
            "graphBuilder.processItem",
            "tenant-gc-graph-add"
        );
        let mut g = DependencyGraph::new();
        g.add_object(oid("a"), vec![]);
        assert!(g.contains(&oid("a")));
    }

    #[test]
    fn owner_creates_forward_dependent_edge() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/dependent_graph.go",
            "addDependentToOwners",
            "tenant-gc-graph-edge"
        );
        let mut g = DependencyGraph::new();
        g.add_object(oid("rs"), vec![]);
        g.add_object(oid("pod"), vec![or("rs")]);
        assert_eq!(g.dependents_of(&oid("rs")), vec![oid("pod")]);
        assert_eq!(g.owners_of(&oid("pod")), vec![oid("rs")]);
    }

    #[test]
    fn add_object_auto_creates_missing_owner_node() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/dependent_graph.go",
            "addDependentToOwners",
            "tenant-gc-graph-auto-owner"
        );
        let mut g = DependencyGraph::new();
        // Add pod referencing an owner UID that isn't in the graph yet.
        g.add_object(oid("pod"), vec![or("rs")]);
        assert!(g.contains(&oid("rs")));
        assert_eq!(g.dependents_of(&oid("rs")), vec![oid("pod")]);
    }

    #[test]
    fn remove_object_clears_owner_dependent_edges() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/dependent_graph.go",
            "removeDependentFromOwners",
            "tenant-gc-graph-remove"
        );
        let mut g = DependencyGraph::new();
        g.add_object(oid("rs"), vec![]);
        g.add_object(oid("pod"), vec![or("rs")]);
        g.remove_object(&oid("pod"));
        assert!(g.dependents_of(&oid("rs")).is_empty());
        assert!(!g.contains(&oid("pod")));
    }

    #[test]
    fn re_add_with_new_owners_scrubs_stale_edges() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/dependent_graph.go",
            "addDependentToOwners",
            "tenant-gc-graph-re-add"
        );
        let mut g = DependencyGraph::new();
        g.add_object(oid("rs1"), vec![]);
        g.add_object(oid("rs2"), vec![]);
        g.add_object(oid("pod"), vec![or("rs1")]);
        // Re-parent the pod to rs2.
        g.add_object(oid("pod"), vec![or("rs2")]);
        assert!(g.dependents_of(&oid("rs1")).is_empty());
        assert_eq!(g.dependents_of(&oid("rs2")), vec![oid("pod")]);
    }

    #[test]
    fn dependents_of_unknown_node_is_empty() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/dependent_graph.go",
            "uidToNode",
            "tenant-gc-graph-unknown"
        );
        let g = DependencyGraph::new();
        assert!(g.dependents_of(&oid("ghost")).is_empty());
    }

    #[test]
    fn transitive_dependents_chain() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/dependent_graph.go",
            "deletableDescendants",
            "tenant-gc-graph-transitive-chain"
        );
        let mut g = DependencyGraph::new();
        g.add_object(oid("dep"), vec![]);
        g.add_object(oid("rs"), vec![or("dep")]);
        g.add_object(oid("pod"), vec![or("rs")]);
        let mut got = g.transitive_dependents(&oid("dep"));
        got.sort();
        assert_eq!(got, vec![oid("pod"), oid("rs")]);
    }

    #[test]
    fn transitive_dependents_diamond_visits_each_node_once() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/dependent_graph.go",
            "deletableDescendants",
            "tenant-gc-graph-diamond"
        );
        let mut g = DependencyGraph::new();
        g.add_object(oid("a"), vec![]);
        g.add_object(oid("b"), vec![or("a")]);
        g.add_object(oid("c"), vec![or("a")]);
        g.add_object(oid("d"), vec![or("b"), or("c")]);
        let mut got = g.transitive_dependents(&oid("a"));
        got.sort();
        assert_eq!(got, vec![oid("b"), oid("c"), oid("d")]);
    }

    #[test]
    fn transitive_dependents_excludes_root() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/dependent_graph.go",
            "deletableDescendants",
            "tenant-gc-graph-no-self"
        );
        let mut g = DependencyGraph::new();
        g.add_object(oid("a"), vec![]);
        g.add_object(oid("b"), vec![or("a")]);
        let got = g.transitive_dependents(&oid("a"));
        assert!(!got.contains(&oid("a")));
    }

    #[test]
    fn cycle_detection_finds_self_loop() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/dependent_graph.go",
            "graphBuilder",
            "tenant-gc-graph-self-loop"
        );
        let mut g = DependencyGraph::new();
        g.add_object(oid("a"), vec![or("a")]);
        assert!(g.has_cycle_from(&oid("a")));
    }

    #[test]
    fn cycle_detection_finds_two_node_cycle() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/dependent_graph.go",
            "graphBuilder",
            "tenant-gc-graph-2cycle"
        );
        let mut g = DependencyGraph::new();
        g.add_object(oid("a"), vec![or("b")]);
        g.add_object(oid("b"), vec![or("a")]);
        assert!(g.has_cycle_from(&oid("a")));
    }

    #[test]
    fn no_cycle_in_acyclic_graph() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/dependent_graph.go",
            "graphBuilder",
            "tenant-gc-graph-acyclic"
        );
        let mut g = DependencyGraph::new();
        g.add_object(oid("a"), vec![]);
        g.add_object(oid("b"), vec![or("a")]);
        g.add_object(oid("c"), vec![or("b")]);
        assert!(!g.has_cycle_from(&oid("a")));
    }

    #[test]
    fn owners_of_returns_multi_owner_set() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/dependent_graph.go",
            "ownerReferences",
            "tenant-gc-graph-multi-owner"
        );
        let mut g = DependencyGraph::new();
        g.add_object(oid("o1"), vec![]);
        g.add_object(oid("o2"), vec![]);
        g.add_object(oid("dep"), vec![or("o1"), or("o2")]);
        let mut got = g.owners_of(&oid("dep"));
        got.sort();
        assert_eq!(got, vec![oid("o1"), oid("o2")]);
    }

    #[test]
    fn remove_root_does_not_delete_dependents_from_graph() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/dependent_graph.go",
            "removeNode",
            "tenant-gc-graph-remove-root-keeps-deps"
        );
        let mut g = DependencyGraph::new();
        g.add_object(oid("rs"), vec![]);
        g.add_object(oid("pod"), vec![or("rs")]);
        g.remove_object(&oid("rs"));
        // Dependent stays in the graph (owner refs are stale until rewritten by GC).
        assert!(g.contains(&oid("pod")));
    }
}
