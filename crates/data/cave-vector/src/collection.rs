// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Collection schema + point storage.
//!
//! Port of Qdrant `lib/collection/src/collection/mod.rs` +
//! `lib/segment` point store, reduced to an in-memory map. A [`Collection`]
//! owns its schema ([`VectorParams`]) and a point map; [`CollectionStore`]
//! is the named registry create/get/delete operate against.

use crate::distance::Metric;
use crate::error::VectorError;
use crate::models::{Payload, Point, PointId, ScoredPoint, VectorParams};
use std::collections::BTreeMap;
use std::collections::HashMap;

/// A single collection: schema + point map.
#[derive(Debug, Clone)]
pub struct Collection {
    /// Vector schema (dimension + metric + index/quant config).
    pub params: VectorParams,
    /// Stored points keyed by id (ordered for deterministic iteration).
    pub points: BTreeMap<PointId, Point>,
}

impl Collection {
    /// New empty collection with the given schema.
    pub fn new(params: VectorParams) -> Self {
        Self { params, points: BTreeMap::new() }
    }

    /// Insert or replace a point. Validates vector dimension against schema.
    pub fn upsert(&mut self, point: Point) -> Result<(), VectorError> {
        if point.vector.len() != self.params.size {
            return Err(VectorError::DimensionMismatch {
                expected: self.params.size,
                got: point.vector.len(),
            });
        }
        self.points.insert(point.id.clone(), point);
        Ok(())
    }

    /// Fetch a point by id.
    pub fn get(&self, id: &PointId) -> Option<&Point> {
        self.points.get(id)
    }

    /// Delete a point; returns whether it existed.
    pub fn delete(&mut self, id: &PointId) -> bool {
        self.points.remove(id).is_some()
    }

    /// Number of stored points.
    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// Whether the collection is empty.
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// Exhaustive (brute-force) top-`k` search by metric score.
    pub fn search_bruteforce(&self, query: &[f32], k: usize) -> Vec<ScoredPoint> {
        topk_scored(Metric(self.params.distance), query, self.points.iter(), k)
    }
}

/// Named registry of collections.
#[derive(Debug, Default)]
pub struct CollectionStore {
    cols: HashMap<String, Collection>,
}

impl CollectionStore {
    /// Empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new collection. Errors if the name is taken.
    pub fn create_collection(
        &mut self,
        name: &str,
        params: VectorParams,
    ) -> Result<(), VectorError> {
        if self.cols.contains_key(name) {
            return Err(VectorError::CollectionExists(name.to_string()));
        }
        self.cols.insert(name.to_string(), Collection::new(params));
        Ok(())
    }

    /// Immutable handle to a collection.
    pub fn get(&self, name: &str) -> Option<&Collection> {
        self.cols.get(name)
    }

    /// Mutable handle to a collection.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut Collection> {
        self.cols.get_mut(name)
    }

    /// Delete a collection; returns whether it existed.
    pub fn delete_collection(&mut self, name: &str) -> bool {
        self.cols.remove(name).is_some()
    }

    /// Sorted list of collection names.
    pub fn list(&self) -> Vec<String> {
        let mut names: Vec<String> = self.cols.keys().cloned().collect();
        names.sort();
        names
    }
}

/// Helper: top-k by `(query, point)` metric score, used by brute-force and
/// (later) the HNSW fallback.
pub(crate) fn topk_scored<'a, I>(
    metric: Metric,
    query: &[f32],
    points: I,
    k: usize,
) -> Vec<ScoredPoint>
where
    I: IntoIterator<Item = (&'a PointId, &'a Point)>,
{
    let mut scored: Vec<ScoredPoint> = points
        .into_iter()
        .map(|(id, p)| ScoredPoint {
            id: id.clone(),
            score: metric.score(query, &p.vector),
            payload: Payload::new(),
        })
        .collect();
    scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(k);
    scored
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Distance;

    fn params(dim: usize) -> VectorParams {
        VectorParams { size: dim, distance: Distance::Euclid, hnsw_config: None, quantization: None }
    }

    fn pt(id: u64, v: &[f32]) -> Point {
        Point { id: PointId::Num(id), vector: v.to_vec(), payload: Payload::new() }
    }

    #[test]
    fn create_and_list_collections() {
        let mut s = CollectionStore::new();
        s.create_collection("docs", params(3)).unwrap();
        s.create_collection("imgs", params(512)).unwrap();
        assert_eq!(s.list(), vec!["docs".to_string(), "imgs".to_string()]);
    }

    #[test]
    fn create_duplicate_errors() {
        let mut s = CollectionStore::new();
        s.create_collection("docs", params(3)).unwrap();
        let err = s.create_collection("docs", params(3)).unwrap_err();
        assert_eq!(err, VectorError::CollectionExists("docs".into()));
    }

    #[test]
    fn delete_collection_round_trip() {
        let mut s = CollectionStore::new();
        s.create_collection("docs", params(3)).unwrap();
        assert!(s.delete_collection("docs"));
        assert!(!s.delete_collection("docs"));
        assert!(s.get("docs").is_none());
    }

    #[test]
    fn upsert_and_get_point() {
        let mut c = Collection::new(params(3));
        c.upsert(pt(1, &[1.0, 2.0, 3.0])).unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c.get(&PointId::Num(1)).unwrap().vector, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn upsert_replaces_same_id() {
        let mut c = Collection::new(params(3));
        c.upsert(pt(1, &[1.0, 2.0, 3.0])).unwrap();
        c.upsert(pt(1, &[9.0, 9.0, 9.0])).unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c.get(&PointId::Num(1)).unwrap().vector, vec![9.0, 9.0, 9.0]);
    }

    #[test]
    fn upsert_dimension_mismatch_errors() {
        let mut c = Collection::new(params(3));
        let err = c.upsert(pt(1, &[1.0, 2.0])).unwrap_err();
        assert_eq!(err, VectorError::DimensionMismatch { expected: 3, got: 2 });
    }

    #[test]
    fn delete_point_round_trip() {
        let mut c = Collection::new(params(2));
        c.upsert(pt(1, &[1.0, 0.0])).unwrap();
        assert!(c.delete(&PointId::Num(1)));
        assert!(!c.delete(&PointId::Num(1)));
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn brute_force_returns_nearest_first() {
        let mut c = Collection::new(params(2));
        c.upsert(pt(1, &[0.0, 0.0])).unwrap();
        c.upsert(pt(2, &[10.0, 10.0])).unwrap();
        c.upsert(pt(3, &[1.0, 1.0])).unwrap();
        let hits = c.search_bruteforce(&[0.0, 0.0], 2);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].id, PointId::Num(1)); // exact match, distance 0
        assert_eq!(hits[1].id, PointId::Num(3)); // closest non-self
    }
}
