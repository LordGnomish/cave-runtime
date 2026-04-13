//! Vector search engine — collection management, point CRUD, payload indexing.
//!
//! Each collection owns an `HnswIndex` for approximate nearest-neighbour
//! search and a payload store for metadata filtering.  The entire state is
//! protected by a `parking_lot::RwLock` for safe concurrent access.

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use rand::SeedableRng;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, warn};

use crate::distance;
use crate::hnsw::HnswIndex;
use crate::models::{
    CollectionConfig, CollectionInfo, CollectionStatus, Condition, Distance, FieldCondition,
    FieldIndexSchema, Filter, MatchCondition, PayloadFieldSchema, Point, PointId,
    RangeCondition, RecommendRequest, ScrollResponse, SearchRequest, ScoredPoint, SetPayloadRequest,
    UpdateResult, UpdateStatus, UpsertPointsRequest, Vector,
};
use crate::VectorError;

// ─────────────────────────────────────────────────────────────────────────────
// Collection data
// ─────────────────────────────────────────────────────────────────────────────

/// All mutable state for a single vector collection.
pub struct CollectionData {
    pub name: String,
    pub config: CollectionConfig,
    /// HNSW graph index.
    pub hnsw: HnswIndex,
    /// Full point store: key → Point (payload + vector).
    pub points: HashMap<String, Point>,
    /// Payload field indexes: field_name → (value_string → set of point keys).
    pub payload_index: HashMap<String, HashMap<String, Vec<String>>>,
    /// Indexed payload field schemas.
    pub field_schemas: HashMap<String, PayloadFieldSchema>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl CollectionData {
    pub fn new(name: impl Into<String>, config: CollectionConfig) -> Self {
        let hnsw = HnswIndex::new(
            config.vectors.size,
            config.vectors.distance,
            config.hnsw_config.m,
            config.hnsw_config.m0,
            config.hnsw_config.ef_construction,
            config.hnsw_config.ef,
        );
        Self {
            name: name.into(),
            config,
            hnsw,
            points: HashMap::new(),
            payload_index: HashMap::new(),
            field_schemas: HashMap::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    pub fn point_count(&self) -> u64 {
        self.points.len() as u64
    }

    // ── Point CRUD ───────────────────────────────────────────────────────

    pub fn upsert_point(&mut self, point: Point, rng: &mut impl rand::Rng) -> Result<(), VectorError> {
        let dim = self.config.vectors.size;
        if point.vector.len() != dim {
            return Err(VectorError::DimensionMismatch {
                expected: dim,
                got: point.vector.len(),
            });
        }

        let key = point.id.as_str_key();

        // Remove old payload index entries.
        if let Some(old) = self.points.get(&key) {
            self.remove_from_payload_index(&old.clone());
        }

        // Insert into HNSW.
        self.hnsw.upsert(point.id.clone(), point.vector.clone(), rng);

        // Add new payload index entries.
        self.add_to_payload_index(&point);

        self.points.insert(key, point);
        self.updated_at = Utc::now();
        Ok(())
    }

    pub fn delete_point(&mut self, id: &PointId) -> bool {
        let key = id.as_str_key();
        if let Some(point) = self.points.remove(&key) {
            self.remove_from_payload_index(&point);
            self.hnsw.remove(id);
            self.updated_at = Utc::now();
            true
        } else {
            false
        }
    }

    pub fn get_point(&self, id: &PointId) -> Option<&Point> {
        self.points.get(&id.as_str_key())
    }

    /// Apply payload overwrite to selected points.
    pub fn set_payload(&mut self, req: SetPayloadRequest) {
        let keys: Vec<String> = if let Some(ids) = req.points {
            ids.iter().map(|id| id.as_str_key()).collect()
        } else {
            self.points.keys().cloned().collect()
        };

        for key in keys {
            // Phase 1: clone old state so we can call &mut self methods.
            let Some(old_point) = self.points.get(&key).cloned() else { continue };
            // Phase 2: remove stale payload-index entries (no points borrow).
            self.remove_from_payload_index(&old_point);
            // Phase 3: update the stored point.
            if let Some(point) = self.points.get_mut(&key) {
                for (k, v) in &req.payload {
                    point.payload.insert(k.clone(), v.clone());
                }
            }
            // Phase 4: add updated entries (clone again so points borrow is released).
            let Some(new_point) = self.points.get(&key).cloned() else { continue };
            self.add_to_payload_index(&new_point);
        }
        self.updated_at = Utc::now();
    }

    /// Remove all points matched by a filter.
    pub fn delete_by_filter(&mut self, filter: &Filter) -> usize {
        let to_delete: Vec<PointId> = self
            .points
            .values()
            .filter(|p| matches_filter(p, filter))
            .map(|p| p.id.clone())
            .collect();

        let count = to_delete.len();
        for id in to_delete {
            self.delete_point(&id);
        }
        count
    }

    // ── Payload index management ─────────────────────────────────────────

    fn add_to_payload_index(&mut self, point: &Point) {
        let key = point.id.as_str_key();
        for (field, value) in &point.payload {
            if let Some(val_str) = value_to_index_key(value) {
                self.payload_index
                    .entry(field.clone())
                    .or_default()
                    .entry(val_str)
                    .or_default()
                    .push(key.clone());
            }
        }
    }

    fn remove_from_payload_index(&mut self, point: &Point) {
        let key = point.id.as_str_key();
        for (field, value) in &point.payload {
            if let Some(val_str) = value_to_index_key(value) {
                if let Some(field_idx) = self.payload_index.get_mut(field) {
                    if let Some(keys) = field_idx.get_mut(&val_str) {
                        keys.retain(|k| k != &key);
                    }
                }
            }
        }
    }

    pub fn create_field_index(&mut self, field: &str, schema: FieldIndexSchema) {
        let type_str = match schema {
            FieldIndexSchema::Keyword => "keyword",
            FieldIndexSchema::Integer => "integer",
            FieldIndexSchema::Float => "float",
            FieldIndexSchema::Geo => "geo",
            FieldIndexSchema::Text => "text",
            FieldIndexSchema::Bool => "bool",
            FieldIndexSchema::Datetime => "datetime",
        };

        let count = self
            .points
            .values()
            .filter(|p| p.payload.contains_key(field))
            .count() as u64;

        self.field_schemas.insert(field.to_string(), PayloadFieldSchema {
            data_type: type_str.to_string(),
            params: None,
            points: count,
        });

        // Build the index from existing points.
        let points_snapshot: Vec<Point> = self.points.values().cloned().collect();
        for point in points_snapshot {
            self.add_to_payload_index(&point);
        }
        self.updated_at = Utc::now();
    }

    // ── Search ───────────────────────────────────────────────────────────

    pub fn search(&self, req: &SearchRequest) -> Result<Vec<ScoredPoint>, VectorError> {
        if req.vector.len() != self.config.vectors.size {
            return Err(VectorError::DimensionMismatch {
                expected: self.config.vectors.size,
                got: req.vector.len(),
            });
        }

        let ef = req.params.as_ref().and_then(|p| p.hnsw_ef);
        let use_exact = req.params.as_ref().map(|p| p.exact).unwrap_or(false);

        // HNSW search or exact search.
        let candidates = if use_exact || self.hnsw.point_count() <= 50 {
            self.hnsw.search_exact(&req.vector, req.limit.max(self.hnsw.point_count()))
        } else {
            self.hnsw.search(&req.vector, (req.limit + req.offset) * 2, ef)
        };

        let metric = self.config.vectors.distance;
        let mut results: Vec<ScoredPoint> = candidates
            .into_iter()
            .filter_map(|(key, dist)| {
                let point = self.points.get(&key)?;
                let score = distance::distance_to_score(dist, metric);

                // Apply score threshold.
                if let Some(threshold) = req.score_threshold {
                    if score < threshold { return None; }
                }

                // Apply payload filter.
                if let Some(filter) = &req.filter {
                    if !matches_filter(point, filter) { return None; }
                }

                Some(ScoredPoint {
                    id: point.id.clone(),
                    score,
                    payload: if req.with_payload { point.payload.clone() } else { HashMap::new() },
                    vector: if req.with_vectors { Some(point.vector.clone()) } else { None },
                    version: point.version,
                })
            })
            .skip(req.offset)
            .take(req.limit)
            .collect();

        // Sort by score descending.
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        Ok(results)
    }

    /// Recommend: find nearest neighbours by averaging positive − negative examples.
    pub fn recommend(&self, req: &RecommendRequest) -> Result<Vec<ScoredPoint>, VectorError> {
        let dim = self.config.vectors.size;

        let positive_vecs: Vec<&Vector> = req
            .positive
            .iter()
            .filter_map(|id| self.points.get(&id.as_str_key()).map(|p| &p.vector))
            .collect();

        let negative_vecs: Vec<&Vector> = req
            .negative
            .iter()
            .filter_map(|id| self.points.get(&id.as_str_key()).map(|p| &p.vector))
            .collect();

        if positive_vecs.is_empty() {
            return Err(VectorError::InvalidRequest("no valid positive example IDs".into()));
        }

        // Build mean vector of positives.
        let mut query = vec![0.0f32; dim];
        for v in &positive_vecs {
            for (i, x) in v.iter().enumerate() {
                query[i] += x;
            }
        }
        let pos_count = positive_vecs.len() as f32;
        for x in &mut query { *x /= pos_count; }

        // Subtract mean of negatives.
        if !negative_vecs.is_empty() {
            let neg_count = negative_vecs.len() as f32;
            let mut neg_mean = vec![0.0f32; dim];
            for v in &negative_vecs {
                for (i, x) in v.iter().enumerate() {
                    neg_mean[i] += x;
                }
            }
            for (i, x) in neg_mean.iter().enumerate() {
                query[i] -= x / neg_count;
            }
        }

        // Normalise the query vector.
        let norm: f32 = query.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut query { *x /= norm; }
        }

        // Exclude example points from results.
        let excluded: std::collections::HashSet<String> = req
            .positive
            .iter()
            .chain(req.negative.iter())
            .map(|id| id.as_str_key())
            .collect();

        let search_req = SearchRequest {
            vector: query,
            limit: req.limit + excluded.len(),
            filter: req.filter.clone(),
            with_payload: req.with_payload,
            with_vectors: req.with_vectors,
            score_threshold: req.score_threshold,
            ..Default::default()
        };

        let mut results = self.search(&search_req)?;
        results.retain(|r| !excluded.contains(&r.id.as_str_key()));
        results.truncate(req.limit);
        Ok(results)
    }

    /// Scroll through all points, optionally filtered.
    pub fn scroll(
        &self,
        filter: Option<&Filter>,
        limit: usize,
        offset_id: Option<&PointId>,
        with_payload: bool,
        with_vectors: bool,
    ) -> ScrollResponse {
        let offset_key = offset_id.map(|id| id.as_str_key());

        let mut all_keys: Vec<&String> = self.points.keys().collect();
        all_keys.sort(); // deterministic order for cursor-based pagination

        let start_pos = if let Some(ref offset) = offset_key {
            all_keys.iter().position(|k| *k == offset).map(|p| p + 1).unwrap_or(0)
        } else {
            0
        };

        let selected: Vec<Point> = all_keys
            .iter()
            .skip(start_pos)
            .filter_map(|key| {
                let point = self.points.get(*key)?;
                if let Some(f) = filter {
                    if !matches_filter(point, f) { return None; }
                }
                let mut p = point.clone();
                if !with_payload { p.payload.clear(); }
                if !with_vectors { p.vector.clear(); }
                Some(p)
            })
            .take(limit)
            .collect();

        let next_page_offset = if selected.len() == limit {
            selected.last().map(|p| p.id.clone())
        } else {
            None
        };

        ScrollResponse { points: selected, next_page_offset }
    }

    pub fn collection_info(&self) -> CollectionInfo {
        CollectionInfo {
            status: CollectionStatus::Green,
            optimizer_status: "ok".into(),
            vectors_count: self.point_count(),
            indexed_vectors_count: self.point_count(),
            points_count: self.point_count(),
            segments_count: 1,
            config: self.config.clone(),
            payload_schema: self.field_schemas.clone(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// BuiltinVectorStore
// ─────────────────────────────────────────────────────────────────────────────

/// In-memory vector store backed by HNSW indices.
pub struct BuiltinVectorStore {
    collections: RwLock<HashMap<String, CollectionData>>,
    rng: RwLock<rand::rngs::StdRng>,
}

impl BuiltinVectorStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            collections: RwLock::new(HashMap::new()),
            rng: RwLock::new(rand::rngs::StdRng::from_entropy()),
        })
    }

    // ── Collection lifecycle ─────────────────────────────────────────────

    pub fn create_collection(
        &self,
        name: &str,
        config: CollectionConfig,
    ) -> Result<(), VectorError> {
        let mut collections = self.collections.write();
        if collections.contains_key(name) {
            return Err(VectorError::CollectionAlreadyExists(name.to_string()));
        }
        collections.insert(name.to_string(), CollectionData::new(name, config));
        Ok(())
    }

    pub fn delete_collection(&self, name: &str) -> Result<(), VectorError> {
        let mut collections = self.collections.write();
        collections.remove(name).ok_or_else(|| VectorError::CollectionNotFound(name.to_string()))?;
        Ok(())
    }

    pub fn collection_exists(&self, name: &str) -> bool {
        self.collections.read().contains_key(name)
    }

    pub fn list_collections(&self) -> Vec<String> {
        self.collections.read().keys().cloned().collect()
    }

    pub fn collection_info(&self, name: &str) -> Result<CollectionInfo, VectorError> {
        let collections = self.collections.read();
        let col = collections.get(name).ok_or_else(|| VectorError::CollectionNotFound(name.to_string()))?;
        Ok(col.collection_info())
    }

    // ── Points ───────────────────────────────────────────────────────────

    pub fn upsert_points(
        &self,
        collection: &str,
        points: Vec<Point>,
    ) -> Result<UpdateResult, VectorError> {
        let mut collections = self.collections.write();
        let col = collections.get_mut(collection)
            .ok_or_else(|| VectorError::CollectionNotFound(collection.to_string()))?;

        let mut rng = self.rng.write();

        for point in points {
            col.upsert_point(point, &mut *rng)?;
        }

        let op_id = col.hnsw.op_counter();
        Ok(UpdateResult { operation_id: op_id, status: UpdateStatus::Completed })
    }

    pub fn get_point(&self, collection: &str, id: &PointId) -> Result<Option<Point>, VectorError> {
        let collections = self.collections.read();
        let col = collections.get(collection)
            .ok_or_else(|| VectorError::CollectionNotFound(collection.to_string()))?;
        Ok(col.get_point(id).cloned())
    }

    pub fn get_points(
        &self,
        collection: &str,
        ids: &[PointId],
        with_payload: bool,
        with_vectors: bool,
    ) -> Result<Vec<Point>, VectorError> {
        let collections = self.collections.read();
        let col = collections.get(collection)
            .ok_or_else(|| VectorError::CollectionNotFound(collection.to_string()))?;

        let points = ids
            .iter()
            .filter_map(|id| {
                let mut p = col.get_point(id)?.clone();
                if !with_payload { p.payload.clear(); }
                if !with_vectors { p.vector.clear(); }
                Some(p)
            })
            .collect();
        Ok(points)
    }

    pub fn delete_points(
        &self,
        collection: &str,
        ids: Vec<PointId>,
        filter: Option<Filter>,
    ) -> Result<UpdateResult, VectorError> {
        let mut collections = self.collections.write();
        let col = collections.get_mut(collection)
            .ok_or_else(|| VectorError::CollectionNotFound(collection.to_string()))?;

        for id in ids {
            col.delete_point(&id);
        }

        if let Some(f) = filter {
            col.delete_by_filter(&f);
        }

        let op_id = col.hnsw.op_counter();
        Ok(UpdateResult { operation_id: op_id, status: UpdateStatus::Completed })
    }

    pub fn set_payload(
        &self,
        collection: &str,
        req: SetPayloadRequest,
    ) -> Result<UpdateResult, VectorError> {
        let mut collections = self.collections.write();
        let col = collections.get_mut(collection)
            .ok_or_else(|| VectorError::CollectionNotFound(collection.to_string()))?;
        col.set_payload(req);
        let op_id = col.hnsw.op_counter();
        Ok(UpdateResult { operation_id: op_id, status: UpdateStatus::Completed })
    }

    pub fn search(
        &self,
        collection: &str,
        req: SearchRequest,
    ) -> Result<Vec<ScoredPoint>, VectorError> {
        let collections = self.collections.read();
        let col = collections.get(collection)
            .ok_or_else(|| VectorError::CollectionNotFound(collection.to_string()))?;
        col.search(&req)
    }

    pub fn recommend(
        &self,
        collection: &str,
        req: RecommendRequest,
    ) -> Result<Vec<ScoredPoint>, VectorError> {
        let collections = self.collections.read();
        let col = collections.get(collection)
            .ok_or_else(|| VectorError::CollectionNotFound(collection.to_string()))?;
        col.recommend(&req)
    }

    pub fn scroll(
        &self,
        collection: &str,
        filter: Option<Filter>,
        limit: usize,
        offset_id: Option<PointId>,
        with_payload: bool,
        with_vectors: bool,
    ) -> Result<ScrollResponse, VectorError> {
        let collections = self.collections.read();
        let col = collections.get(collection)
            .ok_or_else(|| VectorError::CollectionNotFound(collection.to_string()))?;
        Ok(col.scroll(filter.as_ref(), limit, offset_id.as_ref(), with_payload, with_vectors))
    }

    pub fn create_field_index(
        &self,
        collection: &str,
        field: &str,
        schema: FieldIndexSchema,
    ) -> Result<UpdateResult, VectorError> {
        let mut collections = self.collections.write();
        let col = collections.get_mut(collection)
            .ok_or_else(|| VectorError::CollectionNotFound(collection.to_string()))?;
        col.create_field_index(field, schema);
        let op_id = col.hnsw.op_counter();
        Ok(UpdateResult { operation_id: op_id, status: UpdateStatus::Completed })
    }
}

impl Default for BuiltinVectorStore {
    fn default() -> Self {
        Self {
            collections: RwLock::new(HashMap::new()),
            rng: RwLock::new(rand::rngs::StdRng::from_entropy()),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Filter evaluation
// ─────────────────────────────────────────────────────────────────────────────

/// Returns `true` if the point satisfies all conditions in the filter.
pub fn matches_filter(point: &Point, filter: &Filter) -> bool {
    // must: all must match.
    for cond in &filter.must {
        if !matches_condition(point, cond) { return false; }
    }
    // must_not: none must match.
    for cond in &filter.must_not {
        if matches_condition(point, cond) { return false; }
    }
    // should: at least one must match (if any are specified).
    if !filter.should.is_empty() {
        let any = filter.should.iter().any(|c| matches_condition(point, c));
        if !any { return false; }
    }
    true
}

fn matches_condition(point: &Point, cond: &Condition) -> bool {
    match cond {
        Condition::Field(fc) => matches_field_condition(point, fc),
        Condition::Nested(f) => matches_filter(point, f),
        Condition::IsEmpty(e) => {
            point.payload.get(&e.is_empty.key).map_or(true, |v| v.is_null() || v == &Value::Array(vec![]))
        }
        Condition::IsNull(n) => {
            point.payload.get(&n.is_null.key).map_or(true, |v| v.is_null())
        }
        Condition::HasId(h) => h.has_id.contains(&point.id),
    }
}

fn matches_field_condition(point: &Point, fc: &FieldCondition) -> bool {
    let field_val = match point.payload.get(&fc.key) {
        Some(v) => v,
        None => return false,
    };

    // Match condition.
    if let Some(match_cond) = &fc.r#match {
        let passes = match match_cond {
            MatchCondition::Value(mv) => values_equal(field_val, &mv.value),
            MatchCondition::Any(ma) => ma.any.iter().any(|v| values_equal(field_val, v)),
            MatchCondition::Except(me) => !me.except.iter().any(|v| values_equal(field_val, v)),
        };
        if !passes { return false; }
    }

    // Range condition.
    if let Some(range) = &fc.range {
        let Some(num) = field_val.as_f64() else { return false };
        let passes = range.gte.map_or(true, |v| num >= v)
            && range.gt.map_or(true, |v| num > v)
            && range.lte.map_or(true, |v| num <= v)
            && range.lt.map_or(true, |v| num < v);
        if !passes { return false; }
    }

    true
}

fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(sa), Value::String(sb)) => sa == sb,
        (Value::Number(na), Value::Number(nb)) => {
            na.as_f64().unwrap_or(f64::NAN) == nb.as_f64().unwrap_or(f64::NAN)
        }
        (Value::Bool(ba), Value::Bool(bb)) => ba == bb,
        (Value::Null, Value::Null) => true,
        _ => false,
    }
}

fn value_to_index_key(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_store() -> Arc<BuiltinVectorStore> {
        BuiltinVectorStore::new()
    }

    fn make_cfg(dim: usize) -> CollectionConfig {
        CollectionConfig::new(dim, Distance::Cosine)
    }

    fn point(id: u64, v: Vec<f32>) -> Point {
        Point::new(PointId::Num(id), v, HashMap::new())
    }

    fn point_with_payload(id: u64, v: Vec<f32>, payload: Vec<(&str, Value)>) -> Point {
        let payload_map = payload.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        Point::new(PointId::Num(id), v, payload_map)
    }

    #[test]
    fn create_and_delete_collection() {
        let store = make_store();
        store.create_collection("test", make_cfg(3)).unwrap();
        assert!(store.collection_exists("test"));
        store.delete_collection("test").unwrap();
        assert!(!store.collection_exists("test"));
    }

    #[test]
    fn create_duplicate_fails() {
        let store = make_store();
        store.create_collection("test", make_cfg(3)).unwrap();
        let result = store.create_collection("test", make_cfg(3));
        assert!(matches!(result, Err(VectorError::CollectionAlreadyExists(_))));
    }

    #[test]
    fn upsert_and_get_point() {
        let store = make_store();
        store.create_collection("vecs", make_cfg(3)).unwrap();
        let p = point(1, vec![1.0, 0.0, 0.0]);
        store.upsert_points("vecs", vec![p]).unwrap();
        let got = store.get_point("vecs", &PointId::Num(1)).unwrap();
        assert!(got.is_some());
    }

    #[test]
    fn delete_point() {
        let store = make_store();
        store.create_collection("vecs", make_cfg(3)).unwrap();
        store.upsert_points("vecs", vec![point(1, vec![1.0, 0.0, 0.0])]).unwrap();
        let result = store.delete_points("vecs", vec![PointId::Num(1)], None).unwrap();
        assert_eq!(result.status, UpdateStatus::Completed);
        assert!(store.get_point("vecs", &PointId::Num(1)).unwrap().is_none());
    }

    #[test]
    fn search_returns_nearest() {
        let store = make_store();
        store.create_collection("vecs", make_cfg(2)).unwrap();
        store.upsert_points("vecs", vec![
            point(1, vec![1.0, 0.0]),
            point(2, vec![0.0, 1.0]),
            point(3, vec![-1.0, 0.0]),
        ]).unwrap();

        let req = SearchRequest {
            vector: vec![0.9, 0.1],
            limit: 1,
            ..Default::default()
        };
        let results = store.search("vecs", req).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, PointId::Num(1));
    }

