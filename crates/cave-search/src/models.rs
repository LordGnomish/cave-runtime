//! Domain models for cave-search.
//!
//! Wire-format compatible with the OpenSearch / Elasticsearch REST API so
//! existing clients (opensearch-rs, elasticsearch-rs, Kibana-style tooling)
//! work without modification.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────────────
// Document
// ─────────────────────────────────────────────────────────────────────────────

/// A document stored in the search index.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Document {
    /// Unique identifier within the index.
    #[serde(rename = "_id")]
    pub id: String,
    /// The index this document belongs to.
    #[serde(rename = "_index")]
    pub index: String,
    /// The raw document source fields.
    #[serde(rename = "_source")]
    pub source: HashMap<String, Value>,
    /// Relevance score assigned by the query (populated during search).
    #[serde(rename = "_score", skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    /// Monotonically increasing document version.
    #[serde(rename = "_version", skip_serializing_if = "Option::is_none")]
    pub version: Option<u64>,
    /// Sequence number used for optimistic concurrency control.
    #[serde(rename = "_seq_no", skip_serializing_if = "Option::is_none")]
    pub seq_no: Option<u64>,
    /// Custom routing value for tenant / shard placement.
    #[serde(rename = "_routing", skip_serializing_if = "Option::is_none")]
    pub routing: Option<String>,
    /// Wall-clock time when this document was indexed.
    pub indexed_at: DateTime<Utc>,
}

impl Document {
    /// Create a new document with an auto-generated UUID.
    pub fn new(index: impl Into<String>, source: HashMap<String, Value>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            index: index.into(),
            source,
            score: None,
            version: Some(1),
            seq_no: Some(0),
            routing: None,
            indexed_at: Utc::now(),
        }
    }

    /// Create a document with a caller-supplied identifier.
    pub fn with_id(
        id: impl Into<String>,
        index: impl Into<String>,
        source: HashMap<String, Value>,
    ) -> Self {
        Self {
            id: id.into(),
            index: index.into(),
            source,
            score: None,
            version: Some(1),
            seq_no: Some(0),
            routing: None,
            indexed_at: Utc::now(),
        }
    }

    /// Retrieve a field value by name (supports dot-notation for nested fields).
    pub fn get_field(&self, field: &str) -> Option<&Value> {
        let mut parts = field.splitn(2, '.');
        let head = parts.next()?;
        let rest = parts.next();
        let val = self.source.get(head)?;
        if let Some(r) = rest {
            val.get(r)
        } else {
            Some(val)
        }
    }

    /// Return the string representation of a field's value.
    pub fn get_field_text(&self, field: &str) -> Option<String> {
        self.get_field(field).map(|v| match v {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Array(arr) => arr
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect::<Vec<_>>()
                .join(" "),
            other => other.to_string(),
        })
    }

    /// Collect text across all searchable string fields.
    pub fn all_text_fields(&self) -> Vec<(String, String)> {
        let mut result = Vec::new();
        for (key, value) in &self.source {
            match value {
                Value::String(s) => {
                    result.push((key.clone(), s.clone()));
                }
                Value::Array(arr) => {
                    let text: String = arr
                        .iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                    if !text.is_empty() {
                        result.push((key.clone(), text));
                    }
                }
                _ => {}
            }
        }
        result
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Index Configuration
// ─────────────────────────────────────────────────────────────────────────────

/// Data type for a mapped field.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    /// Full-text analysed field (tokenised, lowercased, stop-word filtered).
    #[default]
    Text,
    /// Exact-match keyword field (no analysis).
    Keyword,
    /// Signed 64-bit integer.
    Long,
    /// Signed 32-bit integer.
    Integer,
    /// 64-bit floating-point number.
    Double,
    /// 32-bit floating-point number.
    Float,
    /// Boolean true/false.
    Boolean,
    /// ISO-8601 date / datetime string.
    Date,
    /// Nested JSON object.
    Object,
    /// Array of nested objects (each indexed separately).
    Nested,
    /// IP address (IPv4 or IPv6).
    Ip,
    /// Geo-point (lat, lon).
    GeoPoint,
    /// Binary data (stored but not indexed).
    Binary,
}

