//! Domain models for cave-vector-search.
//!
//! Wire-compatible with the Qdrant REST API so that existing Qdrant client
//! libraries work without modification.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────────────
// Vector and point identifiers
// ─────────────────────────────────────────────────────────────────────────────

/// A point ID can be either a 64-bit unsigned integer or a UUID string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PointId {
    Num(u64),
    Uuid(String),
}

impl PointId {
    pub fn uuid() -> Self {
        Self::Uuid(Uuid::new_v4().to_string())
    }

    pub fn as_str_key(&self) -> String {
        match self {
            Self::Num(n) => n.to_string(),
            Self::Uuid(s) => s.clone(),
        }
    }
}

impl std::fmt::Display for PointId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Num(n) => write!(f, "{}", n),
            Self::Uuid(s) => write!(f, "{}", s),
        }
    }
}

/// A dense vector (list of f32 components).
pub type Vector = Vec<f32>;

// ─────────────────────────────────────────────────────────────────────────────
// Collection configuration
// ─────────────────────────────────────────────────────────────────────────────

/// Distance metric for nearest-neighbour search.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub enum Distance {
    /// 1 − cosine_similarity (smaller = more similar).
    #[default]
    Cosine,
    /// Euclidean (L2) distance.
    Euclid,
    /// Negative dot product (smaller = more similar).
    Dot,
    /// Manhattan (L1) distance.
    Manhattan,
}

/// Parameters for the vector space of a collection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorParams {
    /// Number of dimensions.
    pub size: usize,
    /// Distance metric used for similarity computation.
    pub distance: Distance,
    /// Whether to store vectors on disk (informational; we always use RAM).
    #[serde(default)]
    pub on_disk: bool,
}

/// HNSW graph construction and search parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HnswConfig {
    /// Number of edges per node in all layers except layer 0.
    #[serde(default = "default_m")]
    pub m: usize,
    /// Number of edges per node in layer 0 (usually 2×m).
    #[serde(default = "default_m0")]
    pub m0: usize,
    /// Beam width during index construction (higher = better quality, slower build).
    #[serde(default = "default_ef_construction")]
    pub ef_construction: usize,
    /// Beam width during search (higher = better recall, slower search).
    #[serde(default = "default_ef")]
    pub ef: usize,
}

fn default_m() -> usize { 16 }
fn default_m0() -> usize { 32 }
fn default_ef_construction() -> usize { 100 }
fn default_ef() -> usize { 64 }

impl Default for HnswConfig {
    fn default() -> Self {
        Self { m: 16, m0: 32, ef_construction: 100, ef: 64 }
    }
}

/// Quantisation configuration (informational; full precision always stored).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QuantisationConfig {
    pub scalar: Option<ScalarQuantisationConfig>,
    pub product: Option<ProductQuantisationConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScalarQuantisationConfig {
    pub r#type: String,
    #[serde(default)]
    pub quantile: Option<f32>,
    #[serde(default)]
    pub always_ram: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductQuantisationConfig {
    pub compression: String,
    #[serde(default)]
    pub always_ram: bool,
}

/// Full configuration for creating a collection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionConfig {
    /// Vector space parameters.
    pub vectors: VectorParams,
    /// HNSW graph construction parameters.
    #[serde(default)]
    pub hnsw_config: HnswConfig,
    /// Maximum number of points before triggering optimisation.
    #[serde(default = "default_max_points")]
    pub max_points: usize,
    /// Quantisation configuration.
    #[serde(default)]
    pub quantisation_config: QuantisationConfig,
    /// Whether to replicate to other nodes (informational).
    #[serde(default = "default_replication_factor")]
    pub replication_factor: u32,
    /// Write consistency factor.
    #[serde(default = "default_write_consistency")]
    pub write_consistency_factor: u32,
}

fn default_max_points() -> usize { 1_000_000 }
fn default_replication_factor() -> u32 { 1 }
fn default_write_consistency() -> u32 { 1 }

