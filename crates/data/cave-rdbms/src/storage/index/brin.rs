// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! BRIN access method (Block Range INdex).
//!
//! Port of PostgreSQL `src/backend/access/brin/{brin.c,brin_minmax.c}`.
//!
//! BRIN stores, for each *page range* (a fixed run of `pages_per_range` heap
//! blocks), a tiny summary tuple — here the `minmax` opclass min/max of the
//! keys in that range. Scans are deliberately **lossy**: a range whose
//! `[min,max]` overlaps the scan key is emitted in full as a candidate block
//! set (the executor rechecks each tuple), while a range that cannot possibly
//! overlap is pruned without being read. This trades index precision for an
//! index orders of magnitude smaller than a btree.

use super::key_cmp;
use crate::types::SqlValue;
use std::cmp::Ordering;

type Tid = usize;

/// One `minmax` summary tuple covering a contiguous run of TIDs.
#[derive(Clone)]
pub struct BrinRange {
    /// first TID covered by this range
    pub start_tid: Tid,
    pub min: SqlValue,
    pub max: SqlValue,
}

/// A BRIN minmax index.
pub struct BrinIndex {
    range_size: usize,
    ranges: Vec<Option<BrinRange>>,
}

impl BrinIndex {
    /// `range_size` is the number of consecutive TIDs summarised per range
    /// (the `pages_per_range` storage parameter).
    pub fn new(range_size: usize) -> Self {
        assert!(range_size > 0, "range_size must be positive");
        BrinIndex {
            range_size,
            ranges: Vec::new(),
        }
    }

    /// Number of summarised page ranges.
    pub fn range_count(&self) -> usize {
        self.ranges.iter().filter(|r| r.is_some()).count()
    }

    /// `brin_minmax_add_value`: fold a key into its range's summary, widening
    /// the min/max bounds (and creating the summary tuple on first sight).
    pub fn insert(&mut self, tid: Tid, key: SqlValue) {
        let idx = tid / self.range_size;
        if idx >= self.ranges.len() {
            self.ranges.resize_with(idx + 1, || None);
        }
        let start_tid = idx * self.range_size;
        match &mut self.ranges[idx] {
            Some(r) => {
                if key_cmp(&key, &r.min) == Ordering::Less {
                    r.min = key.clone();
                }
                if key_cmp(&key, &r.max) == Ordering::Greater {
                    r.max = key;
                }
            }
            None => {
                self.ranges[idx] = Some(BrinRange {
                    start_tid,
                    min: key.clone(),
                    max: key,
                });
            }
        }
    }

    /// Summary tuples in TID order.
    pub fn summary(&self) -> Vec<BrinRange> {
        self.ranges.iter().flatten().cloned().collect()
    }

    /// `bringetbitmap`: candidate TIDs from every range whose `[min,max]`
    /// overlaps the inclusive `[lo,hi]` scan key (open bounds = unbounded).
    /// Lossy — the result is a superset of the true matches.
    pub fn search(&self, lo: Option<&SqlValue>, hi: Option<&SqlValue>) -> Vec<Tid> {
        let mut out = Vec::new();
        for r in self.ranges.iter().flatten() {
            // overlap test: range.max >= lo AND range.min <= hi
            let above_lo = lo.map(|l| key_cmp(&r.max, l) != Ordering::Less).unwrap_or(true);
            let below_hi = hi
                .map(|h| key_cmp(&r.min, h) != Ordering::Greater)
                .unwrap_or(true);
            if above_lo && below_hi {
                for t in r.start_tid..r.start_tid + self.range_size {
                    out.push(t);
                }
            }
        }
        out.sort_unstable();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_value_range_is_a_point_summary() {
        let mut brin = BrinIndex::new(8);
        brin.insert(3, SqlValue::Int4(42));
        let s = brin.summary();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].min, SqlValue::Int4(42));
        assert_eq!(s[0].max, SqlValue::Int4(42));
        assert_eq!(s[0].start_tid, 0);
    }

    #[test]
    fn fully_below_range_is_pruned() {
        let mut brin = BrinIndex::new(4);
        for tid in 0..4 {
            brin.insert(tid, SqlValue::Int4(tid as i32));
        }
        // keys >= 100 → range [0,3] pruned
        assert!(brin.search(Some(&SqlValue::Int4(100)), None).is_empty());
    }
}
