//! Hierarchical Navigable Small World (HNSW) graph index.
//!
//! Implements the algorithm from Malkov & Yashunin 2018:
//! "Efficient and robust approximate nearest neighbor search using
//!  Hierarchical Navigable Small World graphs."
//!
//! # Algorithm overview
//!
//! HNSW builds a multi-layer graph of vector nodes:
//!
//! - **Layer 0** (bottom): contains ALL nodes; each node connected to up to
//!   `m0` nearest neighbours.
//! - **Layers 1…max_layer**: contain a random subset of nodes; each node
//!   connected to up to `m` nearest neighbours.
//! - **Entry point**: the single node that exists at the highest layer.
//!
//! **Insert:**
//!   1. Randomly assign a max layer `l = ⌊−ln(U(0,1)) × mL⌋`.
//!   2. Navigate from the entry point top-down to layer `l+1` using greedy
//!      best-first (single beam).
//!   3. At each layer `l'` from `l` down to 0, do beam search with `ef_construction`
//!      candidates and select the best `m` (or `m0`) to become neighbours.
//!   4. Update the entry point if `l > current_max_layer`.
//!
//! **Search:**
//!   1. Navigate from the entry point greedily down to layer 1.
//!   2. At layer 0, do beam search with `ef` candidates.
//!   3. Return the top-k results by distance.

use std::collections::{BinaryHeap, HashMap, HashSet};

use crate::distance;
use crate::models::{Distance, PointId, Vector};

// ─────────────────────────────────────────────────────────────────────────────
// Internal node
// ─────────────────────────────────────────────────────────────────────────────

/// Storage key for a graph node (stable string representation of PointId).
type NodeKey = String;

/// Per-node graph data.
struct Node {
    point_id: PointId,
    vector: Vector,
    /// Neighbour lists indexed by layer.  `connections[0]` is the layer-0 list.
    connections: Vec<Vec<NodeKey>>,
    /// Highest layer this node participates in.
    max_layer: usize,
}

// ─────────────────────────────────────────────────────────────────────────────
// Priority-queue items (for BinaryHeap)
// ─────────────────────────────────────────────────────────────────────────────

/// Max-heap item (largest distance first — used to maintain the ef-candidate set).
#[derive(PartialEq)]
struct MaxHeapItem {
    distance: f32,
    key: NodeKey,
}

impl Eq for MaxHeapItem {}

impl PartialOrd for MaxHeapItem {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MaxHeapItem {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.distance
            .partial_cmp(&other.distance)
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}

/// Min-heap item (smallest distance first — used for the result queue).
#[derive(PartialEq)]
struct MinHeapItem {
    distance: f32,
    key: NodeKey,
}

impl Eq for MinHeapItem {}

impl PartialOrd for MinHeapItem {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MinHeapItem {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Reverse for min-heap
        other
            .distance
            .partial_cmp(&self.distance)
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// HNSW index
// ─────────────────────────────────────────────────────────────────────────────

/// In-memory HNSW graph for approximate nearest-neighbour search.
pub struct HnswIndex {
    nodes: HashMap<NodeKey, Node>,
    /// Entry point for graph traversal (highest-layer node).
    entry_point: Option<NodeKey>,
    current_max_layer: usize,
    /// Vector dimension.
    dim: usize,
    /// Distance metric.
    distance: Distance,

    // ── Construction parameters ──────────────────────────────────────────
    /// Max connections per node per layer (except layer 0).
    m: usize,
    /// Max connections per node in layer 0 (usually 2×m).
    m0: usize,
    /// Beam width during construction.
    ef_construction: usize,
    /// Level generation multiplier: mL = 1 / ln(m).
    ml: f64,

    // ── Search parameters ────────────────────────────────────────────────
    /// Default beam width during search.
    pub ef: usize,

    /// Monotonic operation counter (used as version / operation_id).
    op_counter: u64,
}

impl HnswIndex {
    /// Create a new empty HNSW index.
    pub fn new(dim: usize, distance: Distance, m: usize, m0: usize, ef_construction: usize, ef: usize) -> Self {
        let ml = 1.0 / (m as f64).ln().max(1e-9);
        Self {
            nodes: HashMap::new(),
            entry_point: None,
            current_max_layer: 0,
            dim,
            distance,
            m,
            m0,
            ef_construction,
            ml,
            ef,
            op_counter: 0,
        }
    }

