//! Domain models for cave-logs.
//! Data model: streams (label sets + entries), push/query API types, alerting.
//! Domain models for cave-logs.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ── Log Level ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

// ── Core Entry ──────────────────────────────────────────────────────────────

/// A single log line with parsed metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub id: Uuid,
    pub stream_id: Option<Uuid>,
    pub timestamp: DateTime<Utc>,
    pub level: LogLevel,
    pub message: String,
    pub service: String,
    /// Indexed labels (low cardinality, used for filtering).
    pub labels: HashMap<String, String>,
    /// Arbitrary structured fields from JSON logs.
    pub fields: serde_json::Value,
    /// Original raw line before parsing.
    pub raw: Option<String>,
}

// ── Log Stream ──────────────────────────────────────────────────────────────

/// A named stream of logs (analogous to a Loki stream or an ELK index).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogStream {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub labels_schema: Vec<String>,
    pub retention: RetentionPolicy,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionPolicy {
    /// Maximum number of entries to keep in memory for this stream.
    pub max_entries: usize,
    /// Maximum age in hours before entries are evicted.
    pub max_age_hours: u64,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            max_entries: 100_000,
            max_age_hours: 168, // 7 days
        }
    }
}

// ─── Label matcher ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct LabelMatcher {
    pub name: String,
    pub op: MatchOp,
    pub value: String,
}