    #[test]
    fn dimension_mismatch_error() {
        let store = make_store();
        store.create_collection("vecs", make_cfg(3)).unwrap();
        let p = point(1, vec![1.0, 0.0]); // wrong dim
        let result = store.upsert_points("vecs", vec![p]);
        assert!(matches!(result, Err(VectorError::DimensionMismatch { .. })));
    }

    #[test]
    fn search_with_payload_filter() {
        let store = make_store();
        store.create_collection("vecs", make_cfg(2)).unwrap();
        store.upsert_points("vecs", vec![
            point_with_payload(1, vec![1.0, 0.0], vec![("category", json!("tech"))]),
            point_with_payload(2, vec![1.0, 0.0], vec![("category", json!("news"))]),
        ]).unwrap();

        let req = SearchRequest {
            vector: vec![1.0, 0.0],
            limit: 10,
            filter: Some(Filter {
                must: vec![Condition::Field(FieldCondition {
                    key: "category".into(),
                    r#match: Some(MatchCondition::Value(crate::models::MatchValue {
                        value: json!("tech"),
                    })),
                    range: None,
                    geo_bounding_box: None,
                    geo_radius: None,
                    values_count: None,
                })],
                should: vec![],
                must_not: vec![],
            }),
            ..Default::default()
        };
        let results = store.search("vecs", req).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, PointId::Num(1));
    }

    #[test]
    fn scroll_all_points() {
        let store = make_store();
        store.create_collection("vecs", make_cfg(2)).unwrap();
        for i in 1u64..=5 {
            store.upsert_points("vecs", vec![point(i, vec![i as f32, 0.0])]).unwrap();
        }
        let resp = store.scroll("vecs", None, 10, None, true, false).unwrap();
        assert_eq!(resp.points.len(), 5);
    }

    #[test]
    fn matches_filter_must_all() {
        let mut p = Point::new(PointId::Num(1), vec![1.0], HashMap::new());
        p.payload.insert("a".into(), json!(1));
        p.payload.insert("b".into(), json!("x"));

        let filter = Filter {
            must: vec![
                Condition::Field(FieldCondition {
                    key: "a".into(),
                    r#match: Some(MatchCondition::Value(crate::models::MatchValue { value: json!(1) })),
                    range: None, geo_bounding_box: None, geo_radius: None, values_count: None,
                }),
            ],
            should: vec![],
            must_not: vec![],
        };
        assert!(matches_filter(&p, &filter));
    }
}