    pub fn point_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn op_counter(&self) -> u64 {
        self.op_counter
    }

    // ── Insert ───────────────────────────────────────────────────────────

    /// Insert or update a point in the index.
    pub fn upsert(&mut self, id: PointId, vector: Vector, rng: &mut impl rand::Rng) {
        let key = id.as_str_key();

        // Remove old node if present (then re-insert).
        if self.nodes.contains_key(&key) {
            self.remove_node(&key);
        }

        let new_level = self.random_level(rng);

        // Grow the entry point if this is the first node or new_level > current max.
        if self.entry_point.is_none() {
            let connections = (0..=new_level).map(|_| Vec::new()).collect();
            self.nodes.insert(key.clone(), Node {
                point_id: id,
                vector,
                connections,
                max_layer: new_level,
            });
            self.entry_point = Some(key);
            self.current_max_layer = new_level;
            self.op_counter += 1;
            return;
        }

        // ── Phase 1: descend from top layer to new_level + 1 ────────────
        let mut ep_key = self.entry_point.clone().unwrap();
        let ep_dist = self.dist_to_node(&vector, &ep_key);
        let mut ep_dist_current = ep_dist;

        for layer in (new_level + 1..=self.current_max_layer).rev() {
            let changed = self.greedy_search_layer(&vector, &ep_key, layer);
            if let Some((new_ep, new_dist)) = changed {
                if new_dist < ep_dist_current {
                    ep_key = new_ep;
                    ep_dist_current = new_dist;
                }
            }
        }

        // ── Phase 2: insert at each layer from new_level down to 0 ──────
        let connections: Vec<Vec<NodeKey>> = (0..=new_level).map(|_| Vec::new()).collect();
        self.nodes.insert(key.clone(), Node {
            point_id: id,
            vector: vector.clone(),
            connections,
            max_layer: new_level,
        });

        for layer in (0..=new_level.min(self.current_max_layer)).rev() {
            let max_conn = if layer == 0 { self.m0 } else { self.m };

            // Beam search to find ef_construction nearest neighbours.
            let candidates = self.beam_search_layer(&vector, &ep_key, layer, self.ef_construction);

            // Select the best max_conn neighbours.
            let neighbours: Vec<NodeKey> = candidates
                .iter()
                .take(max_conn)
                .map(|(k, _)| k.clone())
                .collect();

            // Update the new node's connection list.
            if let Some(new_node) = self.nodes.get_mut(&key) {
                if layer < new_node.connections.len() {
                    new_node.connections[layer] = neighbours.clone();
                }
            }

            // Add the new node as a neighbour of each selected neighbour
            // and prune their lists if over capacity.
            for nbr_key in &neighbours {
                if nbr_key == &key { continue; }
                self.add_connection(nbr_key, &key, layer, max_conn);
            }

            // Update entry point for next layer descent.
            if let Some((best_key, _)) = candidates.first() {
                ep_key = best_key.clone();
            }
        }

        // Update entry point if new_level > current_max_layer.
        if new_level > self.current_max_layer {
            self.current_max_layer = new_level;
            self.entry_point = Some(key);
        }

        self.op_counter += 1;
    }

    // ── Delete ───────────────────────────────────────────────────────────

    /// Remove a point from the index.  Returns `true` if it existed.
    pub fn remove(&mut self, id: &PointId) -> bool {
        let key = id.as_str_key();
        self.remove_node(&key)
    }

    fn remove_node(&mut self, key: &str) -> bool {
        if !self.nodes.contains_key(key) {
            return false;
        }

        // Remove the node from all its neighbours' connection lists.
        let max_layer = self.nodes[key].max_layer;
        for layer in 0..=max_layer {
            let nbrs: Vec<NodeKey> = self
                .nodes
                .get(key)
                .and_then(|n| n.connections.get(layer))
                .cloned()
                .unwrap_or_default();

            for nbr_key in nbrs {
                if let Some(nbr_node) = self.nodes.get_mut(&nbr_key) {
                    if layer < nbr_node.connections.len() {
                        nbr_node.connections[layer].retain(|k| k != key);
                    }
                }
            }
        }

        self.nodes.remove(key);

        // Reset entry point if we just removed it.
        if self.entry_point.as_deref() == Some(key) {
            self.entry_point = self.nodes.keys().next().cloned();
            self.current_max_layer = self
                .entry_point
                .as_ref()
                .and_then(|ep| self.nodes.get(ep))
                .map(|n| n.max_layer)
                .unwrap_or(0);
        }

        self.op_counter += 1;
        true
    }

