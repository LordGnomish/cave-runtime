<<<<<<< HEAD
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
=======
//! Loki-compatible data models.
//!
//! Mirrors Loki's push and query API shapes so that any
//! Loki-aware client (Alloy, Promtail, Grafana) works unchanged.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Push API — POST /loki/api/v1/push
// ---------------------------------------------------------------------------

/// Top-level push request body.
/// {"streams": [{...}, ...]}
#[derive(Debug, Deserialize)]
pub struct PushRequest {
    pub streams: Vec<StreamEntry>,
}

/// A single log stream with its labels and log lines.
/// {"stream": {"app": "foo"}, "values": [["<ns_ts>", "line"], ...]}
#[derive(Debug, Deserialize)]
pub struct StreamEntry {
    /// Label set for this stream, e.g. {"app": "myapp", "level": "error"}
    pub stream: HashMap<String, String>,
    /// Log lines: each is [nanosecond_unix_timestamp_string, log_line_string]
    pub values: Vec<(String, String)>,
}

// ---------------------------------------------------------------------------
// Query result types
// ---------------------------------------------------------------------------

/// Loki response envelope: {"status":"success","data":{...}}
#[derive(Debug, Serialize)]
pub struct LokiResponse<T: Serialize> {
    pub status: &'static str,
    pub data: T,
}

impl<T: Serialize> LokiResponse<T> {
    pub fn success(data: T) -> Self {
        Self { status: "success", data }
    }
}

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
>>>>>>> claude/gallant-cartwright
        }
    }
}

<<<<<<< HEAD
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
=======
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
>>>>>>> claude/gallant-cartwright
}
