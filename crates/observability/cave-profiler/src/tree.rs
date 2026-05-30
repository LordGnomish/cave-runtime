// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Profiling stack tree — port of Grafana Pyroscope `pkg/model/tree.go`.
//!
//! A [`Tree`] aggregates leaf-rooted call stacks into a sorted prefix tree with
//! `self`/`total` sample weights per node. This is the in-memory representation
//! Pyroscope reduces profiles into before rendering a flame graph; the ports
//! here cover stack insertion (`InsertStack`), tree merge (`Merge`),
//! max-nodes truncation (`minValue`), folded/collapsed output (`WriteCollapsed`)
//! and A/B diffing (`flamegraph_diff.go`).

/// A single node in the profiling tree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Node {
    pub name: String,
    /// Samples attributed directly to this frame.
    pub self_value: i64,
    /// Samples attributed to this frame and all descendants.
    pub total: i64,
    pub children: Vec<Node>,
}

/// A profiling tree: an ordered forest of root nodes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tree {
    pub root: Vec<Node>,
}

impl Tree {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a root-first call stack carrying `v` samples.
    ///
    /// Children are kept sorted by name; `total` accumulates along the path and
    /// `self` is credited to the leaf. Non-positive `v` is ignored.
    pub fn insert_stack(&mut self, v: i64, stack: &[&str]) {
        if v <= 0 || stack.is_empty() {
            return;
        }
        let last = stack.len() - 1;
        let mut children = &mut self.root;
        for (idx, &name) in stack.iter().enumerate() {
            let i = match children.binary_search_by(|c| c.name.as_str().cmp(name)) {
                Ok(i) => i,
                Err(i) => {
                    children.insert(
                        i,
                        Node {
                            name: name.to_string(),
                            self_value: 0,
                            total: 0,
                            children: Vec::new(),
                        },
                    );
                    i
                }
            };
            let node = &mut children[i];
            node.total += v;
            if idx == last {
                node.self_value += v;
            }
            children = &mut node.children;
        }
    }

    /// Sum of `total` across root nodes.
    pub fn total(&self) -> i64 {
        self.root.iter().map(|n| n.total).sum()
    }

    /// Merge `src` into `self`, summing `self`/`total` for matching paths and
    /// inserting new branches in sorted order. Ports `tree.go` Merge.
    pub fn merge(&mut self, _src: &Tree) {
        // RED placeholder
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_single_stack_sets_self_and_total() {
        let mut t = Tree::new();
        t.insert_stack(10, &["main", "foo", "bar"]);
        assert_eq!(t.total(), 10);
        // root: main(total=10), child foo(total=10), child bar(total=10,self=10)
        assert_eq!(t.root.len(), 1);
        let main = &t.root[0];
        assert_eq!(main.name, "main");
        assert_eq!(main.total, 10);
        assert_eq!(main.self_value, 0);
        let foo = &main.children[0];
        assert_eq!(foo.name, "foo");
        assert_eq!(foo.total, 10);
        let bar = &foo.children[0];
        assert_eq!(bar.name, "bar");
        assert_eq!(bar.total, 10);
        assert_eq!(bar.self_value, 10);
    }

    #[test]
    fn insert_shared_prefix_aggregates() {
        let mut t = Tree::new();
        t.insert_stack(3, &["main", "a"]);
        t.insert_stack(5, &["main", "b"]);
        assert_eq!(t.total(), 8);
        let main = &t.root[0];
        assert_eq!(main.total, 8);
        assert_eq!(main.children.len(), 2);
        // children sorted by name: a then b
        assert_eq!(main.children[0].name, "a");
        assert_eq!(main.children[0].total, 3);
        assert_eq!(main.children[1].name, "b");
        assert_eq!(main.children[1].total, 5);
    }

    #[test]
    fn insert_same_stack_twice_sums_self() {
        let mut t = Tree::new();
        t.insert_stack(2, &["x"]);
        t.insert_stack(4, &["x"]);
        assert_eq!(t.total(), 6);
        assert_eq!(t.root[0].self_value, 6);
        assert_eq!(t.root[0].total, 6);
    }

    #[test]
    fn insert_keeps_children_sorted() {
        let mut t = Tree::new();
        t.insert_stack(1, &["r", "z"]);
        t.insert_stack(1, &["r", "a"]);
        t.insert_stack(1, &["r", "m"]);
        let names: Vec<&str> = t.root[0].children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["a", "m", "z"]);
    }

    #[test]
    fn insert_ignores_non_positive() {
        let mut t = Tree::new();
        t.insert_stack(0, &["a"]);
        t.insert_stack(-5, &["b"]);
        assert_eq!(t.total(), 0);
        assert!(t.root.is_empty());
    }

    #[test]
    fn merge_overlapping_paths_sums_weights() {
        let mut a = Tree::new();
        a.insert_stack(10, &["main", "foo"]);
        let mut b = Tree::new();
        b.insert_stack(5, &["main", "foo"]);
        b.insert_stack(7, &["main", "bar"]);
        a.merge(&b);
        assert_eq!(a.total(), 22);
        let main = &a.root[0];
        assert_eq!(main.total, 22);
        assert_eq!(main.children.len(), 2);
        // sorted: bar, foo
        assert_eq!(main.children[0].name, "bar");
        assert_eq!(main.children[0].total, 7);
        assert_eq!(main.children[0].self_value, 7);
        assert_eq!(main.children[1].name, "foo");
        assert_eq!(main.children[1].total, 15);
        assert_eq!(main.children[1].self_value, 15);
    }

    #[test]
    fn merge_into_empty_copies() {
        let mut a = Tree::new();
        let mut b = Tree::new();
        b.insert_stack(4, &["x", "y"]);
        a.merge(&b);
        assert_eq!(a.total(), 4);
        assert_eq!(a.root[0].name, "x");
        assert_eq!(a.root[0].children[0].name, "y");
    }

    #[test]
    fn merge_disjoint_roots_unions_sorted() {
        let mut a = Tree::new();
        a.insert_stack(1, &["z"]);
        let mut b = Tree::new();
        b.insert_stack(1, &["a"]);
        a.merge(&b);
        let names: Vec<&str> = a.root.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["a", "z"]);
        assert_eq!(a.total(), 2);
    }
}