    // ── Lookup ───────────────────────────────────────────────────────────

    /// Retrieve the vector and PointId for a node key.
    pub fn get_by_key(&self, key: &str) -> Option<(&PointId, &Vector)> {
        self.nodes.get(key).map(|n| (&n.point_id, &n.vector))
    }

    pub fn contains(&self, id: &PointId) -> bool {
        self.nodes.contains_key(&id.as_str_key())
    }

    /// Iterate over all (PointId, Vector) pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&PointId, &Vector)> {
        self.nodes.values().map(|n| (&n.point_id, &n.vector))
    }

    // ── Nearest-neighbour search ─────────────────────────────────────────

    /// Approximate k-nearest-neighbour search.
    ///
    /// Returns up to `k` (key, distance) pairs sorted by ascending distance.
    pub fn search(&self, query: &[f32], k: usize, ef: Option<usize>) -> Vec<(NodeKey, f32)> {
        if self.nodes.is_empty() {
            return vec![];
        }

        let ef = ef.unwrap_or(self.ef).max(k);
        let ep_key = match &self.entry_point {
            Some(ep) => ep.clone(),
            None => return vec![],
        };

        // Descend layers from max down to 1.
        let mut current_ep = ep_key;
        for layer in (1..=self.current_max_layer).rev() {
            if let Some((better_ep, _)) = self.greedy_search_layer(query, &current_ep, layer) {
                current_ep = better_ep;
            }
        }

        // Full beam search at layer 0.
        let mut candidates = self.beam_search_layer(query, &current_ep, 0, ef);
        candidates.truncate(k);
        candidates
    }

    /// Exact brute-force search (for small collections or testing).
    pub fn search_exact(&self, query: &[f32], k: usize) -> Vec<(NodeKey, f32)> {
        let mut all: Vec<(NodeKey, f32)> = self
            .nodes
            .iter()
            .map(|(key, node)| (key.clone(), self.dist(query, &node.vector)))
            .collect();
        all.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        all.truncate(k);
        all
    }

    // ── Graph traversal helpers ──────────────────────────────────────────

    /// Single greedy step at a given layer: move to the closest neighbour.
    /// Returns `Some((new_ep, dist))` if we moved, `None` otherwise.
    fn greedy_search_layer(
        &self,
        query: &[f32],
        ep_key: &str,
        layer: usize,
    ) -> Option<(NodeKey, f32)> {
        let ep_node = self.nodes.get(ep_key)?;
        let mut best_key = ep_key.to_string();
        let mut best_dist = self.dist(query, &ep_node.vector);
        let mut changed = false;

        let nbrs = ep_node
            .connections
            .get(layer)
            .cloned()
            .unwrap_or_default();

        for nbr_key in nbrs {
            if let Some(nbr_node) = self.nodes.get(&nbr_key) {
                let d = self.dist(query, &nbr_node.vector);
                if d < best_dist {
                    best_dist = d;
                    best_key = nbr_key;
                    changed = true;
                }
            }
        }

        if changed { Some((best_key, best_dist)) } else { None }
    }

