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
    pub fn insert(&mut self, id: PointId, vector: Vec<f32>) {
        // Upsert: if id exists, drop the old node's edges (soft) and rewire.
        if let Some(&existing) = self.id_map.get(&id) {
            self.nodes[existing].deleted = true;
            self.id_map.remove(&id);
        }

        let level = self.random_level();
        let new_id = self.nodes.len();
        self.nodes.push(Node {
            id: id.clone(),
            vector,
            neighbors: vec![Vec::new(); level + 1],
            deleted: false,
        });
        self.id_map.insert(id, new_id);

        let Some(entry) = self.entry else {
            self.entry = Some(new_id);
            self.max_layer = level;
            return;
        };

        let query = self.nodes[new_id].vector.clone();
        let mut ep = entry;

        // Phase 1: greedily descend layers above the new point's top layer.
        let mut lc = self.max_layer;
        while lc > level {
            let w = self.search_layer(&query, &[ep], 1, lc);
            if let Some(&(_, best)) = w.first() {
                ep = best;
            }
            lc -= 1;
        }

        // Phase 2: connect on every layer from min(level, max_layer) down to 0.
        let mut entry_points = vec![ep];
        let top = level.min(self.max_layer);
        for lc in (0..=top).rev() {
            let candidates = self.search_layer(&query, &entry_points, self.ef_construct, lc);
            let m_max = if lc == 0 { self.m0 } else { self.m };
            let selected: Vec<usize> =
                candidates.iter().take(m_max).map(|&(_, n)| n).collect();

            self.nodes[new_id].neighbors[lc] = selected.clone();
            for nb in selected {
                self.nodes[nb].neighbors[lc].push(new_id);
                self.prune(nb, lc, m_max);
            }
            entry_points = candidates.iter().map(|&(_, n)| n).collect();
            if entry_points.is_empty() {
                entry_points = vec![ep];
            }
        }

        if level > self.max_layer {
            self.max_layer = level;
            self.entry = Some(new_id);
        }
    }

    /// Soft-delete a point. Returns whether it was present + live.
    pub fn delete(&mut self, id: &PointId) -> bool {
        if let Some(&internal) = self.id_map.get(id) {
            if !self.nodes[internal].deleted {
                self.nodes[internal].deleted = true;
                self.id_map.remove(id);
                return true;
            }
        }
        false
    }

    /// Search for the top-`k` nearest neighbours using `self.ef`.
    pub fn search(&self, query: &[f32], k: usize) -> Vec<ScoredPoint> {
        self.search_with_ef(query, k, self.ef.max(k))
    }

    /// Search with an explicit `ef`.
    pub fn search_with_ef(&self, query: &[f32], k: usize, ef: usize) -> Vec<ScoredPoint> {
        let Some(entry) = self.entry else {
            return Vec::new();
        };
        // Descend the upper layers greedily (ef=1) to find a good entry point.
        let mut ep = entry;
        let mut lc = self.max_layer;
        while lc > 0 {
            let w = self.search_layer(query, &[ep], 1, lc);
            if let Some(&(_, best)) = w.first() {
                ep = best;
            }
            lc -= 1;
        }
        // Layer 0: full ef search, then take k live results.
        let candidates = self.search_layer(query, &[ep], ef.max(k), 0);
        candidates
            .into_iter()
            .filter(|&(_, n)| !self.nodes[n].deleted)
            .take(k)
            .map(|(_dist, n)| ScoredPoint {
                id: self.nodes[n].id.clone(),
                // recover the unified higher-is-better score from the metric.
                score: self.metric.score(query, &self.nodes[n].vector),
                payload: Payload::new(),
            })
            .collect()
    }

    // ── internals ──────────────────────────────────────────────────────────

    fn dist(&self, query: &[f32], internal: usize) -> f32 {
        self.metric.distance(query, &self.nodes[internal].vector)
    }

    /// splitmix64 → uniform `[0,1)`.
    fn next_unit(&mut self) -> f64 {
        self.rng = self.rng.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
    }

    fn random_level(&mut self) -> usize {
        let r = self.next_unit().max(1e-12);
        (-r.ln() * self.ml).floor() as usize
    }

    /// Greedy `search_layer` (Malkov & Yashunin Algorithm 2). Returns the `ef`
    /// closest nodes at `layer`, sorted ascending by distance.
    fn search_layer(
        &self,
        query: &[f32],
        entry_points: &[usize],
        ef: usize,
        layer: usize,
    ) -> Vec<(f32, usize)> {
        let mut visited: HashSet<usize> = HashSet::new();
        // candidates: min-heap on distance (explore closest first).
        let mut candidates: BinaryHeap<std::cmp::Reverse<(OrdF32, usize)>> = BinaryHeap::new();
        // results: max-heap on distance (peek = current farthest), capped at ef.
        let mut results: BinaryHeap<(OrdF32, usize)> = BinaryHeap::new();

        for &ep in entry_points {
            let d = self.dist(query, ep);
            visited.insert(ep);
            candidates.push(std::cmp::Reverse((OrdF32(d), ep)));
            results.push((OrdF32(d), ep));
        }

        while let Some(std::cmp::Reverse((OrdF32(c_dist), c))) = candidates.pop() {
            let farthest = results.peek().map(|x| x.0 .0).unwrap_or(f32::INFINITY);
            if c_dist > farthest && results.len() >= ef {
                break;
            }
            for &nb in &self.nodes[c].neighbors[layer] {
                if visited.insert(nb) {
                    let d = self.dist(query, nb);
                    let farthest = results.peek().map(|x| x.0 .0).unwrap_or(f32::INFINITY);
                    if d < farthest || results.len() < ef {
                        candidates.push(std::cmp::Reverse((OrdF32(d), nb)));
                        results.push((OrdF32(d), nb));
                        if results.len() > ef {
                            results.pop();
                        }
                    }
                }
            }
        }

        let mut out: Vec<(f32, usize)> =
            results.into_iter().map(|(OrdF32(d), n)| (d, n)).collect();
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        out
    }

    /// Trim node `internal`'s neighbour list at `layer` to the `m` closest.
    fn prune(&mut self, internal: usize, layer: usize, m: usize) {
        if self.nodes[internal].neighbors[layer].len() <= m {
            return;
        }
        let base = self.nodes[internal].vector.clone();
        let mut scored: Vec<(f32, usize)> = self.nodes[internal].neighbors[layer]
            .iter()
            .map(|&nb| (self.metric.distance(&base, &self.nodes[nb].vector), nb))
            .collect();
        scored.sort_by(|a, b| a.0.total_cmp(&b.0));
        scored.truncate(m);
        self.nodes[internal].neighbors[layer] = scored.into_iter().map(|(_, n)| n).collect();
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
