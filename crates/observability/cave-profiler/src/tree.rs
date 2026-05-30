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
    pub fn merge(&mut self, src: &Tree) {
        merge_children(&mut self.root, &src.root);
    }
}

impl Tree {
    /// The minimum `total` a node must have to appear when the flame graph is
    /// capped at `max_nodes` — i.e. the `max_nodes`-th largest node total, or
    /// `0` if the tree has fewer than `max_nodes` nodes (no cap needed).
    /// Ports `tree.go` minValue.
    pub fn min_value(&self, max_nodes: i64) -> i64 {
        if max_nodes < 1 {
            return 0;
        }
        // Collect every node's total via DFS.
        let mut totals: Vec<i64> = Vec::new();
        let mut stack: Vec<&Node> = self.root.iter().collect();
        while let Some(n) = stack.pop() {
            totals.push(n.total);
            stack.extend(n.children.iter());
        }
        let max_nodes = max_nodes as usize;
        if totals.len() < max_nodes {
            return 0;
        }
        // Descending sort; the max_nodes-th largest is the threshold (heap top).
        totals.sort_unstable_by(|a, b| b.cmp(a));
        totals[max_nodes - 1]
    }

    /// Total node count (every node in the forest). Ports `tree.go` size.
    pub fn size(&self) -> usize {
        let mut count = 0;
        let mut stack: Vec<&Node> = self.root.iter().collect();
        while let Some(n) = stack.pop() {
            count += 1;
            stack.extend(n.children.iter());
        }
        count
    }

    /// Cap the flame graph at `max_nodes`: subtrees whose `total` is below the
    /// [`min_value`](Self::min_value) threshold collapse into a synthetic
    /// `"other"` sibling carrying their summed weight.
    pub fn truncate(&mut self, max_nodes: i64) {
        let min = self.min_value(max_nodes);
        if min <= 0 {
            return;
        }
        truncate_children(&mut self.root, min);
    }
}

/// Collapse below-threshold subtrees in `nodes` into a single `"other"` sibling.
fn truncate_children(nodes: &mut Vec<Node>, min: i64) {
    let mut kept: Vec<Node> = Vec::with_capacity(nodes.len());
    let mut other = 0i64;
    for mut n in nodes.drain(..) {
        if n.total < min {
            // Whole subtree weight folds into "other".
            other += n.total;
        } else {
            truncate_children(&mut n.children, min);
            kept.push(n);
        }
    }
    if other > 0 {
        kept.push(Node {
            name: "other".to_string(),
            self_value: other,
            total: other,
            children: Vec::new(),
        });
        kept.sort_by(|a, b| a.name.cmp(&b.name));
    }
    *nodes = kept;
}

/// A node in an A/B diff flame graph: the same frame's `total` weight on the
/// left (baseline) and right (comparison) side. Ports the alignment in
/// `pkg/model/flamegraph_diff.go`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffNode {
    pub name: String,
    pub left: i64,
    pub right: i64,
    pub children: Vec<DiffNode>,
}

impl Tree {
    /// Diff two trees, aligning frames by name at each level into a combined
    /// tree carrying the left and right `total` per frame.
    pub fn diff(left: &Tree, right: &Tree) -> Vec<DiffNode> {
        diff_level(&left.root, &right.root)
    }

    /// Build a tree from Brendan-Gregg "folded"/collapsed text — the inverse of
    /// [`collapsed`](Self::collapsed). Each non-blank line is
    /// `frame1;frame2;... <count>`; malformed lines are skipped.
    pub fn from_collapsed(_text: &str) -> Tree {
        // RED placeholder
        Tree::new()
    }

    /// Render the tree as Brendan-Gregg "folded"/collapsed stacks: one line per
    /// frame with non-zero self weight, root-first frames joined by `;` then a
    /// space and the self count. Ports `tree.go` WriteCollapsed.
    pub fn collapsed(&self) -> String {
        let mut out = String::new();
        let mut path: Vec<&str> = Vec::new();
        collapse_walk(&self.root, &mut path, &mut out);
        out
    }
}

