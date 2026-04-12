//! Prometheus-compatible data models.
//!
//! Mirrors the Prometheus HTTP API response envelope and data types
//! so that any Prometheus-aware client can talk to cave-metrics unchanged.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Prometheus HTTP API response envelope
// ---------------------------------------------------------------------------

/// Top-level response returned by all query/metadata endpoints.
/// Matches: {"status":"success","data":{...}} or {"status":"error",...}
#[derive(Debug, Serialize)]
pub struct PromResponse<T: Serialize> {
    pub status: &'static str,
    pub data: T,
}

impl<T: Serialize> PromResponse<T> {
    pub fn success(data: T) -> Self {
        Self { status: "success", data }
    }
}

/// {"status":"error","errorType":"...","error":"..."}
#[derive(Debug, Serialize)]
pub struct PromError {
    pub status: &'static str,
    #[serde(rename = "errorType")]
    pub error_type: String,
    pub error: String,
}

// ---------------------------------------------------------------------------
// Query result types
// ---------------------------------------------------------------------------

/// resultType discriminant used in query responses.
#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ResultType {
    Matrix,
    Vector,
    Scalar,
    String,
}

/// Instant query response data: {"resultType":"vector","result":[...]}
#[derive(Debug, Serialize)]
pub struct VectorData {
    #[serde(rename = "resultType")]
    pub result_type: &'static str,
    pub result: Vec<InstantSample>,
}

/// Range query response data: {"resultType":"matrix","result":[...]}
#[derive(Debug, Serialize)]
pub struct MatrixData {
    #[serde(rename = "resultType")]
    pub result_type: &'static str,
    pub result: Vec<RangeSample>,
}

/// A single instant sample: {"metric":{labels},"value":[timestamp,"value"]}
#[derive(Debug, Serialize)]
pub struct InstantSample {
    pub metric: HashMap<String, String>,
    /// [unix_timestamp_float, "value_string"]
    pub value: (f64, String),
}

/// A range sample: {"metric":{labels},"values":[[ts,"v"],...]}
#[derive(Debug, Serialize)]
pub struct RangeSample {
    pub metric: HashMap<String, String>,
    /// [[unix_timestamp_float, "value_string"], ...]
    pub values: Vec<(f64, String)>,
}

// ---------------------------------------------------------------------------
// Series / label metadata
// ---------------------------------------------------------------------------

/// Response for /api/v1/series — list of label sets.
#[derive(Debug, Serialize)]
pub struct SeriesData {
    pub data: Vec<HashMap<String, String>>,
}

/// Response for /api/v1/labels — list of label names.
#[derive(Debug, Serialize)]
pub struct LabelsData {
    pub data: Vec<String>,
}

/// Response for /api/v1/label/:name/values — list of label values.
#[derive(Debug, Serialize)]
pub struct LabelValuesData {
    pub data: Vec<String>,
}

// ---------------------------------------------------------------------------
// Query parameters (deserialised from query string)
// ---------------------------------------------------------------------------

/// Query parameters for GET /api/v1/query
#[derive(Debug, Deserialize)]
pub struct InstantQueryParams {
    /// PromQL expression
    pub query: String,
    /// Evaluation timestamp (RFC3339 or Unix seconds). Optional → now.
    pub time: Option<String>,
    /// Evaluation timeout, e.g. "30s". Optional.
    pub timeout: Option<String>,
}

/// Query parameters for GET /api/v1/query_range
#[derive(Debug, Deserialize)]
pub struct RangeQueryParams {
    /// PromQL expression
    pub query: String,
    /// Start of range (RFC3339 or Unix seconds)
    pub start: String,
    /// End of range (RFC3339 or Unix seconds)
    pub end: String,
    /// Resolution step, e.g. "15s" or "60"
    pub step: String,
    /// Evaluation timeout. Optional.
    pub timeout: Option<String>,
}

/// Query parameters for GET /api/v1/series
#[derive(Debug, Deserialize)]
pub struct SeriesParams {
    /// One or more series selectors, e.g. `match[]=up`
    #[serde(rename = "match[]")]
    pub matchers: Option<Vec<String>>,
    pub start: Option<String>,
    pub end: Option<String>,
}

/// Query parameters for GET /api/v1/labels
#[derive(Debug, Deserialize)]
pub struct LabelsParams {
    #[serde(rename = "match[]")]
    pub matchers: Option<Vec<String>>,
    pub start: Option<String>,
    pub end: Option<String>,
}

