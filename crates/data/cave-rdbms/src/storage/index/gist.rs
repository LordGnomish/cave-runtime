// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! GiST access method (Generalized Search Tree).
//!
//! Port of PostgreSQL `src/backend/access/gist/`
//! (`gist.c`, `gistproc.c`, `gistsplit.c`) instantiated for the 1-D interval
//! opclass — the shape `range_ops` and `box_ops` build on.
//!
//! GiST is a height-balanced tree where every entry carries a *predicate*. For
//! intervals the predicate is a bounding box `[lo, hi]` and the opclass support
//! functions are:
//!
//!   * **consistent** — does an entry's bbox overlap the query? Drives subtree
//!     pruning during search.
//!   * **union** — the bounding interval covering a set of child boxes.
//!   * **penalty** — how much a bbox must enlarge to include a new entry; used
//!     by `gistchoose` to pick the cheapest insertion subtree.
//!   * **picksplit** — Guttman's linear split that partitions an overflowing
//!     node into two while keeping each above the minimum fill.

type Tid = usize;
/// Inclusive bounding interval `[lo, hi]`.
type Bbox = (i64, i64);

/// Max entries per node; a node overflows at `M + 1`.
const M: usize = 4;
/// Minimum entries kept in each half after a split.
const MIN_FILL: usize = 2;

enum Node {
    Leaf(Vec<(Bbox, Tid)>),
    Internal(Vec<(Bbox, Box<Node>)>),
}

/// A GiST index over 1-D intervals.
#[derive(Default)]
pub struct GistIndex {
    root: Option<Node>,
}

fn union(a: Bbox, b: Bbox) -> Bbox {
    (a.0.min(b.0), a.1.max(b.1))
}

/// consistent(): bounding box `b` overlaps query interval `q`.
fn overlaps(b: Bbox, q: Bbox) -> bool {
    b.0 <= q.1 && b.1 >= q.0
}

fn area(b: Bbox) -> i64 {
    b.1 - b.0
}

/// penalty(): enlargement of `b` required to cover `n`.
fn penalty(b: Bbox, n: Bbox) -> i64 {
    area(union(b, n)) - area(b)
}

impl GistIndex {
    pub fn new() -> Self {
        GistIndex { root: None }
    }

    pub fn is_empty(&self) -> bool {
        self.root.is_none()
    }

    /// Height in node levels (0 when empty).
    pub fn height(&self) -> usize {
        fn h(n: &Node) -> usize {
            match n {
                Node::Leaf(_) => 1,
                Node::Internal(children) => {
                    1 + children.first().map(|(_, c)| h(c)).unwrap_or(0)
                }
            }
        }
        self.root.as_ref().map(h).unwrap_or(0)
    }

    /// Insert interval `[lo, hi]` for heap TID.
    pub fn insert(&mut self, tid: Tid, lo: i64, hi: i64) {
        let bbox = (lo, hi);
        match self.root.take() {
            None => self.root = Some(Node::Leaf(vec![(bbox, tid)])),
            Some(mut root) => {
                if let Some((sib_bbox, sib)) = Self::insert_into(&mut root, bbox, tid) {
                    // Root split → grow a new internal level.
                    let root_bbox = node_bbox(&root);
                    self.root = Some(Node::Internal(vec![
                        (root_bbox, Box::new(root)),
                        (sib_bbox, sib),
                    ]));
                } else {
                    self.root = Some(root);
                }
            }
        }
    }

    /// Returns `Some((bbox, sibling))` when `node` split and a new sibling must
    /// be adopted by the parent.
    fn insert_into(node: &mut Node, bbox: Bbox, tid: Tid) -> Option<(Bbox, Box<Node>)> {
        match node {
            Node::Leaf(entries) => {
                entries.push((bbox, tid));
                if entries.len() > M {
                    let (keep, gone) = pick_split(std::mem::take(entries));
                    *entries = keep;
                    let sib_bbox = entries_bbox(&gone);
                    Some((sib_bbox, Box::new(Node::Leaf(gone))))
                } else {
                    None
                }
            }
            Node::Internal(children) => {
                // gistchoose: least-penalty subtree, tie-break by smaller area.
                let best = (0..children.len())
                    .min_by(|&a, &b| {
                        let (cb_a, cb_b) = (children[a].0, children[b].0);
                        penalty(cb_a, bbox)
                            .cmp(&penalty(cb_b, bbox))
                            .then(area(cb_a).cmp(&area(cb_b)))
                    })
                    .expect("internal node is non-empty");

                let split = Self::insert_into(&mut children[best].1, bbox, tid);
                // Tighten the chosen child's stored predicate.
                children[best].0 = node_bbox(&children[best].1);

                if let Some((sib_bbox, sib)) = split {
                    children.push((sib_bbox, sib));
                    if children.len() > M {
                        let (keep, gone) = pick_split(std::mem::take(children));
                        *children = keep;
                        let sib_bbox = entries_bbox(&gone);
                        return Some((sib_bbox, Box::new(Node::Internal(gone))));
                    }
                }
                None
            }
        }
    }

