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
        }
    }
}

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
}
