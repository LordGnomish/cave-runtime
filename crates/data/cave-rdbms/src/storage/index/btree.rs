// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! B-tree access method (`nbtree`).
//!
//! Port of PostgreSQL `src/backend/access/nbtree/{nbtinsert.c,nbtsearch.c}`.
//! A balanced multiway search tree keyed on [`SqlValue`] mapping each key to a
//! posting list of heap TIDs (row offsets). Like upstream nbtree it supports
//! duplicate keys (multiple TIDs per key, kept in insertion order), equality
//! probes, and ordered forward range scans with inclusive/open bounds.
//!
//! The structure is a classic CLRS B-tree with minimum degree `T`: every node
//! except the root holds between `T-1` and `2T-1` keys, and full children are
//! split pre-emptively on the way down so a single descent both finds the
//! insertion point and keeps the tree balanced.

use super::key_cmp;
use crate::types::SqlValue;

/// Minimum degree. A node holds at most `2*T - 1` separator keys.
const T: usize = 4;

type Tid = usize;

struct Entry {
    key: SqlValue,
    tids: Vec<Tid>,
}

struct Node {
    entries: Vec<Entry>,
    children: Vec<Box<Node>>,
    leaf: bool,
}

impl Node {
    fn new_leaf() -> Box<Node> {
        Box::new(Node {
            entries: Vec::new(),
            children: Vec::new(),
            leaf: true,
        })
    }

    fn is_full(&self) -> bool {
        self.entries.len() == 2 * T - 1
    }
}

/// A B-tree secondary index over `SqlValue` keys.
#[derive(Default)]
pub struct BTreeIndex {
    root: Option<Box<Node>>,
    /// total number of (key, tid) pairs stored
    entries: usize,
}

impl BTreeIndex {
    pub fn new() -> Self {
        BTreeIndex {
            root: None,
            entries: 0,
        }
    }

    /// Number of (key, tid) pairs in the index.
    pub fn len(&self) -> usize {
        self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries == 0
    }

    /// Tree height (number of node levels); 0 for an empty index.
    pub fn height(&self) -> usize {
        fn h(n: &Node) -> usize {
            if n.leaf {
                1
            } else {
                1 + h(&n.children[0])
            }
        }
        self.root.as_ref().map(|r| h(r)).unwrap_or(0)
    }

    /// Insert a heap TID under `key`. Duplicate keys append the TID to the
    /// existing posting list in insertion order.
    pub fn insert(&mut self, key: SqlValue, tid: Tid) {
        self.entries += 1;

        // Fast path: key already present — append to its posting list.
        if let Some(list) = self.find_mut(&key) {
            list.push(tid);
            return;
        }

        // CLRS B-tree insert with pre-emptive root split.
        if self.root.is_none() {
            self.root = Some(Node::new_leaf());
        }
        if self.root.as_ref().unwrap().is_full() {
            let mut new_root = Box::new(Node {
                entries: Vec::new(),
                children: vec![self.root.take().unwrap()],
                leaf: false,
            });
            Self::split_child(&mut new_root, 0);
            self.root = Some(new_root);
        }
        Self::insert_nonfull(self.root.as_mut().unwrap(), key, tid);
    }

    fn find_mut(&mut self, key: &SqlValue) -> Option<&mut Vec<Tid>> {
        let mut node = self.root.as_mut()?;
        loop {
            match node.entries.binary_search_by(|e| key_cmp(&e.key, key)) {
                Ok(i) => return Some(&mut node.entries[i].tids),
                Err(i) => {
                    if node.leaf {
                        return None;
                    }
                    node = &mut node.children[i];
                }
            }
        }
    }

    /// Split the full child `node.children[i]` around its median.
    fn split_child(node: &mut Node, i: usize) {
        let child = &mut node.children[i];
        let mid = T - 1;

        // Right half of entries / children move to a fresh sibling.
        let right_entries = child.entries.split_off(mid + 1);
        let median = child.entries.pop().expect("full child has a median");
        let right_children = if child.leaf {
            Vec::new()
        } else {
            child.children.split_off(mid + 1)
        };

        let sibling = Box::new(Node {
            entries: right_entries,
            children: right_children,
            leaf: child.leaf,
        });

        node.entries.insert(i, median);
        node.children.insert(i + 1, sibling);
    }