    /// All TIDs whose interval overlaps `[qlo, qhi]` (leaf-exact).
    pub fn search_overlap(&self, qlo: i64, qhi: i64) -> Vec<Tid> {
        let mut out = Vec::new();
        if let Some(root) = self.root.as_ref() {
            search(root, (qlo, qhi), &mut out);
        }
        out
    }

    /// Degenerate overlap query: intervals containing the point `p`.
    pub fn search_contains_point(&self, p: i64) -> Vec<Tid> {
        self.search_overlap(p, p)
    }
}

fn search(node: &Node, q: Bbox, out: &mut Vec<Tid>) {
    match node {
        Node::Leaf(entries) => {
            for (b, tid) in entries {
                if overlaps(*b, q) {
                    out.push(*tid);
                }
            }
        }
        Node::Internal(children) => {
            for (b, child) in children {
                if overlaps(*b, q) {
                    search(child, q, out);
                }
            }
        }
    }
}

fn node_bbox(node: &Node) -> Bbox {
    match node {
        Node::Leaf(e) => entries_bbox(e),
        Node::Internal(c) => entries_bbox(c),
    }
}

fn entries_bbox<T>(entries: &[(Bbox, T)]) -> Bbox {
    let mut it = entries.iter();
    let first = it.next().expect("non-empty entry set").0;
    it.fold(first, |acc, (b, _)| union(acc, *b))
}

/// Guttman linear `picksplit`: seed with the most-separated pair along the
/// single dimension, then assign each remaining entry to whichever group needs
/// the least enlargement, honouring the minimum fill on both sides.
fn pick_split<T>(items: Vec<(Bbox, T)>) -> (Vec<(Bbox, T)>, Vec<(Bbox, T)>) {
    let n = items.len();
    // Seeds: entry with the highest low side and entry with the lowest high side.
    let mut seed_hi_lo = 0usize;
    let mut seed_lo_hi = 0usize;
    let (mut max_lo, mut min_hi) = (i64::MIN, i64::MAX);
    for (i, (b, _)) in items.iter().enumerate() {
        if b.0 > max_lo {
            max_lo = b.0;
            seed_hi_lo = i;
        }
        if b.1 < min_hi {
            min_hi = b.1;
            seed_lo_hi = i;
        }
    }
    let (mut sa, mut sb) = (seed_hi_lo, seed_lo_hi);
    if sa == sb {
        sa = 0;
        sb = n - 1;
    }

    let mut slots: Vec<Option<(Bbox, T)>> = items.into_iter().map(Some).collect();
    let mut g1: Vec<(Bbox, T)> = vec![slots[sa].take().unwrap()];
    let mut g2: Vec<(Bbox, T)> = vec![slots[sb].take().unwrap()];
    let mut bb1 = g1[0].0;
    let mut bb2 = g2[0].0;

    loop {
        let remaining: Vec<usize> = (0..n).filter(|&i| slots[i].is_some()).collect();
        if remaining.is_empty() {
            break;
        }
        // Force the rest into a group that would otherwise starve.
        if g1.len() + remaining.len() == MIN_FILL {
            for i in remaining {
                let e = slots[i].take().unwrap();
                bb1 = union(bb1, e.0);
                g1.push(e);
            }
            continue;
        }
        if g2.len() + remaining.len() == MIN_FILL {
            for i in remaining {
                let e = slots[i].take().unwrap();
                bb2 = union(bb2, e.0);
                g2.push(e);
            }
            continue;
        }
        let i = remaining[0];
        let nb = slots[i].as_ref().unwrap().0;
        let (p1, p2) = (penalty(bb1, nb), penalty(bb2, nb));
        let to_g1 = p1 < p2
            || (p1 == p2 && (area(bb1) < area(bb2) || (area(bb1) == area(bb2) && g1.len() <= g2.len())));
        let e = slots[i].take().unwrap();
        if to_g1 {
            bb1 = union(bb1, e.0);
            g1.push(e);
        } else {
            bb2 = union(bb2, e.0);
            g2.push(e);
        }
    }

    (g1, g2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_penalty_overlap_primitives() {
        assert_eq!(union((1, 5), (3, 9)), (1, 9));
        assert!(overlaps((1, 5), (4, 6)));
        assert!(!overlaps((1, 5), (6, 9)));
        // enlarging [1,5] to include [3,9] adds 4 to the length
        assert_eq!(penalty((1, 5), (3, 9)), 4);
    }

    #[test]
    fn single_insert_is_a_leaf_root() {
        let mut g = GistIndex::new();
        g.insert(0, 2, 8);
        assert_eq!(g.height(), 1);
        assert_eq!(g.search_overlap(5, 5), vec![0]);
    }
}
