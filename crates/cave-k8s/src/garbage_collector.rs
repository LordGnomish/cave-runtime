// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Generic GarbageCollector — tracks owner-reference edges and computes
//! the cascade-delete closure when an owning resource is removed.
//!
//! Mirrors `pkg/controller/garbagecollector` of upstream Kubernetes:
//! every resource may declare zero or more `OwnerReference`s pointing
//! back at a parent.  When a parent is deleted with the `Foreground`
//! propagation policy, every child whose ownership tree roots in that
//! parent enters a deletion wave in dependency order.
//!
//! cave-k8s' GC only computes the *plan*; the actual store delete is
//! issued by the apiserver layer.

use crate::models::ResourceRef;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Propagation {
    Foreground,
    Background,
    Orphan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerEdge {
    pub owner: ResourceRef,
    pub child: ResourceRef,
    /// When true, deletion of the owner blocks until the child is gone
    /// (`metadata.ownerReferences[].blockOwnerDeletion`).
    pub block: bool,
}

pub struct GarbageCollector {
    edges: RwLock<Vec<OwnerEdge>>,
}

impl Default for GarbageCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl GarbageCollector {
    pub fn new() -> Self {
        Self {
            edges: RwLock::new(Vec::new()),
        }
    }

    pub fn link(&self, edge: OwnerEdge) {
        self.edges.write().expect("gc lock").push(edge);
    }

    pub fn unlink(&self, owner: &ResourceRef, child: &ResourceRef) -> bool {
        let mut g = self.edges.write().expect("gc lock");
        let before = g.len();
        g.retain(|e| !(e.owner == *owner && e.child == *child));
        before != g.len()
    }

    pub fn count(&self) -> usize {
        self.edges.read().expect("gc lock").len()
    }

    pub fn children_of(&self, owner: &ResourceRef) -> Vec<ResourceRef> {
        let g = self.edges.read().expect("gc lock");
        g.iter()
            .filter(|e| e.owner == *owner)
            .map(|e| e.child.clone())
            .collect()
    }

    pub fn owners_of(&self, child: &ResourceRef) -> Vec<ResourceRef> {
        let g = self.edges.read().expect("gc lock");
        g.iter()
            .filter(|e| e.child == *child)
            .map(|e| e.owner.clone())
            .collect()
    }

    /// Returns the topological deletion order for a cascade rooted at
    /// `root`.  Children appear before owners.  Orphan policy returns
    /// only `[root]`. Background policy returns `[root, all descendants]`
    /// in BFS order (so apiserver can issue async deletes).
    pub fn cascade_plan(&self, root: &ResourceRef, policy: Propagation) -> Vec<ResourceRef> {
        match policy {
            Propagation::Orphan => vec![root.clone()],
            Propagation::Background => self.bfs_descendants_incl(root),
            Propagation::Foreground => self.topo_dependents(root),
        }
    }

    fn bfs_descendants_incl(&self, root: &ResourceRef) -> Vec<ResourceRef> {
        let mut order = vec![root.clone()];
        let mut seen = BTreeSet::new();
        seen.insert(root.clone());
        let mut q: VecDeque<ResourceRef> = VecDeque::new();
        q.push_back(root.clone());
        while let Some(cur) = q.pop_front() {
            for c in self.children_of(&cur) {
                if seen.insert(c.clone()) {
                    order.push(c.clone());
                    q.push_back(c);
                }
            }
        }
        order
    }

    /// Foreground cascade — children must be deleted before the parent;
    /// returns a deletion-order list with `root` last.
    fn topo_dependents(&self, root: &ResourceRef) -> Vec<ResourceRef> {
        let g = self.edges.read().expect("gc lock");
        // Collect every node reachable from `root`.
        let mut reachable: BTreeSet<ResourceRef> = BTreeSet::new();
        reachable.insert(root.clone());
        let mut stack = vec![root.clone()];
        while let Some(cur) = stack.pop() {
            for e in g.iter().filter(|e| e.owner == cur) {
                if reachable.insert(e.child.clone()) {
                    stack.push(e.child.clone());
                }
            }
        }
        // Kahn-style: order = children with no outgoing edge inside `reachable` first.
        let mut outgoing: BTreeMap<ResourceRef, usize> = BTreeMap::new();
        for r in &reachable {
            outgoing.insert(r.clone(), 0);
        }
        for e in g.iter() {
            if reachable.contains(&e.owner) && reachable.contains(&e.child) {
                *outgoing.entry(e.owner.clone()).or_insert(0) += 1;
            }
        }
        let mut ready: VecDeque<ResourceRef> = outgoing
            .iter()
            .filter(|(_, c)| **c == 0)
            .map(|(r, _)| r.clone())
            .collect();
        let mut order = Vec::with_capacity(reachable.len());
        while let Some(r) = ready.pop_front() {
            order.push(r.clone());
            // For every parent whose child = r, decrement.
            for e in g.iter().filter(|e| e.child == r && reachable.contains(&e.owner)) {
                let cnt = outgoing.entry(e.owner.clone()).or_insert(0);
                if *cnt > 0 {
                    *cnt -= 1;
                }
                if *cnt == 0 && !order.contains(&e.owner) && !ready.contains(&e.owner) {
                    ready.push_back(e.owner.clone());
                }
            }
        }
        // Cycle fallback: any unrelated remainder is appended.
        for r in &reachable {
            if !order.contains(r) {
                order.push(r.clone());
            }
        }
        order
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ns(kind: &str, n: &str) -> ResourceRef {
        ResourceRef::namespaced(kind, "default", n)
    }

    #[test]
    fn orphan_returns_only_root() {
        let g = GarbageCollector::new();
        g.link(OwnerEdge {
            owner: ns("Deployment", "d"),
            child: ns("ReplicaSet", "r"),
            block: false,
        });
        let plan = g.cascade_plan(&ns("Deployment", "d"), Propagation::Orphan);
        assert_eq!(plan, vec![ns("Deployment", "d")]);
    }

    #[test]
    fn background_bfs_includes_descendants() {
        let g = GarbageCollector::new();
        g.link(OwnerEdge {
            owner: ns("Deployment", "d"),
            child: ns("ReplicaSet", "r"),
            block: false,
        });
        g.link(OwnerEdge {
            owner: ns("ReplicaSet", "r"),
            child: ns("Pod", "p1"),
            block: false,
        });
        g.link(OwnerEdge {
            owner: ns("ReplicaSet", "r"),
            child: ns("Pod", "p2"),
            block: false,
        });
        let plan = g.cascade_plan(&ns("Deployment", "d"), Propagation::Background);
        assert_eq!(plan.len(), 4);
        assert_eq!(plan[0], ns("Deployment", "d"));
    }

    #[test]
    fn foreground_topological_children_first() {
        let g = GarbageCollector::new();
        g.link(OwnerEdge {
            owner: ns("Deployment", "d"),
            child: ns("ReplicaSet", "r"),
            block: true,
        });
        g.link(OwnerEdge {
            owner: ns("ReplicaSet", "r"),
            child: ns("Pod", "p1"),
            block: true,
        });
        let plan = g.cascade_plan(&ns("Deployment", "d"), Propagation::Foreground);
        // Pod first, RS, then Deployment last.
        let pi = plan.iter().position(|x| *x == ns("Pod", "p1")).unwrap();
        let ri = plan.iter().position(|x| *x == ns("ReplicaSet", "r")).unwrap();
        let di = plan.iter().position(|x| *x == ns("Deployment", "d")).unwrap();
        assert!(pi < ri && ri < di, "plan order: {:?}", plan);
    }

    #[test]
    fn unlink_drops_edge() {
        let g = GarbageCollector::new();
        let owner = ns("Deployment", "d");
        let child = ns("ReplicaSet", "r");
        g.link(OwnerEdge {
            owner: owner.clone(),
            child: child.clone(),
            block: false,
        });
        assert_eq!(g.count(), 1);
        assert!(g.unlink(&owner, &child));
        assert_eq!(g.count(), 0);
        assert!(!g.unlink(&owner, &child));
    }

    #[test]
    fn owners_of_lists_parents() {
        let g = GarbageCollector::new();
        g.link(OwnerEdge {
            owner: ns("Deployment", "a"),
            child: ns("Pod", "p"),
            block: false,
        });
        g.link(OwnerEdge {
            owner: ns("Deployment", "b"),
            child: ns("Pod", "p"),
            block: false,
        });
        let owners = g.owners_of(&ns("Pod", "p"));
        assert_eq!(owners.len(), 2);
    }

    #[test]
    fn children_of_lists_descendants() {
        let g = GarbageCollector::new();
        g.link(OwnerEdge {
            owner: ns("Deployment", "d"),
            child: ns("ReplicaSet", "r1"),
            block: false,
        });
        g.link(OwnerEdge {
            owner: ns("Deployment", "d"),
            child: ns("ReplicaSet", "r2"),
            block: false,
        });
        let c = g.children_of(&ns("Deployment", "d"));
        assert_eq!(c.len(), 2);
    }
}