    fn insert_nonfull(node: &mut Node, key: SqlValue, tid: Tid) {
        match node.entries.binary_search_by(|e| key_cmp(&e.key, &key)) {
            Ok(i) => {
                // Key materialised here after an earlier descent split.
                node.entries[i].tids.push(tid);
            }
            Err(mut i) => {
                if node.leaf {
                    node.entries.insert(
                        i,
                        Entry {
                            key,
                            tids: vec![tid],
                        },
                    );
                } else {
                    if node.children[i].is_full() {
                        Self::split_child(node, i);
                        // The promoted median may now equal or precede `key`.
                        match key_cmp(&node.entries[i].key, &key) {
                            std::cmp::Ordering::Equal => {
                                node.entries[i].tids.push(tid);
                                return;
                            }
                            std::cmp::Ordering::Less => i += 1,
                            std::cmp::Ordering::Greater => {}
                        }
                    }
                    Self::insert_nonfull(&mut node.children[i], key, tid);
                }
            }
        }
    }

    /// Equality probe: heap TIDs stored under `key`, in insertion order.
    pub fn search(&self, key: &SqlValue) -> Vec<Tid> {
        let mut node = match self.root.as_ref() {
            Some(r) => r,
            None => return Vec::new(),
        };
        loop {
            match node.entries.binary_search_by(|e| key_cmp(&e.key, key)) {
                Ok(i) => return node.entries[i].tids.clone(),
                Err(i) => {
                    if node.leaf {
                        return Vec::new();
                    }
                    node = &node.children[i];
                }
            }
        }
    }

    /// Ordered forward range scan. `lo`/`hi` are inclusive bounds; `None` means
    /// unbounded on that side. Returns `(key, tid)` pairs in ascending key
    /// order, expanding posting lists in insertion order.
    pub fn range_scan(
        &self,
        lo: Option<&SqlValue>,
        hi: Option<&SqlValue>,
    ) -> Vec<(SqlValue, Tid)> {
        let mut out = Vec::new();
        if let Some(root) = self.root.as_ref() {
            Self::walk(root, lo, hi, &mut out);
        }
        out
    }

    fn walk(
        node: &Node,
        lo: Option<&SqlValue>,
        hi: Option<&SqlValue>,
        out: &mut Vec<(SqlValue, Tid)>,
    ) {
        use std::cmp::Ordering::*;
        for i in 0..node.entries.len() {
            if !node.leaf {
                // Descend into the left child unless the whole subtree is below `lo`.
                let below_lo = lo
                    .map(|l| key_cmp(&node.entries[i].key, l) == Less)
                    .unwrap_or(false);
                if !below_lo {
                    Self::walk(&node.children[i], lo, hi, out);
                }
            }
            let k = &node.entries[i].key;
            let ge_lo = lo.map(|l| key_cmp(k, l) != Less).unwrap_or(true);
            let le_hi = hi.map(|h| key_cmp(k, h) != Greater).unwrap_or(true);
            if ge_lo && le_hi {
                for &t in &node.entries[i].tids {
                    out.push((k.clone(), t));
                }
            }
        }
        if !node.leaf {
            let last = node.entries.len();
            let above_hi = hi
                .and_then(|h| node.entries.last().map(|e| key_cmp(&e.key, h) == Greater))
                .unwrap_or(false);
            if !above_hi {
                Self::walk(&node.children[last], lo, hi, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_index_probes_and_scans_clean() {
        let idx = BTreeIndex::new();
        assert!(idx.is_empty());
        assert_eq!(idx.height(), 0);
        assert!(idx.search(&SqlValue::Int4(1)).is_empty());
        assert!(idx.range_scan(None, None).is_empty());
    }

    #[test]
    fn split_promotes_median_and_grows_height() {
        let mut idx = BTreeIndex::new();
        // 2T-1 = 7 keys fill the root leaf; the 8th forces a split → height 2.
        for k in 1..=7 {
            idx.insert(SqlValue::Int4(k), k as usize);
        }
        assert_eq!(idx.height(), 1);
        idx.insert(SqlValue::Int4(8), 8);
        assert_eq!(idx.height(), 2);
        assert_eq!(idx.search(&SqlValue::Int4(4)), vec![4]);
    }
}
