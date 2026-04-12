//! OTLP Logs ingestion — HTTP/JSON format.
//!
//! Implements the OpenTelemetry Protocol log format as received via:
//!   POST /otlp/v1/logs   (HTTP JSON)
//!
//! The full gRPC path would require tonic + generated proto stubs; we handle
//! the JSON/HTTP path here, which is spec-equivalent and avoids the build.rs
//! dependency for the proto-generated code.
//!
//! OTLP JSON schema:
//!   ExportLogsServiceRequest {
//!     resource_logs: [
//!       ResourceLogs {
//!         resource: { attributes: [{key,value},...] },
//!         scope_logs: [
//!           ScopeLogs {
//!             scope: { name, version },
//!             log_records: [ LogRecord { ... } ]
//!           }
//!         ]
//!       }
//!     ]
//!   }

use std::collections::HashMap;
use std::sync::Arc;

use serde::Deserialize;

use crate::models::{Labels, LogEntry, TimestampNs};
use crate::store::LogStore;

// ── JSON deserialization types ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportLogsServiceRequest {
    #[serde(default)]
    pub resource_logs: Vec<ResourceLogs>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceLogs {
    #[serde(default)]
    pub resource: Option<Resource>,
    #[serde(default)]
    pub scope_logs: Vec<ScopeLogs>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resource {
    #[serde(default)]
    pub attributes: Vec<KeyValue>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeLogs {
    #[serde(default)]
    pub scope: Option<InstrumentationScope>,
    #[serde(default)]
    pub log_records: Vec<LogRecord>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstrumentationScope {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

/// A single OTLP log record.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogRecord {
    /// Unix epoch nanoseconds as string (OTLP JSON uses string for u64).
    #[serde(default)]
    pub time_unix_nano: Option<String>,
    /// Severity number (1-24).
    #[serde(default)]
    pub severity_number: Option<u32>,
    /// Severity text label.
    #[serde(default)]
    pub severity_text: Option<String>,
    /// The log body.
    #[serde(default)]
    pub body: Option<AnyValue>,
    /// Per-record attributes.
    #[serde(default)]
    pub attributes: Vec<KeyValue>,
    /// Trace context.
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub span_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyValue {
    pub key: String,
    pub value: AnyValue,
}

/// OTLP AnyValue — simplified to the types we care about for log bodies.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum AnyValue {
    String { #[serde(rename = "stringValue")] string_value: String },
    Bool { #[serde(rename = "boolValue")] bool_value: bool },
    Int { #[serde(rename = "intValue")] int_value: serde_json::Value },
    Double { #[serde(rename = "doubleValue")] double_value: f64 },
    Object(serde_json::Value),
}

impl AnyValue {
    pub fn as_string(&self) -> String {
        match self {
            AnyValue::String { string_value } => string_value.clone(),
            AnyValue::Bool { bool_value } => bool_value.to_string(),
            AnyValue::Int { int_value } => int_value.to_string(),
            AnyValue::Double { double_value } => double_value.to_string(),
            AnyValue::Object(v) => v.to_string(),
        }
    }
}

/// Convert a `KeyValue` slice into a flat `HashMap<String, String>`.
fn kv_to_map(kvs: &[KeyValue]) -> HashMap<String, String> {
    kvs.iter().map(|kv| (kv.key.clone(), kv.value.as_string())).collect()
}

// ── Ingestion ─────────────────────────────────────────────────────────────────

/// Parse and ingest an OTLP JSON export request.
pub fn ingest_otlp_json(body: &[u8], tenant: &str, store: &Arc<LogStore>) -> anyhow::Result<usize> {
    let req: ExportLogsServiceRequest = serde_json::from_slice(body)?;
    let mut total = 0usize;

    for resource_log in req.resource_logs {
        // Resource attributes become stream labels.
        let resource_attrs: HashMap<String, String> = resource_log
            .resource
            .map(|r| kv_to_map(&r.attributes))
            .unwrap_or_default();

        for scope_log in resource_log.scope_logs {
            // Scope name/version become additional labels.
            let mut stream_labels = resource_attrs.clone();
            if let Some(scope) = &scope_log.scope {
                if let Some(name) = &scope.name {
                    if !name.is_empty() { stream_labels.insert("otel_scope_name".into(), name.clone()); }
                }
                if let Some(version) = &scope.version {
                    if !version.is_empty() { stream_labels.insert("otel_scope_version".into(), version.clone()); }
                }
            }

            let labels = Labels::new(stream_labels.clone());
            let mut entries = Vec::new();

            for record in scope_log.log_records {
                let ts_ns: TimestampNs = record
                    .time_unix_nano
                    .as_deref()
                    .and_then(|s| s.parse::<i64>().ok())
                    .unwrap_or_else(|| chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0));

                let line = record.body.map(|b| b.as_string()).unwrap_or_default();

                // Per-record attributes become entry metadata.
                let mut meta = kv_to_map(&record.attributes);
                if let Some(sev_text) = record.severity_text {
                    if !sev_text.is_empty() { meta.insert("severity".into(), sev_text); }
                }
                if let Some(sev_num) = record.severity_number {
                    meta.insert("severity_number".into(), sev_num.to_string());
                }
                if let Some(trace_id) = record.trace_id {
                    if !trace_id.is_empty() && trace_id != "00000000000000000000000000000000" {
                        meta.insert("trace_id".into(), trace_id);
                    }
                }
                if let Some(span_id) = record.span_id {
                    if !span_id.is_empty() && span_id != "0000000000000000" {
                        meta.insert("span_id".into(), span_id);
                    }
                }

                entries.push(LogEntry { ts: ts_ns, line, metadata: meta });
            }

            total += entries.len();
            if !entries.is_empty() {
                store.push(tenant, labels, entries)?;
            }
        }
    }

    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::LogStore;
    use crate::models::Direction;

    #[test]
    fn ingest_otlp_json_basic() {
        let store = LogStore::new();
        let body = serde_json::json!({
            "resourceLogs": [{
                "resource": {
                    "attributes": [
                        {"key": "service.name", "value": {"stringValue": "my-service"}}
                    ]
                },
                "scopeLogs": [{
                    "scope": {"name": "my-lib", "version": "1.0"},
                    "logRecords": [{
                        "timeUnixNano": "1000000000",
                        "severityText": "INFO",
                        "severityNumber": 9,
                        "body": {"stringValue": "hello from otlp"},
                        "attributes": [
                            {"key": "http.method", "value": {"stringValue": "GET"}}
                        ]
                    }]
                }]
            }]
        });

        let n = ingest_otlp_json(&serde_json::to_vec(&body).unwrap(), "tenant", &store).unwrap();
        assert_eq!(n, 1);

        let fps = store.matching_fps("tenant", |_| true);
        assert_eq!(fps.len(), 1);
        let results = store.query_entries("tenant", &fps, 0, i64::MAX, 10, Direction::Forward);
        assert_eq!(results[0].2[0].line, "hello from otlp");
        assert_eq!(results[0].2[0].metadata.get("http.method").map(|s| s.as_str()), Some("GET"));
        assert_eq!(results[0].2[0].metadata.get("severity").map(|s| s.as_str()), Some("INFO"));
    }
}
