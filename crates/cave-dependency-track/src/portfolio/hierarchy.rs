// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Project hierarchy — parent/child tree views over the portfolio.

use crate::models::Project;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectNode {
    pub project: Project,
    pub children: Vec<ProjectNode>,
}

pub fn build_tree(projects: &[Project]) -> Vec<ProjectNode> {
    let mut by_parent: HashMap<Option<Uuid>, Vec<Project>> = HashMap::new();
    let known: HashSet<Uuid> = projects.iter().map(|p| p.uuid).collect();
    for p in projects {
        let key = p.parent.filter(|u| known.contains(u));
        by_parent.entry(key).or_default().push(p.clone());
    }

    fn build_inner(
        parent: Option<Uuid>,
        by_parent: &HashMap<Option<Uuid>, Vec<Project>>,
        visiting: &mut HashSet<Uuid>,
    ) -> Vec<ProjectNode> {
        let mut out = Vec::new();
        let Some(children) = by_parent.get(&parent) else {
            return out;
        };
        let mut sorted = children.clone();
        sorted.sort_by(|a, b| a.name.cmp(&b.name));
        for child in sorted {
            if !visiting.insert(child.uuid) {
                continue;
            }
            let kids = build_inner(Some(child.uuid), by_parent, visiting);
            visiting.remove(&child.uuid);
            out.push(ProjectNode {
                project: child,
                children: kids,
            });
        }
        out
    }

    let mut visiting = HashSet::new();
    build_inner(None, &by_parent, &mut visiting)
}

pub fn descendants(root: Uuid, projects: &[Project]) -> Vec<Uuid> {
    let by_parent: HashMap<Uuid, Vec<Uuid>> = projects.iter().fold(HashMap::new(), |mut m, p| {
        if let Some(parent) = p.parent {
            m.entry(parent).or_default().push(p.uuid);
        }
        m
    });
    let mut out = Vec::new();
    let mut stack = vec![root];
    let mut seen = HashSet::new();
    seen.insert(root);
    while let Some(cur) = stack.pop() {
        if let Some(kids) = by_parent.get(&cur) {
            for k in kids {
                if seen.insert(*k) {
                    out.push(*k);
                    stack.push(*k);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Classifier;

    fn make(name: &str, parent: Option<Uuid>) -> Project {
        let mut p = Project::new(name, Classifier::Application);
        p.parent = parent;
        p
    }

    #[test]
    fn build_tree_separates_roots_and_children() {
        let r = make("root", None);
        let c1 = make("c1", Some(r.uuid));
        let c2 = make("c2", Some(r.uuid));
        let g1 = make("g1", Some(c1.uuid));
        let tree = build_tree(&[r.clone(), c1.clone(), c2.clone(), g1.clone()]);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].project.uuid, r.uuid);
        assert_eq!(tree[0].children.len(), 2);
        let c1n = tree[0].children.iter().find(|n| n.project.uuid == c1.uuid).unwrap();
        assert_eq!(c1n.children.len(), 1);
        assert_eq!(c1n.children[0].project.uuid, g1.uuid);
    }

    #[test]
    fn build_tree_orphans_become_root() {
        let lone = make("orphan", Some(Uuid::new_v4()));
        let tree = build_tree(&[lone.clone()]);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].project.uuid, lone.uuid);
    }

    #[test]
    fn build_tree_breaks_cycle_safely() {
        let mut a = make("a", None);
        let b = make("b", Some(a.uuid));
        a.parent = Some(b.uuid);
        let tree = build_tree(&[a.clone(), b.clone()]);
        assert!(!tree.is_empty());
    }

    #[test]
    fn descendants_preorder() {
        let r = make("r", None);
        let c = make("c", Some(r.uuid));
        let g = make("g", Some(c.uuid));
        let d = descendants(r.uuid, &[r.clone(), c.clone(), g.clone()]);
        assert!(d.contains(&c.uuid));
        assert!(d.contains(&g.uuid));
        assert_eq!(d.len(), 2);
    }

    #[test]
    fn descendants_empty_for_leaf() {
        let r = make("r", None);
        let d = descendants(r.uuid, &[r.clone()]);
        assert!(d.is_empty());
    }

    #[test]
    fn build_tree_sorts_children_alphabetically() {
        let r = make("r", None);
        let z = make("z", Some(r.uuid));
        let a = make("a", Some(r.uuid));
        let tree = build_tree(&[r.clone(), z.clone(), a.clone()]);
        assert_eq!(tree[0].children[0].project.name, "a");
        assert_eq!(tree[0].children[1].project.name, "z");
    }
}
