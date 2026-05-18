// SPDX-License-Identifier: AGPL-3.0-or-later
//! Core domain types for cave-logs — mirrors Loki's data model.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

/// Tenant identifier extracted from X-Scope-OrgID header.
/// Empty string or "fake" is treated as the anonymous tenant.
pub type TenantId = String;

/// Nanosecond-precision Unix timestamp, matching Loki's wire format.
pub type TimestampNs = i64;

/// A set of key-value label pairs that identify a log stream.
/// Labels are sorted by key for stable fingerprinting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Labels(pub HashMap<String, String>);

impl Labels {
    pub fn new(map: HashMap<String, String>) -> Self {
        Self(map)
    }

    /// Canonical string representation: `{a="v1",b="v2"}` (keys sorted).
    pub fn to_selector(&self) -> String {
        let mut pairs: Vec<(&str, &str)> = self.0.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        pairs.sort_by_key(|(k, _)| *k);
        let inner: Vec<String> = pairs.iter().map(|(k, v)| format!("{}=\"{}\"", k, v)).collect();
        format!("{{{}}}", inner.join(","))
    }

    /// Stable 64-bit fingerprint of the label set.
    pub fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        use std::collections::hash_map::DefaultHasher;
        let mut sorted: Vec<(&String, &String)> = self.0.iter().collect();
        sorted.sort_by_key(|(k, _)| k.as_str());
        let mut hasher = DefaultHasher::new();
        for (k, v) in &sorted {
            k.hash(&mut hasher);
            v.hash(&mut hasher);
        }
        hasher.finish()
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(|s| s.as_str())
    }

    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.0.insert(key.into(), value.into());
    }

    pub fn remove(&mut self, key: &str) -> Option<String> {
        self.0.remove(key)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &String)> {
        self.0.iter()
    }
}

impl From<HashMap<String, String>> for Labels {
    fn from(m: HashMap<String, String>) -> Self {
        Self(m)
    }
}

/// A single log entry stored in a stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    /// Nanosecond Unix timestamp.
    pub ts: TimestampNs,
    /// The raw log line.
    pub line: String,
    /// Structured metadata attached by the sender (Loki 2.9+).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

impl LogEntry {
    pub fn new(ts: TimestampNs, line: impl Into<String>) -> Self {
        Self { ts, line: line.into(), metadata: HashMap::new() }
    }

    pub fn with_metadata(mut self, metadata: HashMap<String, String>) -> Self {
        self.metadata = metadata;
        self
    }

    pub fn timestamp(&self) -> DateTime<Utc> {
        let secs = self.ts / 1_000_000_000;
        let nanos = (self.ts % 1_000_000_000) as u32;
        DateTime::from_timestamp(secs, nanos).unwrap_or_default()
    }

    pub fn size_bytes(&self) -> usize {
        self.line.len()
    }
}

/// A log stream = a set of labels + an ordered sequence of entries.
#[derive(Debug, Clone)]
pub struct LogStream {
    pub labels: Labels,
    pub tenant: TenantId,
    /// Entries sorted ascending by timestamp.
    pub entries: Vec<LogEntry>,
}

impl LogStream {
    pub fn new(labels: Labels, tenant: impl Into<TenantId>) -> Self {
        Self { labels, tenant: tenant.into(), entries: Vec::new() }
    }

    pub fn fingerprint(&self) -> u64 {
        self.labels.fingerprint()
    }

    pub fn push(&mut self, entry: LogEntry) {
        // Insert maintaining sort order (most pushes are append, so optimise for that).
        match self.entries.last() {
            Some(last) if last.ts <= entry.ts => self.entries.push(entry),
            _ => {
                let pos = self.entries.partition_point(|e| e.ts <= entry.ts);
                self.entries.insert(pos, entry);
            }
        }
    }

    pub fn byte_size(&self) -> usize {
        self.entries.iter().map(|e| e.size_bytes()).sum()
    }
}

/// Query direction for log results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    #[default]
    Backward,
    Forward,
}

/// Compression codec used by a chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    None,
    Gzip,
    #[default]
    Snappy,
    Lz4,
    Zstd,
}

/// A single compressed chunk of log entries for one stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub stream_fp: u64,
    pub tenant: TenantId,
    pub min_ts: TimestampNs,
    pub max_ts: TimestampNs,
    pub codec: Codec,
    pub data: Vec<u8>,
    pub num_entries: u64,
    pub uncompressed_size: u64,
}

/// Loki-compatible push request (JSON wire format).
#[derive(Debug, Deserialize)]
pub struct PushRequest {
    pub streams: Vec<StreamValue>,
}

#[derive(Debug, Deserialize)]
pub struct StreamValue {
    pub stream: HashMap<String, String>,
    pub values: Vec<EntryValue>,
}

/// `["<ts_ns>", "<line>"]` or `["<ts_ns>", "<line>", {metadata}]`
#[derive(Debug)]
pub struct EntryValue {
    pub ts_ns: TimestampNs,
    pub line: String,
    pub metadata: Option<HashMap<String, String>>,
}

