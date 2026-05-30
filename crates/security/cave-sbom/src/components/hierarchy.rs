// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/model/Project.java  (parent/children, active)
//   src/main/java/org/dependencytrack/persistence/ProjectQueryManager.java  (reparent cycle guard)
//
//! Project parent/children hierarchy — ancestor walk + reparent cycle guard.

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