/// Mapping definition for a single document field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldMapping {
    /// Data type of this field.
    #[serde(rename = "type", default)]
    pub field_type: FieldType,
    /// Whether this field is indexed for search (default: true).
    #[serde(default = "default_true")]
    pub index: bool,
    /// Name of the analyser to use for indexing (text fields only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analyzer: Option<String>,
    /// Name of the analyser to use at query time (defaults to `analyzer`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_analyzer: Option<String>,
    /// Whether to store the original value separately from `_source`.
    #[serde(default)]
    pub store: bool,
    /// Term vector setting ("no", "yes", "with_positions", "with_offsets", …).
    #[serde(rename = "term_vector", skip_serializing_if = "Option::is_none")]
    pub term_vector: Option<String>,
    /// Boost factor applied to BM25 scores for this field.
    #[serde(default = "default_boost")]
    pub boost: f64,
    /// Date format string (date fields only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Nested field mappings (object / nested fields).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub properties: HashMap<String, FieldMapping>,
    /// Sub-fields (e.g. `title.keyword` for exact match on a text field).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub fields: HashMap<String, FieldMapping>,
}

fn default_true() -> bool { true }
fn default_boost() -> f64 { 1.0 }

impl Default for FieldMapping {
    fn default() -> Self {
        Self {
            field_type: FieldType::Text,
            index: true,
            analyzer: None,
            search_analyzer: None,
            store: false,
            term_vector: None,
            boost: 1.0,
            format: None,
            properties: HashMap::new(),
            fields: HashMap::new(),
        }
    }
}

/// Mapping for an entire index.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexMapping {
    /// Field-level mapping definitions.
    #[serde(default)]
    pub properties: HashMap<String, FieldMapping>,
    /// Whether unmapped fields are indexed dynamically.
    #[serde(default = "default_true")]
    pub dynamic: bool,
}

/// Analysis-related index settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AnalysisSettings {
    #[serde(default)]
    pub analyzer: HashMap<String, Value>,
    #[serde(default)]
    pub tokenizer: HashMap<String, Value>,
    #[serde(default)]
    pub filter: HashMap<String, Value>,
}

/// Index-level settings (mirrors the OpenSearch `settings.index` block).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexSettings {
    #[serde(default = "default_shards")]
    pub number_of_shards: u32,
    #[serde(default = "default_replicas")]
    pub number_of_replicas: u32,
    /// Hard cap on the result window (default: 10 000).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_result_window: Option<usize>,
    /// Refresh interval in seconds (−1 = manual).
    #[serde(default = "default_refresh_interval")]
    pub refresh_interval: i32,
    #[serde(default)]
    pub analysis: AnalysisSettings,
}

fn default_shards() -> u32 { 1 }
fn default_replicas() -> u32 { 1 }
fn default_refresh_interval() -> i32 { 1 }

impl Default for IndexSettings {
    fn default() -> Self {
        Self {
            number_of_shards: 1,
            number_of_replicas: 1,
            max_result_window: Some(10_000),
            refresh_interval: 1,
            analysis: AnalysisSettings::default(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Search Request / Response
// ─────────────────────────────────────────────────────────────────────────────

/// A full OpenSearch-compatible search request body.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchRequest {
    /// Root query clause.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<Value>,
    /// Maximum number of hits to return (default: 10).
    #[serde(default = "default_size")]
    pub size: usize,
    /// Number of hits to skip for pagination.
    #[serde(default)]
    pub from: usize,
    /// Sort criteria.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sort: Vec<Value>,
    /// Source filtering (bool, list of fields, or includes/excludes object).
    #[serde(rename = "_source", skip_serializing_if = "Option::is_none")]
    pub source: Option<Value>,
    /// Highlighting configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlight: Option<HighlightConfig>,
    /// Named aggregations.
    #[serde(rename = "aggs", default, skip_serializing_if = "HashMap::is_empty")]
    pub aggregations: HashMap<String, Value>,
    /// Whether to compute the exact total hit count.
    #[serde(default = "default_true")]
    pub track_total_hits: bool,
    /// Exclude hits whose score is below this threshold.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_score: Option<f64>,
    /// Tenant routing value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing: Option<String>,
    /// Runtime fields to include in the response.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<String>,
    /// Timeout string (e.g. "5s").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<String>,
}

fn default_size() -> usize { 10 }

/// Highlighting configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HighlightConfig {
    #[serde(default)]
    pub fields: HashMap<String, HighlightFieldConfig>,
    #[serde(default = "default_fragment_size")]
    pub fragment_size: usize,
    #[serde(default = "default_num_fragments")]
    pub number_of_fragments: usize,
    #[serde(default = "default_pre_tag")]
    pub pre_tags: Vec<String>,
    #[serde(default = "default_post_tag")]
    pub post_tags: Vec<String>,
    /// Analyser to use when highlighting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlight_query: Option<Value>,
}

fn default_fragment_size() -> usize { 150 }
fn default_num_fragments() -> usize { 5 }
fn default_pre_tag() -> Vec<String> { vec!["<em>".to_string()] }
fn default_post_tag() -> Vec<String> { vec!["</em>".to_string()] }

/// Per-field highlighting options.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HighlightFieldConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fragment_size: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number_of_fragments: Option<usize>,
    /// Bytes to return for a field that has no match (0 = omit).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_match_size: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_tags: Option<Vec<String>>,
}

/// An individual search hit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    #[serde(rename = "_index")]
    pub index: String,
    #[serde(rename = "_id")]
    pub id: String,
    #[serde(rename = "_score")]
    pub score: Option<f64>,
    #[serde(rename = "_source")]
    pub source: HashMap<String, Value>,
    #[serde(rename = "_version", skip_serializing_if = "Option::is_none")]
    pub version: Option<u64>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub highlight: HashMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub fields: HashMap<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sort: Vec<Value>,
}