    /// Beam search at a given layer with `ef` candidates.
    /// Returns `(key, distance)` pairs sorted by ascending distance.
    fn beam_search_layer(
        &self,
        query: &[f32],
        ep_key: &str,
        layer: usize,
        ef: usize,
    ) -> Vec<(NodeKey, f32)> {
        let ep_node = match self.nodes.get(ep_key) {
            Some(n) => n,
            None => return vec![],
        };

        let ep_dist = self.dist(query, &ep_node.vector);

        // `candidates` is a min-heap (closest first).
        let mut candidates: BinaryHeap<MinHeapItem> = BinaryHeap::new();
        candidates.push(MinHeapItem { distance: ep_dist, key: ep_key.to_string() });

        // `result_set` is a max-heap of size ef (furthest first — for easy trimming).
        let mut result_set: BinaryHeap<MaxHeapItem> = BinaryHeap::new();
        result_set.push(MaxHeapItem { distance: ep_dist, key: ep_key.to_string() });

        let mut visited: HashSet<NodeKey> = HashSet::new();
        visited.insert(ep_key.to_string());

        while let Some(MinHeapItem { distance: c_dist, key: c_key }) = candidates.pop() {
            // If the nearest candidate is further than the furthest result, stop.
            if let Some(worst) = result_set.peek() {
                if c_dist > worst.distance {
                    break;
                }
            }

            let nbrs = self
                .nodes
                .get(&c_key)
                .and_then(|n| n.connections.get(layer))
                .cloned()
                .unwrap_or_default();

            for nbr_key in nbrs {
                if visited.contains(&nbr_key) { continue; }
                visited.insert(nbr_key.clone());

                let nbr_dist = self.nodes.get(&nbr_key)
                    .map(|n| self.dist(query, &n.vector))
                    .unwrap_or(f32::INFINITY);

                let worst_dist = result_set.peek().map(|w| w.distance).unwrap_or(f32::INFINITY);

                if nbr_dist < worst_dist || result_set.len() < ef {
                    candidates.push(MinHeapItem { distance: nbr_dist, key: nbr_key.clone() });
                    result_set.push(MaxHeapItem { distance: nbr_dist, key: nbr_key });
                    if result_set.len() > ef {
                        result_set.pop(); // remove the furthest
                    }
                }
            }
        }

        // Collect and sort by ascending distance.
        let mut results: Vec<(NodeKey, f32)> =
            result_set.into_iter().map(|item| (item.key, item.distance)).collect();
        results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        results
    }

    /// Add `new_nbr` to `node_key`'s connection list at `layer`, pruning to `max_conn`.
    fn add_connection(&mut self, node_key: &str, new_nbr: &str, layer: usize, max_conn: usize) {
        // Phase 1: add the neighbour and capture the snapshot (scoped mutable borrow).
        let (conns_snapshot, node_vector) = {
            let Some(node) = self.nodes.get_mut(node_key) else { return };
            if layer >= node.connections.len() { return; }

            let conns = &mut node.connections[layer];
            if !conns.contains(&new_nbr.to_string()) {
                conns.push(new_nbr.to_string());
            }
            if conns.len() <= max_conn {
                return; // no pruning needed
            }
            (conns.clone(), node.vector.clone())
        }; // mutable borrow released here

        // Phase 2: compute distances (immutable borrow).
        let dist_metric = self.distance;
        let mut with_dist: Vec<(NodeKey, f32)> = conns_snapshot
            .iter()
            .map(|k| {
                let d = self.nodes.get(k)
                    .map(|n| distance::distance(&node_vector, &n.vector, dist_metric))
                    .unwrap_or(f32::INFINITY);
                (k.clone(), d)
            })
            .collect();
        with_dist.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        with_dist.truncate(max_conn);
        let pruned: Vec<NodeKey> = with_dist.into_iter().map(|(k, _)| k).collect();

        // Phase 3: write back pruned list (new mutable borrow).
        if let Some(n) = self.nodes.get_mut(node_key) {
            if layer < n.connections.len() {
                n.connections[layer] = pruned;
            }
        }
    }

    fn dist_to_node(&self, query: &[f32], key: &str) -> f32 {
        self.nodes
            .get(key)
            .map(|n| self.dist(query, &n.vector))
            .unwrap_or(f32::INFINITY)
    }

    #[inline]
    fn dist(&self, a: &[f32], b: &[f32]) -> f32 {
        distance::distance(a, b, self.distance)
    }