impl LabelMatcher {
    pub fn matches_opt(&self, val: Option<&str>) -> bool {
        match self.op {
            MatchOp::Eq => val == Some(self.value.as_str()),
            MatchOp::Ne => val != Some(self.value.as_str()),
            MatchOp::Re => val
                .and_then(|v| {
                    regex::Regex::new(&format!("^(?:{})$", self.value))
                        .ok()
                        .map(|re| re.is_match(v))
                })
                .unwrap_or(false),
            MatchOp::NRe => !val
                .and_then(|v| {
                    regex::Regex::new(&format!("^(?:{})$", self.value))
                        .ok()
                        .map(|re| re.is_match(v))
                })
                .unwrap_or(false),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchOp {
    Eq,  // =
    Ne,  // !=
    Re,  // =~
    NRe, // !~
}

// ─── Log entry / stream ───────────────────────────────────────────────────────

/// A single log entry: timestamp + text line + optional structured metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: DateTime<Utc>,
    pub line: String,
    /// Structured metadata attached to this specific entry (not stream-level labels).
    pub structured_metadata: HashMap<String, String>,
}

/// A log stream: label set + ordered entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogStream {
    pub labels: Labels,
    pub entries: Vec<LogEntry>,
}

// ─── Push API ─────────────────────────────────────────────────────────────────

/// JSON push request body (`/loki/api/v1/push`).
#[derive(Debug, Deserialize)]
pub struct PushRequest {
    pub streams: Vec<StreamPush>,
}

#[derive(Debug, Deserialize)]
pub struct StreamPush {
    pub stream: HashMap<String, String>,
    /// Each element is `[timestamp_ns_string, log_line]` or
    /// `[timestamp_ns_string, log_line, structured_metadata_json_string]`.
    pub values: Vec<serde_json::Value>,
}

// ─── Query API ────────────────────────────────────────────────────────────────

/// Data portion of a streams query result.
/// {"resultType":"streams","result":[...]}
#[derive(Debug, Serialize)]
pub struct StreamsData {
    #[serde(rename = "resultType")]
    pub result_type: &'static str,
    pub result: Vec<StreamResult>,
    pub stats: QueryStats,
}

/// A single stream result entry.
/// {"stream": {labels}, "values": [[ts, line], ...]}
#[derive(Debug, Serialize)]
pub struct StreamResult {
    pub stream: HashMap<String, String>,
    pub values: Vec<(String, String)>,
}

/// Minimal query stats block (Loki includes this in every response).
#[derive(Debug, Serialize)]
pub struct QueryStats {
    pub summary: StatsSummary,
}

#[derive(Debug, Serialize)]
pub struct StatsSummary {
    #[serde(rename = "bytesProcessedPerSecond")]
    pub bytes_processed_per_second: u64,
    #[serde(rename = "linesProcessedPerSecond")]
    pub lines_processed_per_second: u64,
    #[serde(rename = "totalBytesProcessed")]
    pub total_bytes_processed: u64,
    #[serde(rename = "totalLinesProcessed")]
    pub total_lines_processed: u64,
    #[serde(rename = "execTime")]
    pub exec_time: f64,
}

impl Default for QueryStats {
    fn default() -> Self {
        Self {
            summary: StatsSummary {
                bytes_processed_per_second: 0,
                lines_processed_per_second: 0,
                total_bytes_processed: 0,
                total_lines_processed: 0,
                exec_time: 0.0,
            },
/// Data portion of a streams query result.
/// {"resultType":"streams","result":[...]}
#[derive(Debug, Serialize)]
pub struct StreamsData {
    #[serde(rename = "resultType")]
    pub result_type: &'static str,
    pub result: Vec<StreamResult>,
    pub stats: QueryStats,
}

/// A single stream result entry.
/// {"stream": {labels}, "values": [[ts, line], ...]}
#[derive(Debug, Serialize)]
pub struct StreamResult {
    pub stream: HashMap<String, String>,
    pub values: Vec<(String, String)>,
}

/// Minimal query stats block (Loki includes this in every response).
#[derive(Debug, Serialize)]
pub struct QueryStats {
    pub summary: StatsSummary,
}

#[derive(Debug, Serialize)]
pub struct StatsSummary {
    #[serde(rename = "bytesProcessedPerSecond")]
    pub bytes_processed_per_second: u64,
    #[serde(rename = "linesProcessedPerSecond")]
    pub lines_processed_per_second: u64,
    #[serde(rename = "totalBytesProcessed")]
    pub total_bytes_processed: u64,
    #[serde(rename = "totalLinesProcessed")]
    pub total_lines_processed: u64,
    #[serde(rename = "execTime")]
    pub exec_time: f64,
}

impl Default for QueryStats {
    fn default() -> Self {
        Self {
            summary: StatsSummary {
                bytes_processed_per_second: 0,
                lines_processed_per_second: 0,
                total_bytes_processed: 0,
                total_lines_processed: 0,
                exec_time: 0.0,
            },
#[serde(tag = "resultType", rename_all = "camelCase")]
pub enum QueryData {
    #[serde(rename = "streams")]
    Streams { result: Vec<StreamResult> },
    #[serde(rename = "matrix")]
    Matrix { result: Vec<MatrixResult> },
    #[serde(rename = "vector")]
    Vector { result: Vec<VectorResult> },
#[derive(Debug, Clone, Serialize)]
    /// Each value: `[timestamp_ns_string, log_line]`.
    pub values: Vec<[String; 2]>,
pub struct MatrixResult {
    pub metric: HashMap<String, String>,
    /// Each sample: `[unix_timestamp_float, value_string]`.
    pub values: Vec<(f64, String)>,
pub struct VectorResult {
    pub metric: HashMap<String, String>,
    pub value: (f64, String),
// ─── Tail API ─────────────────────────────────────────────────────────────────
#[derive(Debug, Clone, Serialize)]
pub struct TailResponse {
    pub streams: Vec<StreamResult>,
    pub dropped_entries: Option<Vec<DroppedEntry>>,
#[derive(Debug, Clone, Serialize)]
pub struct DroppedEntry {
    pub timestamp: String,
    pub labels: String,
// ─── Alerting ─────────────────────────────────────────────────────────────────
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertRule {
    pub id: Uuid,
    pub name: String,
    /// LogQL metric expression that evaluates to a scalar.
    pub expr: String,
    pub duration_secs: u64,
    pub condition: AlertCondition,
    pub severity: AlertSeverity,
    pub annotations: HashMap<String, String>,
    pub tenant: Option<String>,
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertCondition {
    pub op: CompareOp,
    pub threshold: f64,
impl AlertCondition {
    pub fn eval(&self, value: f64) -> bool {
        match self.op {
            CompareOp::Gt => value > self.threshold,
            CompareOp::Gte => value >= self.threshold,
            CompareOp::Lt => value < self.threshold,
            CompareOp::Lte => value <= self.threshold,
            CompareOp::Eq => (value - self.threshold).abs() < f64::EPSILON,
            CompareOp::Ne => (value - self.threshold).abs() >= f64::EPSILON,
        }
    }
}

// ── Log Query ───────────────────────────────────────────────────────────────

/// Aggregation / operation type for a query (LogQL-like).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogQueryOp {
    /// Return matching log lines (default).
    Filter,
    /// Count lines per time bucket.
    CountOverTime,
    /// Lines-per-second rate per time bucket.
    Rate,
    /// Top-K services / labels by volume.
    TopK,
    /// Full-text keyword search.
    FullTextSearch,
}

/// A LogQL-inspired query descriptor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogQuery {
    /// Free-form expression (stored for display; filtering uses the explicit fields).
    pub expr: String,
    pub stream_id: Option<Uuid>,
    pub level: Option<String>,
    pub service: Option<String>,
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
    /// Regex applied to the log message.
    pub regex_filter: Option<String>,
    /// Substring full-text search on message.
    pub full_text: Option<String>,
    pub operation: LogQueryOp,
    pub limit: Option<usize>,
    /// Bucket width in seconds for CountOverTime / Rate.
    pub step_seconds: Option<u64>,
    /// K for TopK.
    pub top_k: Option<usize>,
}

// ── Alerts ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertCondition {
    GreaterThan,
    LessThan,
    EqualTo,
    /// Count how many log lines match the query's regex_filter pattern.
    PatternMatch,
    /// Detect anomalous error rate vs recent baseline.
    AnomalyDetected,
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CompareOp {
    Gt,
    Gte,
    Lt,
    Lte,
    Eq,
    Ne,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlertSeverity {
    Info,
    Warning,
    Critical,
}

/// A log-based alert rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogAlert {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    /// The log query that produces the numeric value to evaluate.
    pub query: LogQuery,
    pub condition: AlertCondition,
    pub threshold: f64,
    /// Sliding evaluation window in seconds.
    pub window_seconds: u64,
    pub severity: AlertSeverity,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub last_triggered: Option<DateTime<Utc>>,
}

// ── Pipelines / Parse Rules ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParseFormat {
    /// Named-capture regex (e.g. `(?P<level>\w+) (?P<msg>.*)`).
    Regex,
    /// Parse the message as JSON and merge top-level keys as labels.
    Json,
    /// logfmt key=value pairs.
    Logfmt,
    /// Grok pattern (stored as string; resolved to regex at runtime).
    Grok,
}

/// One parsing step within a pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseRule {
    pub id: Uuid,
    pub name: String,
    /// The pattern string (regex / grok pattern).
    pub pattern: String,
    /// Named capture groups to promote to labels.
    pub labels: Vec<String>,
    pub format: ParseFormat,
}

/// An ordered sequence of parse rules and filters applied to incoming logs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogPipeline {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub parse_rules: Vec<ParseRule>,
    /// Label keys to extract via key=value scanning.
    pub label_extractors: Vec<String>,
    /// Label keys to strip from the final entry.
    pub drop_labels: Vec<String>,
    /// Regex patterns — lines matching any of these are flagged as filtered.
    pub filters: Vec<String>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}

// ── Dashboards ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardPanel {
    pub id: Uuid,
    pub title: String,
    pub query: LogQuery,
    /// One of "table", "timeseries", "bar", "stat".
    pub visualization: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogDashboard {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub panels: Vec<DashboardPanel>,
    pub created_at: DateTime<Utc>,
// ---------------------------------------------------------------------------
// Query parameters
// ---------------------------------------------------------------------------
/// GET /loki/api/v1/query
#[derive(Debug, Deserialize)]
pub struct InstantQueryParams {
    /// LogQL expression
    pub query: String,
    /// Max entries to return (default 100)
    pub limit: Option<u64>,
    /// Evaluation timestamp (nanoseconds or RFC3339)
    pub time: Option<String>,
    /// Log direction: "forward" | "backward" (default "backward")
    pub direction: Option<String>,
}
/// GET /loki/api/v1/query_range
#[derive(Debug, Deserialize)]
pub struct RangeQueryParams {
    /// LogQL expression
    pub query: String,
    /// Max entries to return (default 100)
    pub limit: Option<u64>,
    /// Start time (nanoseconds or RFC3339)
    pub start: Option<String>,
    /// End time (nanoseconds or RFC3339)
    pub end: Option<String>,
    /// Query resolution step (duration string or nanoseconds)
    pub step: Option<String>,
    /// Log direction: "forward" | "backward"
    pub direction: Option<String>,
}
/// GET /loki/api/v1/labels  and  GET /loki/api/v1/label/:name/values
#[derive(Debug, Deserialize)]
pub struct LabelParams {
    pub start: Option<String>,
    pub end: Option<String>,
    pub query: Option<String>,
    Critical,
    Warning,
    Info,
}
#[derive(Debug, Clone, Serialize)]
pub struct FiredAlert {
    pub rule_id: Uuid,
    pub rule_name: String,
    pub value: f64,
    pub fired_at: DateTime<Utc>,
    pub severity: AlertSeverity,
    pub annotations: HashMap<String, String>,
}
// ─── Protobuf types (Loki wire format) ───────────────────────────────────────
pub mod proto {
    /// `/loki/api/v1/push` protobuf body (Snappy-compressed).
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct PushRequest {
        #[prost(message, repeated, tag = "1")]
        pub streams: Vec<StreamAdapter>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct StreamAdapter {
        #[prost(string, tag = "1")]
        pub labels: String,
        #[prost(message, repeated, tag = "2")]
        pub entries: Vec<EntryAdapter>,
        #[prost(string, tag = "3")]
        pub hash: String,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct EntryAdapter {
        #[prost(message, optional, tag = "1")]
        pub timestamp: Option<prost_types::Timestamp>,
        #[prost(string, tag = "2")]
        pub line: String,
        #[prost(message, repeated, tag = "3")]
        pub structured_metadata: Vec<LabelPairAdapter>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct LabelPairAdapter {
        #[prost(string, tag = "1")]
        pub name: String,
        #[prost(string, tag = "2")]
        pub value: String,
    }
}
