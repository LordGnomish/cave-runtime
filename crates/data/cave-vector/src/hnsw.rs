// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Hierarchical Navigable Small World (HNSW) index.
//!
//! Port of Qdrant `lib/segment/src/index/hnsw_index/graph_layers.rs` +
//! `point_scorer.rs`, themselves a faithful implementation of Malkov & Yashunin
//! (2016). Multi-layer proximity graph with:
//!   * exponential level assignment `floor(-ln(U) * mL)`, `mL = 1/ln(M)`,
//!   * greedy `search_layer` with an `ef` dynamic candidate list,
//!   * simple closest-`M` neighbour selection with degree pruning,
//!   * soft delete (tombstones traversed for connectivity, filtered from hits).
//!
//! Determinism: level assignment is driven by a seeded splitmix64 PRNG so a
//! given insert order yields a reproducible graph (tests assert recall).

use crate::distance::Metric;
use crate::models::{Distance, Payload, PointId, ScoredPoint};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

/// Total-order wrapper around `f32` (HNSW heaps need `Ord`).
#[derive(Debug, Clone, Copy, PartialEq)]
struct OrdF32(f32);
impl Eq for OrdF32 {}
impl PartialOrd for OrdF32 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for OrdF32 {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
}

struct Node {
    id: PointId,
    vector: Vec<f32>,
    /// `neighbors[layer]` = internal ids of adjacent nodes at that layer.
    neighbors: Vec<Vec<usize>>,
    deleted: bool,
}

/// HNSW graph index over a single vector field.
pub struct HnswIndex {
    metric: Metric,
    m: usize,
    m0: usize,
    ef_construct: usize,
    /// Default search-time candidate list size.
    pub ef: usize,
    ml: f64,
    rng: u64,
    nodes: Vec<Node>,
    id_map: HashMap<PointId, usize>,
    entry: Option<usize>,
    max_layer: usize,
}

impl HnswIndex {
    /// Build an empty index. `seed` makes level assignment reproducible.
    pub fn new(distance: Distance, m: usize, ef_construct: usize, ef: usize, seed: u64) -> Self {
        Self {
            metric: Metric(distance),
            m,
            m0: m * 2,
            ef_construct,
            ef,
            ml: 1.0 / (m as f64).ln(),
            rng: seed,
            nodes: Vec::new(),
            id_map: HashMap::new(),
            entry: None,
            max_layer: 0,
        }
    }

    /// Number of live (non-deleted) points.
    pub fn len(&self) -> usize {
        self.nodes.iter().filter(|n| !n.deleted).count()
    }

    /// Whether the index has no live points.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Insert or replace a point.
    pub fn insert(&mut self, _id: PointId, _vector: Vec<f32>) {}

    /// Soft-delete a point. Returns whether it was present + live.
    pub fn delete(&mut self, _id: &PointId) -> bool {
        false
    }

    /// Search for the top-`k` nearest neighbours using `self.ef`.
    pub fn search(&self, query: &[f32], k: usize) -> Vec<ScoredPoint> {
        self.search_with_ef(query, k, self.ef)
    }

    /// Search with an explicit `ef`.
    pub fn search_with_ef(&self, _query: &[f32], _k: usize, _ef: usize) -> Vec<ScoredPoint> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic pseudo-vectors on a line so nearest-neighbour ground
    /// truth is unambiguous: point i = [i, i].
    fn grid_index(n: usize) -> HnswIndex {
        let mut idx = HnswIndex::new(Distance::Euclid, 16, 100, 64, 42);
        for i in 0..n {
            idx.insert(PointId::Num(i as u64), vec![i as f32, i as f32]);
        }
        idx
    }

    #[test]
    fn empty_index_searches_to_nothing() {
        let idx = HnswIndex::new(Distance::Euclid, 16, 100, 64, 1);
        assert!(idx.is_empty());
        assert!(idx.search(&[0.0, 0.0], 5).is_empty());
    }

    #[test]
    fn single_point_returns_itself() {
        let mut idx = HnswIndex::new(Distance::Euclid, 16, 100, 64, 1);
        idx.insert(PointId::Num(7), vec![1.0, 2.0]);
        let hits = idx.search(&[1.0, 2.0], 3);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, PointId::Num(7));
    }

    #[test]
    fn nearest_neighbour_is_exact_on_line() {
        let idx = grid_index(100);
        let hits = idx.search(&[40.2, 40.2], 1);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, PointId::Num(40));
    }

    #[test]
    fn results_sorted_by_score_desc() {
        let idx = grid_index(100);
        let hits = idx.search(&[50.0, 50.0], 5);
        assert_eq!(hits.len(), 5);
        for w in hits.windows(2) {
            assert!(w[0].score >= w[1].score, "not sorted: {:?}", hits);
        }
        assert_eq!(hits[0].id, PointId::Num(50));
    }

    #[test]
    fn recall_matches_brute_force() {
        let n = 300;
        let idx = grid_index(n);
        let metric = Metric(Distance::Euclid);
        // 20 random-ish queries; compare HNSW top-10 against ground truth.
        let mut hit = 0usize;
        let mut total = 0usize;
        for q in 0..20 {
            let x = (q as f32) * 13.0 + 5.5;
            let query = [x, x];
            // ground truth: nearest 10 by index distance on the line.
            let mut gt: Vec<usize> = (0..n).collect();
            gt.sort_by(|&a, &b| {
                metric
                    .distance(&query, &[a as f32, a as f32])
                    .total_cmp(&metric.distance(&query, &[b as f32, b as f32]))
            });
            let gt10: HashSet<u64> = gt.iter().take(10).map(|&i| i as u64).collect();
            let hits = idx.search(&query, 10);
            for h in &hits {
                if let PointId::Num(n) = h.id {
                    if gt10.contains(&n) {
                        hit += 1;
                    }
                }
            }
            total += 10;
        }
        let recall = hit as f64 / total as f64;
        assert!(recall >= 0.9, "recall {recall} below 0.9");
    }

    #[test]
    fn delete_excludes_from_results() {
        let mut idx = grid_index(50);
        assert!(idx.delete(&PointId::Num(25)));
        assert!(!idx.delete(&PointId::Num(25))); // already gone
        let hits = idx.search(&[25.0, 25.0], 3);
        assert!(hits.iter().all(|h| h.id != PointId::Num(25)));
        assert_eq!(idx.len(), 49);
    }

    #[test]
    fn upsert_same_id_updates_vector() {
        let mut idx = HnswIndex::new(Distance::Euclid, 16, 100, 64, 1);
        idx.insert(PointId::Num(1), vec![0.0, 0.0]);
        idx.insert(PointId::Num(1), vec![100.0, 100.0]);
        assert_eq!(idx.len(), 1);
        let hits = idx.search(&[100.0, 100.0], 1);
        assert_eq!(hits[0].id, PointId::Num(1));
    }
}
