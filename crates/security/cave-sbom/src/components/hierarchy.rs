// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/model/Project.java  (parent/children, active)
//   src/main/java/org/dependencytrack/persistence/ProjectQueryManager.java  (reparent cycle guard)
//
//! Project parent/children hierarchy — ancestor walk + reparent cycle guard.

use crate::components::Project;
use uuid::Uuid;

/// Why a reparent was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HierarchyError {
    /// The requested parent is the project itself or one of its descendants —
    /// accepting it would create a cycle in the portfolio tree.
    Cycle,
}

/// Read-only view over a set of projects, resolving parent/child relationships
/// by `uuid`. Mirrors the portfolio-tree traversal helpers backing
/// `ProjectQueryManager`.
pub struct ProjectGraph<'a> {
    projects: &'a [Project],
}

impl<'a> ProjectGraph<'a> {
    pub fn new(projects: &'a [Project]) -> Self {
        Self { projects }
    }

    fn by_uuid(&self, uuid: Uuid) -> Option<&'a Project> {
        self.projects.iter().find(|p| p.uuid == uuid)
    }

    /// Direct children of `uuid` (projects whose `parent` is `uuid`).
    pub fn children_of(&self, uuid: Uuid) -> Vec<&'a Project> {
        self.projects
            .iter()
            .filter(|p| p.parent == Some(uuid))
            .collect()
    }

    /// Ancestors of `uuid`, nearest first, walking the `parent` chain to the
    /// root. Defensive against malformed cycles via a visited set.
    pub fn ancestors_of(&self, uuid: Uuid) -> Vec<Uuid> {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut cur = self.by_uuid(uuid).and_then(|p| p.parent);
        while let Some(parent) = cur {
            if !seen.insert(parent) {
                break; // cycle guard
            }
            out.push(parent);
            cur = self.by_uuid(parent).and_then(|p| p.parent);
        }
        out
    }

    /// True if `candidate` is `ancestor` itself or a transitive child of it.
    pub fn is_descendant(&self, candidate: Uuid, ancestor: Uuid) -> bool {
        self.ancestors_of(candidate).contains(&ancestor)
    }

    /// Validate moving `child` under `new_parent`. Rejected when `new_parent`
    /// is `child` itself or a descendant of `child` (would create a cycle).
    /// Mirrors the upstream reparent guard in `ProjectQueryManager`.
    pub fn validate_reparent(&self, child: Uuid, new_parent: Uuid) -> Result<(), HierarchyError> {
        if new_parent == child || self.is_descendant(new_parent, child) {
            return Err(HierarchyError::Cycle);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::components::Project;
    use crate::components::hierarchy::{HierarchyError, ProjectGraph};
    use uuid::Uuid;

    fn child(name: &str, parent: Option<Uuid>) -> Project {
        let mut p = Project::new(name, Some("1.0.0".into()));
        p.parent = parent;
        p
    }

    #[test]
    fn is_active_defaults_to_true_when_unset() {
        let p = Project::new("p", None);
        assert!(p.active.is_none());
        assert!(p.is_active());
    }

    #[test]
    fn is_active_respects_explicit_false() {
        let mut p = Project::new("p", None);
        p.active = Some(false);
        assert!(!p.is_active());
    }

    #[test]
    fn active_none_serializes_as_true() {
        // Upstream BooleanDefaultTrueSerializer writes `true` for a null active.
        let p = Project::new("p", None);
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["active"], serde_json::json!(true));
    }

    #[test]
    fn children_of_returns_only_direct_children() {
        let root = child("root", None);
        let a = child("a", Some(root.uuid));
        let b = child("b", Some(root.uuid));
        let grandchild = child("gc", Some(a.uuid));
        let projects = vec![root.clone(), a.clone(), b.clone(), grandchild.clone()];
        let g = ProjectGraph::new(&projects);
        let mut kids: Vec<&str> = g.children_of(root.uuid).iter().map(|p| p.name.as_str()).collect();
        kids.sort();
        assert_eq!(kids, vec!["a", "b"]);
        // grandchild is NOT a direct child of root.
        assert!(!kids.contains(&"gc"));
    }

    #[test]
    fn ancestors_of_walks_parent_chain_rootward() {
        let root = child("root", None);
        let a = child("a", Some(root.uuid));
        let gc = child("gc", Some(a.uuid));
        let projects = vec![root.clone(), a.clone(), gc.clone()];
        let g = ProjectGraph::new(&projects);
        assert_eq!(g.ancestors_of(gc.uuid), vec![a.uuid, root.uuid]);
        assert!(g.ancestors_of(root.uuid).is_empty());
    }

    #[test]
    fn is_descendant_detects_transitive_children() {
        let root = child("root", None);
        let a = child("a", Some(root.uuid));
        let gc = child("gc", Some(a.uuid));
        let projects = vec![root.clone(), a.clone(), gc.clone()];
        let g = ProjectGraph::new(&projects);
        assert!(g.is_descendant(gc.uuid, root.uuid));
        assert!(g.is_descendant(a.uuid, root.uuid));
        assert!(!g.is_descendant(root.uuid, gc.uuid));
    }

    #[test]
    fn reparent_under_self_is_rejected() {
        let root = child("root", None);
        let projects = vec![root.clone()];
        let g = ProjectGraph::new(&projects);
        assert_eq!(g.validate_reparent(root.uuid, root.uuid), Err(HierarchyError::Cycle));
    }

    #[test]
    fn reparent_under_descendant_is_rejected() {
        let root = child("root", None);
        let a = child("a", Some(root.uuid));
        let gc = child("gc", Some(a.uuid));
        let projects = vec![root.clone(), a.clone(), gc.clone()];
        let g = ProjectGraph::new(&projects);
        // Making root a child of gc would create a cycle.
        assert_eq!(g.validate_reparent(root.uuid, gc.uuid), Err(HierarchyError::Cycle));
    }

    #[test]
    fn valid_reparent_is_allowed() {
        let root = child("root", None);
        let a = child("a", Some(root.uuid));
        let b = child("b", None);
        let projects = vec![root.clone(), a.clone(), b.clone()];
        let g = ProjectGraph::new(&projects);
        // Moving b under a is fine (a is not a descendant of b).
        assert_eq!(g.validate_reparent(b.uuid, a.uuid), Ok(()));
    }
}