    /// Randomly assign a max layer for a new node.
    ///
    /// Level is drawn from a geometric distribution:
    /// `l = ⌊−ln(U(0,1)) × mL⌋`.
    fn random_level(&self, rng: &mut impl rand::Rng) -> usize {
        let u: f64 = rng.sample::<f64, _>(rand::distributions::Standard).max(1e-9);
        let level = (-u.ln() * self.ml).floor() as usize;
        level.min(32) // hard cap to prevent degenerate tall trees
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn seeded_rng() -> impl rand::Rng {
        rand::rngs::StdRng::seed_from_u64(42)
    }

    fn make_index() -> HnswIndex {
        HnswIndex::new(2, Distance::Cosine, 4, 8, 10, 10)
    }

    fn pid(n: u64) -> PointId { PointId::Num(n) }
    fn v2(x: f32, y: f32) -> Vector { vec![x, y] }

    #[test]
    fn insert_and_count() {
        let mut idx = make_index();
        let mut rng = seeded_rng();
        idx.upsert(pid(1), v2(1.0, 0.0), &mut rng);
        idx.upsert(pid(2), v2(0.0, 1.0), &mut rng);
        assert_eq!(idx.point_count(), 2);
    }

    #[test]
    fn contains_after_insert() {
        let mut idx = make_index();
        let mut rng = seeded_rng();
        idx.upsert(pid(1), v2(1.0, 0.0), &mut rng);
        assert!(idx.contains(&pid(1)));
        assert!(!idx.contains(&pid(99)));
    }

    #[test]
    fn remove_decreases_count() {
        let mut idx = make_index();
        let mut rng = seeded_rng();
        idx.upsert(pid(1), v2(1.0, 0.0), &mut rng);
        assert!(idx.remove(&pid(1)));
        assert_eq!(idx.point_count(), 0);
        assert!(!idx.remove(&pid(1)));
    }

    #[test]
    fn search_returns_nearest() {
        let mut idx = make_index();
        let mut rng = seeded_rng();
        idx.upsert(pid(1), v2(1.0, 0.0), &mut rng);
        idx.upsert(pid(2), v2(0.0, 1.0), &mut rng);
        idx.upsert(pid(3), v2(-1.0, 0.0), &mut rng);

        let results = idx.search(&v2(0.9, 0.1), 1, None);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, pid(1).as_str_key());
    }

    #[test]
    fn search_k_larger_than_n() {
        let mut idx = make_index();
        let mut rng = seeded_rng();
        idx.upsert(pid(1), v2(1.0, 0.0), &mut rng);
        let results = idx.search(&v2(1.0, 0.0), 10, None);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn search_empty_index() {
        let idx = make_index();
        let results = idx.search(&v2(1.0, 0.0), 5, None);
        assert!(results.is_empty());
    }

    #[test]
    fn exact_search_same_result_as_approx_small() {
        let mut idx = HnswIndex::new(2, Distance::Euclid, 4, 8, 20, 20);
        let mut rng = seeded_rng();

        // Insert 10 random vectors.
        for i in 0..10u64 {
            let x = (i as f32) * 0.1;
            idx.upsert(PointId::Num(i), vec![x, 1.0 - x], &mut rng);
        }

        let query = vec![0.5, 0.5];
        let approx = idx.search(&query, 3, Some(20));
        let exact = idx.search_exact(&query, 3);

        // Top result should agree.
        assert!(!approx.is_empty());
        assert!(!exact.is_empty());
        assert_eq!(approx[0].0, exact[0].0);
    }

    #[test]
    fn op_counter_increments() {
        let mut idx = make_index();
        let mut rng = seeded_rng();
        assert_eq!(idx.op_counter(), 0);
        idx.upsert(pid(1), v2(1.0, 0.0), &mut rng);
        assert_eq!(idx.op_counter(), 1);
        idx.remove(&pid(1));
        assert_eq!(idx.op_counter(), 2);
    }

    #[test]
    fn upsert_replaces_existing() {
        let mut idx = make_index();
        let mut rng = seeded_rng();
        idx.upsert(pid(1), v2(1.0, 0.0), &mut rng);
        idx.upsert(pid(1), v2(0.0, 1.0), &mut rng);
        assert_eq!(idx.point_count(), 1);
        let (_, vec) = idx.get_by_key(&pid(1).as_str_key()).unwrap();
        assert_eq!(*vec, v2(0.0, 1.0));
    }

    #[test]
    fn large_insert_and_recall() {
        let mut idx = HnswIndex::new(4, Distance::Cosine, 8, 16, 40, 40);
        let mut rng = rand::rngs::StdRng::seed_from_u64(0);

        // Insert 100 vectors.
        for i in 0..100u64 {
            let v: Vec<f32> = (0..4).map(|j| ((i * 7 + j * 13) % 100) as f32 / 100.0).collect();
            idx.upsert(PointId::Num(i), v, &mut rng);
        }

        // Query for the nearest to vector of all 0.5s.
        let query = vec![0.5, 0.5, 0.5, 0.5];
        let approx = idx.search(&query, 5, Some(60));
        let exact = idx.search_exact(&query, 5);

        // Allow that the top result is among the top-5 exact results.
        let exact_top5_keys: Vec<_> = exact.iter().map(|(k, _)| k.as_str()).collect();
        assert!(exact_top5_keys.contains(&approx[0].0.as_str()));
    }
}