/// Total-hits relation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TotalRelation {
    Eq,
    Gte,
}

/// Total hit count with relation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TotalHits {
    pub value: u64,
    pub relation: TotalRelation,
}

/// The `hits` section of a search response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HitsCollection {
    pub total: TotalHits,
    pub max_score: Option<f64>,
    pub hits: Vec<SearchHit>,
}

/// Full OpenSearch-compatible search response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    pub took: u64,
    pub timed_out: bool,
    #[serde(rename = "_shards")]
    pub shards: ShardStats,
    pub hits: HitsCollection,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub aggregations: HashMap<String, Value>,
    #[serde(rename = "_scroll_id", skip_serializing_if = "Option::is_none")]
    pub scroll_id: Option<String>,
}

/// Shard-level execution statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShardStats {
    pub total: u32,
    pub successful: u32,
    pub skipped: u32,
    pub failed: u32,
}

impl Default for ShardStats {
    fn default() -> Self {
        Self { total: 1, successful: 1, skipped: 0, failed: 0 }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Bulk API
// ─────────────────────────────────────────────────────────────────────────────

/// Action descriptor for a bulk operation item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkActionMeta {
    #[serde(rename = "_index", skip_serializing_if = "Option::is_none")]
    pub index: Option<String>,
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "_routing", skip_serializing_if = "Option::is_none")]
    pub routing: Option<String>,
}

/// Result for a single bulk item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkItemResult {
    #[serde(rename = "_index")]
    pub index: String,
    #[serde(rename = "_id")]
    pub id: String,
    #[serde(rename = "_version")]
    pub version: u64,
    pub result: String,
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<BulkError>,
}

/// Error payload inside a bulk item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkError {
    #[serde(rename = "type")]
    pub error_type: String,
    pub reason: String,
    pub status: u16,
}

/// Full response for a `_bulk` request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkResponse {
    pub took: u64,
    pub errors: bool,
    pub items: Vec<HashMap<String, BulkItemResult>>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Index Management
// ─────────────────────────────────────────────────────────────────────────────

/// Body for `PUT /{index}`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CreateIndexRequest {
    #[serde(default)]
    pub mappings: IndexMapping,
    #[serde(default)]
    pub settings: IndexSettings,
    #[serde(default)]
    pub aliases: HashMap<String, AliasConfig>,
}

/// Alias configuration attached to an index.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AliasConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<Value>,
    #[serde(default)]
    pub is_write_index: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing: Option<String>,
}

/// Index-level statistics.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexStats {
    pub doc_count: u64,
    pub deleted_count: u64,
    pub store_size_bytes: u64,
    pub index_total: u64,
    pub index_time_ms: u64,
    pub search_total: u64,
    pub search_time_ms: u64,
}

/// Full index description returned by `GET /{index}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexInfo {
    pub aliases: HashMap<String, Value>,
    pub mappings: IndexMapping,
    pub settings: IndexSettingsWrapper,
}

/// OpenSearch wire wrapping for index settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexSettingsWrapper {
    pub index: IndexSettings,
}

// ─────────────────────────────────────────────────────────────────────────────
// Sort
// ─────────────────────────────────────────────────────────────────────────────

/// Sort direction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    #[default]
    Desc,
    Asc,
}

/// A parsed sort criterion for a single field.
#[derive(Debug, Clone)]
pub struct SortField {
    pub field: String,
    pub order: SortOrder,
    pub missing: Option<Value>,
    pub unmapped_type: Option<FieldType>,
}

