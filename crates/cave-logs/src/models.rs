//! Data model: streams (label sets + entries), push/query API types, alerting.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ─── Label set ───────────────────────────────────────────────────────────────

/// A set of key/value labels that uniquely identify a log stream.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Labels(pub HashMap<String, String>);

impl Labels {
    pub fn new(map: HashMap<String, String>) -> Self {
        Self(map)
    }

    /// Deterministic fingerprint (sorted keys).
    pub fn fingerprint(&self) -> u64 {
        self.fingerprint_with_tenant(None)
    }

    /// Fingerprint scoped to a tenant so identical labels in different tenants
    /// are stored as distinct streams.
    pub fn fingerprint_with_tenant(&self, tenant: Option<&str>) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut pairs: Vec<_> = self.0.iter().collect();
        pairs.sort_by_key(|(k, _)| k.as_str());
        let mut h = DefaultHasher::new();
        tenant.hash(&mut h);
        for (k, v) in &pairs {
            k.hash(&mut h);
            v.hash(&mut h);
        }
        h.finish()
    }

    /// Loki-style selector string: `{app="foo", env="prod"}`.
    pub fn to_selector(&self) -> String {
        let mut pairs: Vec<_> = self.0.iter().collect();
        pairs.sort_by_key(|(k, _)| k.as_str());
        let inner = pairs
            .iter()
            .map(|(k, v)| format!("{}=\"{}\"", k, v))
            .collect::<Vec<_>>()
            .join(", ");
        format!("{{{}}}", inner)
    }

    /// Check if this label set satisfies all matchers.
    pub fn matches(&self, matchers: &[LabelMatcher]) -> bool {
        matchers.iter().all(|m| m.matches_opt(self.0.get(&m.name).map(|s| s.as_str())))
    }

    /// Merge extracted labels (from parser stages) into a cloned label set.
    pub fn merged(&self, extra: &HashMap<String, String>) -> Labels {
        let mut m = self.0.clone();
        m.extend(extra.iter().map(|(k, v)| (k.clone(), v.clone())));
        Labels(m)
    }
}

impl std::hash::Hash for Labels {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let mut pairs: Vec<_> = self.0.iter().collect();
        pairs.sort_by_key(|(k, _)| k.as_str());
        for (k, v) in pairs {
            k.hash(state);
            v.hash(state);
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

#[derive(Debug, Serialize)]
#[serde(tag = "resultType", rename_all = "camelCase")]
pub enum QueryData {
    #[serde(rename = "streams")]
    Streams { result: Vec<StreamResult> },
    #[serde(rename = "matrix")]
    Matrix { result: Vec<MatrixResult> },
    #[serde(rename = "vector")]
    Vector { result: Vec<VectorResult> },
}

#[derive(Debug, Clone, Serialize)]
pub struct StreamResult {
    pub stream: HashMap<String, String>,
    /// Each value: `[timestamp_ns_string, log_line]`.
    pub values: Vec<[String; 2]>,
}

#[derive(Debug, Serialize)]
pub struct MatrixResult {
    pub metric: HashMap<String, String>,
    /// Each sample: `[unix_timestamp_float, value_string]`.
    pub values: Vec<(f64, String)>,
}

#[derive(Debug, Serialize)]
pub struct VectorResult {
    pub metric: HashMap<String, String>,
    pub value: (f64, String),
}

// ─── Tail API ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct TailResponse {
    pub streams: Vec<StreamResult>,
    pub dropped_entries: Option<Vec<DroppedEntry>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DroppedEntry {
    pub timestamp: String,
    pub labels: String,
}

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertCondition {
    pub op: CompareOp,
    pub threshold: f64,
}

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