/// Query parameters for GET /api/v1/label/:name/values
#[derive(Debug, Deserialize)]
pub struct LabelValuesParams {
    #[serde(rename = "match[]")]
    pub matchers: Option<Vec<String>>,
    pub start: Option<String>,
    pub end: Option<String>,
//! Data models for cave-metrics.
use chrono::{DateTime, Utc};
use uuid::Uuid;
/// Metric type.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MetricType {
    Counter,
    Gauge,
    Histogram,
    Summary,
/// A metric descriptor (metadata about a metric).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metric {
    pub id: Uuid,
    pub name: String,
    pub labels: HashMap<String, String>,
    pub metric_type: MetricType,
    pub help: String,
    pub unit: String,
    pub created_at: DateTime<Utc>,
impl Metric {
    pub fn new(name: impl Into<String>, metric_type: MetricType) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            labels: HashMap::new(),
            metric_type,
            help: String::new(),
            unit: String::new(),
            created_at: Utc::now(),
/// A single data point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sample {
    pub timestamp: DateTime<Utc>,
    pub value: f64,
/// A time series: metric identity + ordered samples.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeSeries {
    pub id: Uuid,
    pub metric_name: String,
    pub labels: HashMap<String, String>,
    pub samples: Vec<Sample>,
impl TimeSeries {
    pub fn new(metric_name: impl Into<String>, labels: HashMap<String, String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            metric_name: metric_name.into(),
            labels,
            samples: Vec::new(),
    /// Fingerprint: name + sorted label pairs.
    pub fn fingerprint(metric_name: &str, labels: &HashMap<String, String>) -> String {
        let mut pairs: Vec<_> = labels.iter().collect();
        pairs.sort_by_key(|(k, _)| k.as_str());
        let label_str: String = pairs
            .into_iter()
            .map(|(k, v)| format!("{k}=\"{v}\""))
            .collect::<Vec<_>>()
            .join(",");
        format!("{metric_name}{{{label_str}}}")
/// PromQL-like query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricQuery {
    pub expr: String,
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
    pub step_seconds: Option<u64>,
/// Alert state.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AlertState {
    Inactive,
    Pending,
    Firing,
/// Alert rule definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertRule {
    pub id: Uuid,
    pub name: String,
    pub group: String,
    pub expr: String,
    pub for_duration_seconds: u64,
    pub labels: HashMap<String, String>,
    pub annotations: HashMap<String, String>,
    pub state: AlertState,
    pub last_evaluated: Option<DateTime<Utc>>,
    pub fired_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
impl AlertRule {
    pub fn new(name: impl Into<String>, group: impl Into<String>, expr: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            group: group.into(),
            expr: expr.into(),
            for_duration_seconds: 0,
            labels: HashMap::new(),
            annotations: HashMap::new(),
            state: AlertState::Inactive,
            last_evaluated: None,
            fired_at: None,
            created_at: Utc::now(),
/// Recording rule — pre-compute expensive queries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingRule {
    pub id: Uuid,
    pub name: String,
    pub group: String,
    pub expr: String,
    pub labels: HashMap<String, String>,
    pub interval_seconds: u64,
    pub last_evaluated: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
impl RecordingRule {
    pub fn new(name: impl Into<String>, group: impl Into<String>, expr: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            group: group.into(),
            expr: expr.into(),
            labels: HashMap::new(),
            interval_seconds: 60,
            last_evaluated: None,
            created_at: Utc::now(),
/// A scrape target (Prometheus-compatible).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrapeTarget {
    pub id: Uuid,
    pub job: String,
    pub address: String,          // host:port
    pub metrics_path: String,     // default /metrics
    pub scheme: String,           // http or https
    pub scrape_interval_seconds: u64,
    pub scrape_timeout_seconds: u64,
    pub labels: HashMap<String, String>,
    pub enabled: bool,
    pub last_scrape: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
impl ScrapeTarget {
    pub fn new(job: impl Into<String>, address: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            job: job.into(),
            address: address.into(),
            metrics_path: "/metrics".to_string(),
            scheme: "http".to_string(),
            scrape_interval_seconds: 15,
            scrape_timeout_seconds: 10,
            labels: HashMap::new(),
            enabled: true,
            last_scrape: None,
            last_error: None,
            created_at: Utc::now(),
    pub fn url(&self) -> String {
        format!("{}://{}{}", self.scheme, self.address, self.metrics_path)
/// Metadata about a metric (type, help, unit).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricMetadata {
    pub metric_name: String,
    pub metric_type: MetricType,
    pub help: String,
    pub unit: String,
// ---- DTOs ----
pub struct WriteRequest {
    pub metric_name: String,
    pub labels: HashMap<String, String>,
    pub samples: Vec<SampleRequest>,
pub struct SampleRequest {
    pub timestamp: Option<DateTime<Utc>>,
    pub value: f64,
pub struct QueryRequest {
    pub expr: String,
    pub time: Option<DateTime<Utc>>,
pub struct RangeQueryRequest {
    pub expr: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub step: u64,
pub struct QueryResult {
    pub status: String,
    pub data: QueryData,
pub struct QueryData {
    pub result_type: String,
    pub result: Vec<SeriesResult>,
pub struct SeriesResult {
    pub values: Vec<[serde_json::Value; 2]>,
pub struct CreateAlertRuleRequest {
    pub name: String,
    pub group: String,
    pub expr: String,
    pub for_duration_seconds: Option<u64>,
    pub labels: Option<HashMap<String, String>>,
    pub annotations: Option<HashMap<String, String>>,
pub struct CreateRecordingRuleRequest {
    pub name: String,
    pub group: String,
    pub expr: String,
    pub interval_seconds: Option<u64>,
    pub labels: Option<HashMap<String, String>>,
pub struct CreateScrapeTargetRequest {
    pub job: String,
    pub address: String,
    pub metrics_path: Option<String>,
    pub scheme: Option<String>,
    pub scrape_interval_seconds: Option<u64>,
    pub scrape_timeout_seconds: Option<u64>,
    pub labels: Option<HashMap<String, String>>,
}