impl SortField {
    /// Parse the OpenSearch sort format: either `"field"` or `{"field":{"order":"asc"}}`.
    pub fn parse(value: &Value) -> Option<Self> {
        match value {
            Value::String(field) => Some(Self {
                field: field.clone(),
                order: SortOrder::Desc,
                missing: None,
                unmapped_type: None,
            }),
            Value::Object(map) => {
                let (field, opts) = map.iter().next()?;
                let order = opts
                    .get("order")
                    .and_then(|o| o.as_str())
                    .map(|s| if s == "asc" { SortOrder::Asc } else { SortOrder::Desc })
                    .unwrap_or_default();
                let missing = opts.get("missing").cloned();
                Some(Self {
                    field: field.clone(),
                    order,
                    missing,
                    unmapped_type: None,
                })
            }
            _ => None,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Count / Cat APIs
// ─────────────────────────────────────────────────────────────────────────────

/// Response for `GET /{index}/_count`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CountResponse {
    pub count: u64,
    #[serde(rename = "_shards")]
    pub shards: ShardStats,
}

/// One row in the `_cat/indices` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatIndexRow {
    pub health: String,
    pub status: String,
    pub index: String,
    pub uuid: String,
    pub pri: u32,
    pub rep: u32,
    #[serde(rename = "docs.count")]
    pub docs_count: u64,
    #[serde(rename = "docs.deleted")]
    pub docs_deleted: u64,
    #[serde(rename = "store.size")]
    pub store_size: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Alias
// ─────────────────────────────────────────────────────────────────────────────

/// An alias that points one or more indices under a single name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexAlias {
    pub alias: String,
    pub index: String,
    pub is_write_index: bool,
    pub filter: Option<Value>,
    pub routing: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Tenant isolation helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Prefix an index name with the tenant ID for namespace isolation.
pub fn tenant_index(tenant_id: &str, index: &str) -> String {
    format!("{}:{}", tenant_id, index)
}

/// Strip the tenant prefix from an index name.
pub fn strip_tenant(qualified_name: &str) -> &str {
    qualified_name.splitn(2, ':').last().unwrap_or(qualified_name)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn document_get_field_nested() {
        let mut source = HashMap::new();
        source.insert("author".to_string(), json!({"name": "Alice", "age": 30}));
        let doc = Document::new("test", source);
        assert!(doc.get_field("author").is_some());
        assert_eq!(doc.get_field_text("author.name"), Some("Alice".to_string()));
    }

    #[test]
    fn document_all_text_fields_skips_non_string() {
        let mut source = HashMap::new();
        source.insert("title".to_string(), json!("Hello World"));
        source.insert("count".to_string(), json!(42));
        let doc = Document::new("test", source);
        let fields = doc.all_text_fields();
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].0, "title");
    }

    #[test]
    fn document_array_field_joined() {
        let mut source = HashMap::new();
        source.insert("tags".to_string(), json!(["rust", "search", "engine"]));
        let doc = Document::new("test", source);
        let fields = doc.all_text_fields();
        assert_eq!(fields[0].1, "rust search engine");
    }

    #[test]
    fn search_request_defaults() {
        let req: SearchRequest = serde_json::from_str("{}").unwrap();
        assert_eq!(req.size, 10);
        assert_eq!(req.from, 0);
        assert!(req.track_total_hits);
    }

    #[test]
    fn field_mapping_default() {
        let fm = FieldMapping::default();
        assert_eq!(fm.field_type, FieldType::Text);
        assert!(fm.index);
        assert_eq!(fm.boost, 1.0);
    }

    #[test]
    fn sort_field_parse_string() {
        let sf = SortField::parse(&json!("created_at")).unwrap();
        assert_eq!(sf.field, "created_at");
        assert_eq!(sf.order, SortOrder::Desc);
    }

    #[test]
    fn sort_field_parse_object() {
        let sf = SortField::parse(&json!({"price": {"order": "asc"}})).unwrap();
        assert_eq!(sf.field, "price");
        assert_eq!(sf.order, SortOrder::Asc);
    }

    #[test]
    fn tenant_isolation_roundtrip() {
        let qualified = tenant_index("acme", "logs");
        assert_eq!(qualified, "acme:logs");
        assert_eq!(strip_tenant(&qualified), "logs");
    }

    #[test]
    fn shard_stats_default() {
        let ss = ShardStats::default();
        assert_eq!(ss.total, 1);
        assert_eq!(ss.successful, 1);
        assert_eq!(ss.failed, 0);
    }

    #[test]
    fn bulk_item_result_roundtrip() {
        let item = BulkItemResult {
            index: "my-index".into(),
            id: "1".into(),
            version: 1,
            result: "created".into(),
            status: 201,
            error: None,
        };
        let json = serde_json::to_string(&item).unwrap();
        let back: BulkItemResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.result, "created");
    }

    #[test]
    fn index_settings_max_result_window() {
        let s = IndexSettings::default();
        assert_eq!(s.max_result_window, Some(10_000));
    }
}