impl<'de> Deserialize<'de> for EntryValue {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let v: serde_json::Value = Deserialize::deserialize(d)?;
        let arr = v.as_array().ok_or_else(|| D::Error::custom("entry must be array"))?;
        if arr.len() < 2 {
            return Err(D::Error::custom("entry array must have at least 2 elements"));
        }
        let ts_str = arr[0].as_str().ok_or_else(|| D::Error::custom("timestamp must be string"))?;
        let ts_ns: TimestampNs = ts_str.parse().map_err(|_| D::Error::custom("invalid timestamp"))?;
        let line = arr[1].as_str().ok_or_else(|| D::Error::custom("line must be string"))?.to_owned();
        let metadata = if arr.len() > 2 {
            serde_json::from_value(arr[2].clone()).ok()
        } else {
            None
        };
        Ok(Self { ts_ns, line, metadata })
    }
}

/// Loki query result types.

#[derive(Debug, Serialize)]
pub struct QueryResponse {
    pub status: &'static str,
    pub data: QueryData,
}

#[derive(Debug, Serialize)]
#[serde(tag = "resultType", content = "result", rename_all = "camelCase")]
pub enum QueryData {
    #[serde(rename = "streams")]
    Streams(Vec<StreamResult>),
    #[serde(rename = "matrix")]
    Matrix(Vec<MatrixResult>),
    #[serde(rename = "vector")]
    Vector(Vec<VectorResult>),
}

#[derive(Debug, Serialize)]
pub struct StreamResult {
    pub stream: HashMap<String, String>,
    pub values: Vec<(String, String)>, // (ts_ns_str, line)
}

#[derive(Debug, Serialize)]
pub struct MatrixResult {
    pub metric: HashMap<String, String>,
    pub values: Vec<(f64, String)>, // (unix_seconds_f64, value_str)
}

#[derive(Debug, Serialize)]
pub struct VectorResult {
    pub metric: HashMap<String, String>,
    pub value: (f64, String),
}

/// Params for /loki/api/v1/query_range
#[derive(Debug, Deserialize)]
pub struct QueryRangeParams {
    pub query: String,
    pub start: Option<String>,
    pub end: Option<String>,
    pub step: Option<String>,
    pub interval: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub direction: Direction,
}

fn default_limit() -> usize { 100 }

/// Params for /loki/api/v1/query (instant)
#[derive(Debug, Deserialize)]
pub struct QueryParams {
    pub query: String,
    pub time: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub direction: Direction,
}

/// Params for /loki/api/v1/labels and /loki/api/v1/label/{name}/values
#[derive(Debug, Deserialize)]
pub struct LabelParams {
    pub start: Option<String>,
    pub end: Option<String>,
    pub query: Option<String>,
}

/// Params for /loki/api/v1/series
#[derive(Debug, Deserialize)]
pub struct SeriesParams {
    #[serde(rename = "match[]")]
    pub matchers: Option<Vec<String>>,
    pub start: Option<String>,
    pub end: Option<String>,
}

/// Index statistics response.
#[derive(Debug, Serialize, Default)]
pub struct IndexStats {
    pub streams: u64,
    pub chunks: u64,
    pub entries: u64,
    pub bytes: u64,
}

/// Per-tenant limits configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantLimits {
    /// Maximum ingestion rate in bytes/sec.
    pub ingestion_rate_bytes: u64,
    /// Maximum burst size in bytes.
    pub ingestion_burst_bytes: u64,
    /// Maximum number of active streams.
    pub max_streams: u64,
    /// Maximum number of entries per query.
    pub max_entries_per_query: usize,
    /// Maximum query time range in hours.
    pub max_query_range_hours: u64,
    /// Maximum log line length in bytes (0 = unlimited).
    pub max_line_size: usize,
    /// Retention duration in hours (0 = global default).
    pub retention_hours: u64,
}

impl Default for TenantLimits {
    fn default() -> Self {
        Self {
            ingestion_rate_bytes: 4 * 1024 * 1024,   // 4 MB/s
            ingestion_burst_bytes: 16 * 1024 * 1024, // 16 MB
            max_streams: 10_000,
            max_entries_per_query: 5_000,
            max_query_range_hours: 24 * 30, // 30 days
            max_line_size: 256 * 1024,      // 256 KB
            retention_hours: 24 * 7,        // 7 days
        }
    }
}

/// Tail filter parameters (WebSocket).
#[derive(Debug, Deserialize)]
pub struct TailParams {
    pub query: String,
    #[serde(default = "default_delay_for")]
    pub delay_for: u64,
}

fn default_delay_for() -> u64 { 0 }

/// Single entry in a tail response.
#[derive(Debug, Serialize, Clone)]
pub struct TailEntry {
    pub ts: String,
    pub line: String,
    pub labels: HashMap<String, String>,
}

/// WebSocket tail push message.
#[derive(Debug, Serialize, Clone)]
pub struct TailResponse {
    pub streams: Vec<TailStream>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dropped_entries: Vec<DroppedEntry>,
}

#[derive(Debug, Serialize, Clone)]
pub struct TailStream {
    pub stream: HashMap<String, String>,
    pub values: Vec<(String, String)>,
}

#[derive(Debug, Serialize, Clone)]
pub struct DroppedEntry {
    pub labels: HashMap<String, String>,
    pub timestamp: String,
}

/// Broadcast event pushed to WebSocket subscribers.
#[derive(Debug, Clone)]
pub struct TailEvent {
    pub tenant: TenantId,
    pub stream_labels: Labels,
    pub entry: LogEntry,
}
