//! Data models for cave-metrics.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Metric type.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MetricType {
    Counter,
    Gauge,
    Histogram,
    Summary,
}

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
}

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
        }
    }
}

/// A single data point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sample {
    pub timestamp: DateTime<Utc>,
    pub value: f64,
}

/// A time series: metric identity + ordered samples.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeSeries {
    pub id: Uuid,
    pub metric_name: String,
    pub labels: HashMap<String, String>,
    pub samples: Vec<Sample>,
}

impl TimeSeries {
    pub fn new(metric_name: impl Into<String>, labels: HashMap<String, String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            metric_name: metric_name.into(),
            labels,
            samples: Vec::new(),
        }
    }

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
    }
}

/// PromQL-like query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricQuery {
    pub expr: String,
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
    pub step_seconds: Option<u64>,
}

/// Alert state.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AlertState {
    Inactive,
    Pending,
    Firing,
}

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
}

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
        }
    }
}

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
}

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
        }
    }
}

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
}

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
        }
    }

    pub fn url(&self) -> String {
        format!("{}://{}{}", self.scheme, self.address, self.metrics_path)
    }
}

/// Metadata about a metric (type, help, unit).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricMetadata {
    pub metric_name: String,
    pub metric_type: MetricType,
    pub help: String,
    pub unit: String,
}

// ---- DTOs ----

#[derive(Debug, Deserialize)]
pub struct WriteRequest {
    pub metric_name: String,
    pub labels: HashMap<String, String>,
    pub samples: Vec<SampleRequest>,
}

#[derive(Debug, Deserialize)]
pub struct SampleRequest {
    pub timestamp: Option<DateTime<Utc>>,
    pub value: f64,
}

#[derive(Debug, Deserialize)]
pub struct QueryRequest {
    pub expr: String,
    pub time: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct RangeQueryRequest {
    pub expr: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub step: u64,
}

#[derive(Debug, Serialize)]
pub struct QueryResult {
    pub status: String,
    pub data: QueryData,
}

#[derive(Debug, Serialize)]
pub struct QueryData {
    pub result_type: String,
    pub result: Vec<SeriesResult>,
}

#[derive(Debug, Serialize)]
pub struct SeriesResult {
    pub metric: HashMap<String, String>,
    pub values: Vec<[serde_json::Value; 2]>,
}

#[derive(Debug, Deserialize)]
pub struct CreateAlertRuleRequest {
    pub name: String,
    pub group: String,
    pub expr: String,
    pub for_duration_seconds: Option<u64>,
    pub labels: Option<HashMap<String, String>>,
    pub annotations: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateRecordingRuleRequest {
    pub name: String,
    pub group: String,
    pub expr: String,
    pub interval_seconds: Option<u64>,
    pub labels: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateScrapeTargetRequest {
    pub job: String,
    pub address: String,
    pub metrics_path: Option<String>,
    pub scheme: Option<String>,
    pub scrape_interval_seconds: Option<u64>,
    pub scrape_timeout_seconds: Option<u64>,
    pub labels: Option<HashMap<String, String>>,
}