/// Pre-order DFS emitting a folded line per node with self weight > 0.
fn collapse_walk<'a>(nodes: &'a [Node], path: &mut Vec<&'a str>, out: &mut String) {
    for n in nodes {
        path.push(n.name.as_str());
        if n.self_value > 0 {
            out.push_str(&path.join(";"));
            out.push(' ');
            out.push_str(&n.self_value.to_string());
            out.push('\n');
        }
        collapse_walk(&n.children, path, out);
        path.pop();
    }
}

/// Sorted merge-join of two name-sorted child lists into diff nodes.
fn diff_level(l: &[Node], r: &[Node]) -> Vec<DiffNode> {
    let mut out: Vec<DiffNode> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < l.len() || j < r.len() {
        let take_left = j >= r.len() || (i < l.len() && l[i].name <= r[j].name);
        let take_right = i >= l.len() || (j < r.len() && r[j].name <= l[i].name);
        match (take_left, take_right) {
            // same name on both sides
            (true, true) => {
                out.push(DiffNode {
                    name: l[i].name.clone(),
                    left: l[i].total,
                    right: r[j].total,
                    children: diff_level(&l[i].children, &r[j].children),
                });
                i += 1;
                j += 1;
            }
            // present only on the left
            (true, false) => {
                out.push(DiffNode {
                    name: l[i].name.clone(),
                    left: l[i].total,
                    right: 0,
                    children: diff_level(&l[i].children, &[]),
                });
                i += 1;
            }
            // present only on the right
            (false, true) => {
                out.push(DiffNode {
                    name: r[j].name.clone(),
                    left: 0,
                    right: r[j].total,
                    children: diff_level(&[], &r[j].children),
                });
                j += 1;
            }
            (false, false) => unreachable!(),
        }
    }
    out
}