impl CollectionConfig {
    pub fn new(size: usize, distance: Distance) -> Self {
        Self {
            vectors: VectorParams { size, distance, on_disk: false },
            hnsw_config: HnswConfig::default(),
            max_points: default_max_points(),
            quantisation_config: QuantisationConfig::default(),
            replication_factor: 1,
            write_consistency_factor: 1,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Collection info
// ─────────────────────────────────────────────────────────────────────────────

/// Status of a collection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum CollectionStatus {
    Green,
    Yellow,
    Red,
}

/// Metadata returned when describing a collection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionInfo {
    pub status: CollectionStatus,
    pub optimizer_status: String,
    pub vectors_count: u64,
    pub indexed_vectors_count: u64,
    pub points_count: u64,
    pub segments_count: u32,
    pub config: CollectionConfig,
    pub payload_schema: HashMap<String, PayloadFieldSchema>,
}

/// Schema description for a payload field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayloadFieldSchema {
    pub data_type: String,
    pub params: Option<Value>,
    pub points: u64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Points
// ─────────────────────────────────────────────────────────────────────────────

/// A vector point stored in a collection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Point {
    /// Unique point identifier.
    pub id: PointId,
    /// Dense vector embedding.
    pub vector: Vector,
    /// Arbitrary JSON payload attached to the point.
    #[serde(default)]
    pub payload: HashMap<String, Value>,
    /// Wall-clock time of last upsert.
    #[serde(default = "Utc::now")]
    pub created_at: DateTime<Utc>,
    /// Version counter for optimistic concurrency.
    #[serde(default = "default_version")]
    pub version: u64,
}

fn default_version() -> u64 { 1 }

impl Point {
    pub fn new(id: PointId, vector: Vector, payload: HashMap<String, Value>) -> Self {
        Self { id, vector, payload, created_at: Utc::now(), version: 1 }
    }
}

/// A point with an associated similarity score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredPoint {
    pub id: PointId,
    pub score: f32,
    pub payload: HashMap<String, Value>,
    pub vector: Option<Vector>,
    pub version: u64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Search request / response
// ─────────────────────────────────────────────────────────────────────────────

/// Nearest-neighbour search request (Qdrant wire format).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchRequest {
    /// The query vector.
    pub vector: Vector,
    /// Maximum number of results.
    #[serde(default = "default_top_k")]
    pub limit: usize,
    /// Number of candidates explored during HNSW search (overrides collection default).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<SearchParams>,
    /// Payload filter applied after vector search.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<Filter>,
    /// Whether to return vectors in the response.
    #[serde(default)]
    pub with_vectors: bool,
    /// Whether to return payloads in the response.
    #[serde(default = "default_true")]
    pub with_payload: bool,
    /// Score threshold — exclude results below this score.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score_threshold: Option<f32>,
    /// Starting offset for pagination.
    #[serde(default)]
    pub offset: usize,
}

fn default_top_k() -> usize { 10 }
fn default_true() -> bool { true }

impl Default for SearchRequest {
    fn default() -> Self {
        Self {
            vector: Vec::new(),
            limit: 10,
            params: None,
            filter: None,
            with_vectors: false,
            with_payload: true,
            score_threshold: None,
            offset: 0,
        }
    }
}

/// Runtime HNSW search parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchParams {
    /// ef value (beam width) for this query.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hnsw_ef: Option<usize>,
    /// Whether to use exact search (no HNSW approximation).
    #[serde(default)]
    pub exact: bool,
    /// Quantisation rescoring flag.
    #[serde(default)]
    pub quantisation: Option<QuantisationSearchParams>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuantisationSearchParams {
    pub rescore: bool,
    pub oversampling: Option<f32>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Payload filtering
// ─────────────────────────────────────────────────────────────────────────────

/// A payload filter condition (Qdrant wire format).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Filter {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub must: Vec<Condition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub should: Vec<Condition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub must_not: Vec<Condition>,
}

impl Filter {
    pub fn new() -> Self {
        Self { must: vec![], should: vec![], must_not: vec![] }
    }
}

impl Default for Filter {
    fn default() -> Self {
        Self::new()
    }
}

