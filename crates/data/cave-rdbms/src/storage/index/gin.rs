// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! GIN access method (Generalized INverted index).
//!
//! Port of PostgreSQL `src/backend/access/gin/`
//! (`gininsert.c`, `ginget.c`, `ginlogic.c`).
//!
//! GIN indexes composite values: each indexed datum is decomposed by its
//! opclass into a set of keys (array elements, tsvector lexemes, jsonb paths),
//! and the **entry tree** maps each key to a sorted, de-duplicated posting list
//! of heap TIDs. Searches run the opclass *consistent* function over the
//! per-key posting lists:
//!
//!   * overlap (`&&`, `ANY`)      → union of the matching posting lists
//!   * containment (`@>`, `ALL`)  → intersection of the posting lists
//!
//! The entry list is kept sorted by [`key_cmp`](super::key_cmp) so lookups are
//! `O(log k)` binary searches, mirroring the upstream entry-tree descent.

use super::key_cmp;
use crate::types::SqlValue;

type Tid = usize;

struct EntryPosting {
    key: SqlValue,
    /// sorted, de-duplicated heap TIDs
    tids: Vec<Tid>,
}

/// An inverted (GIN) index over decomposed `SqlValue` keys.
#[derive(Default)]
pub struct GinIndex {
    /// entry tree: posting lists sorted by key
    entries: Vec<EntryPosting>,
}

impl GinIndex {
    pub fn new() -> Self {
        GinIndex {
            entries: Vec::new(),
        }
    }

    /// Number of distinct keys in the entry tree.
    pub fn key_count(&self) -> usize {
        self.entries.len()
    }

    /// Index a heap TID under each of its decomposed `keys`
    /// (`ginInsertItemPointers`). Re-inserting the same (tid, key) is a no-op.
    pub fn insert(&mut self, tid: Tid, keys: Vec<SqlValue>) {
        for key in keys {
            match self.entries.binary_search_by(|e| key_cmp(&e.key, &key)) {
                Ok(i) => {
                    let list = &mut self.entries[i].tids;
                    if let Err(pos) = list.binary_search(&tid) {
                        list.insert(pos, tid);
                    }
                }
                Err(i) => self.entries.insert(
                    i,
                    EntryPosting {
                        key,
                        tids: vec![tid],
                    },
                ),
            }
        }
    }

    /// Sorted posting list for a single key (empty if absent).
    pub fn posting_list(&self, key: &SqlValue) -> Vec<Tid> {
        match self.entries.binary_search_by(|e| key_cmp(&e.key, key)) {
            Ok(i) => self.entries[i].tids.clone(),
            Err(_) => Vec::new(),
        }
    }

    /// Containment (`@>`): TIDs present in the posting lists of **all** query
    /// keys — the intersection. Empty query matches nothing.
    pub fn search_all(&self, keys: &[SqlValue]) -> Vec<Tid> {
        if keys.is_empty() {
            return Vec::new();
        }
        let mut acc = self.posting_list(&keys[0]);
        for k in &keys[1..] {
            if acc.is_empty() {
                break;
            }
            acc = intersect_sorted(&acc, &self.posting_list(k));
        }
        acc
    }

    /// Overlap (`&&`): TIDs present in the posting list of **any** query key —
    /// the union. Empty query matches nothing.
    pub fn search_any(&self, keys: &[SqlValue]) -> Vec<Tid> {
        let mut acc: Vec<Tid> = Vec::new();
        for k in keys {
            acc = union_sorted(&acc, &self.posting_list(k));
        }
        acc
    }
}

/// Sorted-list intersection (both inputs ascending & unique).
fn intersect_sorted(a: &[Tid], b: &[Tid]) -> Vec<Tid> {
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                out.push(a[i]);
                i += 1;
                j += 1;
            }
        }
    }
    out
}

/// Sorted-list union (both inputs ascending & unique).
fn union_sorted(a: &[Tid], b: &[Tid]) -> Vec<Tid> {
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::with_capacity(a.len() + b.len());
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => {
                out.push(a[i]);
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                out.push(b[j]);
                j += 1;
            }
            std::cmp::Ordering::Equal => {
                out.push(a[i]);
                i += 1;
                j += 1;
            }
        }
    }
    out.extend_from_slice(&a[i..]);
    out.extend_from_slice(&b[j..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> SqlValue {
        SqlValue::Text(s.into())
    }

    #[test]
    fn intersect_and_union_helpers() {
        assert_eq!(intersect_sorted(&[1, 2, 3, 5], &[2, 5, 9]), vec![2, 5]);
        assert_eq!(union_sorted(&[1, 3], &[2, 3, 4]), vec![1, 2, 3, 4]);
    }

    #[test]
    fn entry_tree_stays_key_sorted() {
        let mut gin = GinIndex::new();
        gin.insert(0, vec![t("m"), t("a"), t("z")]);
        // probing in any order resolves via binary search
        assert_eq!(gin.posting_list(&t("a")), vec![0]);
        assert_eq!(gin.posting_list(&t("z")), vec![0]);
        assert_eq!(gin.key_count(), 3);
    }
}