/// Merge `src` nodes into the sorted `dst` child list, recursing into
/// matching children.
fn merge_children(dst: &mut Vec<Node>, src: &[Node]) {
    for s in src {
        let i = match dst.binary_search_by(|c| c.name.cmp(&s.name)) {
            Ok(i) => i,
            Err(i) => {
                dst.insert(
                    i,
                    Node {
                        name: s.name.clone(),
                        self_value: 0,
                        total: 0,
                        children: Vec::new(),
                    },
                );
                i
            }
        };
        dst[i].self_value += s.self_value;
        dst[i].total += s.total;
        merge_children(&mut dst[i].children, &s.children);
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

    // node totals: a=10, b=8, c=5, d=3  -> sorted desc [10, 8, 5, 3]
    fn mv_tree() -> Tree {
        let mut t = Tree::new();
        t.insert_stack(10, &["a"]);
        t.insert_stack(5, &["b", "c"]);
        t.insert_stack(3, &["b", "d"]);
        t
    }

    #[test]
    fn size_counts_all_nodes() {
        assert_eq!(mv_tree().size(), 4);
        assert_eq!(Tree::new().size(), 0);
    }

    #[test]
    fn min_value_is_nth_largest_total() {
        let t = mv_tree();
        assert_eq!(t.min_value(1), 10);
        assert_eq!(t.min_value(2), 8);
        assert_eq!(t.min_value(3), 5);
        assert_eq!(t.min_value(4), 3);
    }

    #[test]
    fn min_value_zero_when_fewer_nodes_than_cap() {
        let t = mv_tree();
        assert_eq!(t.min_value(5), 0);
        assert_eq!(t.min_value(0), 0);
    }

    #[test]
    fn truncate_collapses_below_threshold_into_other() {
        let mut t = mv_tree();
        t.truncate(2); // threshold = min_value(2) = 8
        // total weight is preserved
        assert_eq!(t.total(), 18);
        // root keeps a(10) and b(8)
        assert_eq!(t.root.len(), 2);
        assert_eq!(t.root[0].name, "a");
        let b = &t.root[1];
        assert_eq!(b.name, "b");
        // c(5) and d(3) both below 8 -> collapsed into one "other"
        assert_eq!(b.children.len(), 1);
        assert_eq!(b.children[0].name, "other");
        assert_eq!(b.children[0].total, 8);
        assert_eq!(b.children[0].self_value, 8);
    }

    #[test]
    fn diff_aligns_frames_left_and_right() {
        let mut left = Tree::new();
        left.insert_stack(10, &["main", "foo"]);
        let mut right = Tree::new();
        right.insert_stack(4, &["main", "foo"]);
        right.insert_stack(6, &["main", "bar"]);

        let d = Tree::diff(&left, &right);
        assert_eq!(d.len(), 1);
        let main = &d[0];
        assert_eq!(main.name, "main");
        assert_eq!(main.left, 10);
        assert_eq!(main.right, 10);
        // children union sorted: bar then foo
        assert_eq!(main.children.len(), 2);
        assert_eq!(main.children[0].name, "bar");
        assert_eq!(main.children[0].left, 0);
        assert_eq!(main.children[0].right, 6);
        assert_eq!(main.children[1].name, "foo");
        assert_eq!(main.children[1].left, 10);
        assert_eq!(main.children[1].right, 4);
    }

    #[test]
    fn diff_identical_trees_match() {
        let mut a = Tree::new();
        a.insert_stack(5, &["x", "y"]);
        let d = Tree::diff(&a, &a);
        assert_eq!(d[0].name, "x");
        assert_eq!(d[0].left, d[0].right);
        assert_eq!(d[0].children[0].left, 5);
        assert_eq!(d[0].children[0].right, 5);
    }

    #[test]
    fn diff_left_only_branch_has_zero_right() {
        let mut left = Tree::new();
        left.insert_stack(3, &["only_left"]);
        let right = Tree::new();
        let d = Tree::diff(&left, &right);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].name, "only_left");
        assert_eq!(d[0].left, 3);
        assert_eq!(d[0].right, 0);
    }

    #[test]
    fn collapsed_emits_folded_stacks_root_first() {
        let mut t = Tree::new();
        t.insert_stack(2, &["a"]);
        t.insert_stack(10, &["a", "b"]);
        t.insert_stack(5, &["a", "c"]);
        let out = t.collapsed();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines, vec!["a 2", "a;b 10", "a;c 5"]);
    }

    #[test]
    fn collapsed_skips_zero_self_internal_nodes() {
        let mut t = Tree::new();
        t.insert_stack(7, &["root", "leaf"]);
        let out = t.collapsed();
        // "root" has self 0 -> not emitted; only the leaf line appears
        assert_eq!(out.lines().collect::<Vec<_>>(), vec!["root;leaf 7"]);
    }

    #[test]
    fn collapsed_empty_tree_is_empty() {
        assert_eq!(Tree::new().collapsed(), "");
    }

    #[test]
    fn from_collapsed_parses_folded_text() {
        let t = Tree::from_collapsed("a;b 10\na;c 5\na 2\n");
        assert_eq!(t.total(), 17);
        let a = &t.root[0];
        assert_eq!(a.name, "a");
        assert_eq!(a.self_value, 2);
        assert_eq!(a.children.len(), 2);
        assert_eq!(a.children[0].name, "b");
        assert_eq!(a.children[0].self_value, 10);
        assert_eq!(a.children[1].name, "c");
        assert_eq!(a.children[1].self_value, 5);
    }

    #[test]
    fn from_collapsed_skips_blank_and_malformed() {
        let t = Tree::from_collapsed("\n   \nq 9\nbroken_no_count\n");
        assert_eq!(t.total(), 9);
        assert_eq!(t.root.len(), 1);
        assert_eq!(t.root[0].name, "q");
    }

    #[test]
    fn collapsed_round_trips_through_from_collapsed() {
        let mut t = Tree::new();
        t.insert_stack(3, &["x", "y"]);
        t.insert_stack(4, &["x", "z"]);
        t.insert_stack(1, &["w"]);
        let round = Tree::from_collapsed(&t.collapsed());
        assert_eq!(round, t);
    }
}
