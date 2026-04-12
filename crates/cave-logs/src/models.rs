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
}