/// A single filter condition.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Condition {
    /// Exact field match.
    Field(FieldCondition),
    /// Nested filter.
    Nested(Box<Filter>),
    /// Field existence check.
    IsEmpty(IsEmptyCondition),
    /// Null check.
    IsNull(IsNullCondition),
    /// Point-id membership.
    HasId(HasIdCondition),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldCondition {
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#match: Option<MatchCondition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<RangeCondition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geo_bounding_box: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geo_radius: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub values_count: Option<ValuesCountCondition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MatchCondition {
    Value(MatchValue),
    Any(MatchAny),
    Except(MatchExcept),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchValue {
    pub value: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchAny {
    pub any: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchExcept {
    pub except: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RangeCondition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gt: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gte: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lt: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lte: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ValuesCountCondition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gt: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gte: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lt: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lte: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IsEmptyCondition {
    pub is_empty: FieldPath,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IsNullCondition {
    pub is_null: FieldPath,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldPath {
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HasIdCondition {
    pub has_id: Vec<PointId>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Upsert / update / delete
// ─────────────────────────────────────────────────────────────────────────────

/// Request body for upserting points.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertPointsRequest {
    pub points: Vec<Point>,
    #[serde(default)]
    pub batch: Option<BatchPoints>,
}

/// Alternative batch format: parallel arrays.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchPoints {
    pub ids: Vec<PointId>,
    pub vectors: Vec<Vector>,
    #[serde(default)]
    pub payloads: Vec<HashMap<String, Value>>,
}

/// Result of an upsert / delete operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateResult {
    /// Sequential operation ID.
    pub operation_id: u64,
    pub status: UpdateStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum UpdateStatus {
    Acknowledged,
    Completed,
}

/// Request body for deleting points by IDs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeletePointsRequest {
    pub points: Vec<PointId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<Filter>,
}

/// Request body for setting / overwriting point payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetPayloadRequest {
    pub payload: HashMap<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub points: Option<Vec<PointId>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<Filter>,
}

/// Request body for scrolling through all points in a collection.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScrollRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<PointId>,
    #[serde(default = "default_scroll_limit")]
    pub limit: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<Filter>,
    #[serde(default = "default_true")]
    pub with_payload: bool,
    #[serde(default)]
    pub with_vectors: bool,
}

fn default_scroll_limit() -> usize { 10 }

/// Response for a scroll operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrollResponse {
    pub points: Vec<Point>,
    pub next_page_offset: Option<PointId>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Payload index
// ─────────────────────────────────────────────────────────────────────────────

/// Request to create a payload field index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateFieldIndexRequest {
    pub field_name: String,
    pub field_schema: FieldIndexSchema,
}

/// Type of index to create for a payload field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldIndexSchema {
    Keyword,
    Integer,
    Float,
    Geo,
    Text,
    Bool,
    Datetime,
}

// ─────────────────────────────────────────────────────────────────────────────
// Recommend API
// ─────────────────────────────────────────────────────────────────────────────

/// Request for the recommend-by-example endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecommendRequest {
    /// IDs of positive example points.
    pub positive: Vec<PointId>,
    /// IDs of negative example points.
    #[serde(default)]
    pub negative: Vec<PointId>,
    #[serde(default = "default_top_k")]
    pub limit: usize,
    #[serde(default = "default_true")]
    pub with_payload: bool,
    #[serde(default)]
    pub with_vectors: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<Filter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score_threshold: Option<f32>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn point_id_num_display() {
        let id = PointId::Num(42);
        assert_eq!(id.as_str_key(), "42");
        assert_eq!(id.to_string(), "42");
    }

    #[test]
    fn point_id_uuid_display() {
        let id = PointId::Uuid("abc-def".into());
        assert_eq!(id.as_str_key(), "abc-def");
    }

    #[test]
    fn collection_config_new() {
        let cfg = CollectionConfig::new(128, Distance::Cosine);
        assert_eq!(cfg.vectors.size, 128);
        assert_eq!(cfg.vectors.distance, Distance::Cosine);
        assert_eq!(cfg.replication_factor, 1);
    }

    #[test]
    fn point_new_stores_payload() {
        let mut payload = HashMap::new();
        payload.insert("category".into(), json!("tech"));
        let point = Point::new(PointId::Num(1), vec![0.1, 0.2, 0.3], payload);
        assert_eq!(point.payload["category"], json!("tech"));
    }

    #[test]
    fn search_request_defaults() {
        let req = SearchRequest::default();
        assert_eq!(req.limit, 10);
        assert!(req.with_payload);
        assert!(!req.with_vectors);
    }

    #[test]
    fn hnsw_config_defaults() {
        let cfg = HnswConfig::default();
        assert_eq!(cfg.m, 16);
        assert_eq!(cfg.m0, 32);
        assert_eq!(cfg.ef_construction, 100);
        assert_eq!(cfg.ef, 64);
    }

    #[test]
    fn update_status_serialisation() {
        let result = UpdateResult { operation_id: 1, status: UpdateStatus::Completed };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("completed"));
    }

    #[test]
    fn filter_default_empty() {
        let f = Filter::default();
        assert!(f.must.is_empty());
        assert!(f.should.is_empty());
        assert!(f.must_not.is_empty());
    }

    #[test]
    fn point_id_roundtrip_num() {
        let id = PointId::Num(99);
        let json = serde_json::to_string(&id).unwrap();
        let back: PointId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn point_id_roundtrip_uuid() {
        let id = PointId::Uuid("some-uuid".into());
        let json = serde_json::to_string(&id).unwrap();
        let back: PointId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn distance_default() {
        let d = Distance::default();
        assert_eq!(d, Distance::Cosine);
    }
}
