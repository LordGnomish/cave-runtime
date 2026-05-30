// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Kafka span marshaller codec — line-port of jaeger
//! `plugin/storage/kafka/{marshaller,unmarshaller,writer}.go`, pinned v1.52.0.
//!
//! The collector ships spans onto a Kafka topic for the ingester to drain.
//! The JSON [`Marshaller`](https://github.com/jaegertracing/jaeger) serialises
//! a `model.Span` in the jsonpb representation:
//!
//!   * `traceID` / `spanID` as `model.TraceID.String()` / `SpanID.String()`
//!     lowercase hex,
//!   * `references` as `[{refType, traceID, spanID}]` (a `parentSpanID`
//!     becomes a single `CHILD_OF`),
//!   * `startTime` as an RFC3339 timestamp and `duration` as a
//!     fractional-seconds string (`google.protobuf.Duration` jsonpb form),
//!   * `tags` / `process.tags` as `vType`-discriminated key/value objects
//!     (`STRING`→`vStr`, `BOOL`→`vBool`, `INT64`→`vInt64` (string-encoded),
//!     `FLOAT64`→`vFloat64`, `BINARY`→`vBinary` (base64)).
//!
//! The producer keys each message by `span.TraceID.String()` so every span of
//! a trace lands on the same partition (in-trace ordering) — see
//! [`KafkaSpanCodec::partition_key`].
//!
//! The live sarama AsyncProducer + the ingester consumer group + the protobuf
//! (binary) marshaller stay scope_cut (operational-storage-backends, Phase 3
//! cave-streams bridge); this is the pure marshal/unmarshal codec a writer
//! invokes before handing bytes to the transport.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::storage_es::{span_id_string, trace_id_string, CHILD_OF, FOLLOWS_FROM};
use crate::types::{Span, SpanId, SpanKind, SpanStatus, TagValue, TraceId};
use crate::{Result, TraceError};

// ─── jsonpb vType discriminators ─────────────────────────────────────────────

const VTYPE_STRING: &str = "STRING";
const VTYPE_BOOL: &str = "BOOL";
const VTYPE_INT64: &str = "INT64";
const VTYPE_FLOAT64: &str = "FLOAT64";
const VTYPE_BINARY: &str = "BINARY";

// ─── Wire document (jsonpb model.Span) ───────────────────────────────────────

/// jsonpb `model.KeyValue` — a `vType`-discriminated tag.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct KafkaKeyValue {
    key: String,
    #[serde(rename = "vType", default, skip_serializing_if = "String::is_empty")]
    v_type: String,
    #[serde(rename = "vStr", default, skip_serializing_if = "Option::is_none")]
    v_str: Option<String>,
    #[serde(rename = "vBool", default, skip_serializing_if = "Option::is_none")]
    v_bool: Option<bool>,
    /// jsonpb renders int64 as a quoted string.
    #[serde(rename = "vInt64", default, skip_serializing_if = "Option::is_none")]
    v_int64: Option<String>,
    #[serde(rename = "vFloat64", default, skip_serializing_if = "Option::is_none")]
    v_float64: Option<f64>,
    /// jsonpb renders bytes as base64.
    #[serde(rename = "vBinary", default, skip_serializing_if = "Option::is_none")]
    v_binary: Option<String>,
}

impl KafkaKeyValue {
    fn from_tag(key: &str, v: &TagValue) -> Self {
        let mut kv = KafkaKeyValue {
            key: key.to_owned(),
            v_type: String::new(),
            v_str: None,
            v_bool: None,
            v_int64: None,
            v_float64: None,
            v_binary: None,
        };
        match v {
            TagValue::String(s) => {
                kv.v_type = VTYPE_STRING.into();
                kv.v_str = Some(s.clone());
            }
            TagValue::Bool(b) => {
                kv.v_type = VTYPE_BOOL.into();
                kv.v_bool = Some(*b);
            }
            TagValue::Int(i) => {
                kv.v_type = VTYPE_INT64.into();
                kv.v_int64 = Some(i.to_string());
            }
            TagValue::Float(f) => {
                kv.v_type = VTYPE_FLOAT64.into();
                kv.v_float64 = Some(*f);
            }
            TagValue::Binary(b) => {
                use base64::{engine::general_purpose::STANDARD, Engine as _};
                kv.v_type = VTYPE_BINARY.into();
                kv.v_binary = Some(STANDARD.encode(b));
            }
        }
        kv
    }

    fn to_tag(&self) -> (String, TagValue) {
        let value = match self.v_type.as_str() {
            VTYPE_BOOL => TagValue::Bool(self.v_bool.unwrap_or(false)),
            VTYPE_INT64 => TagValue::Int(
                self.v_int64
                    .as_deref()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0),
            ),
            VTYPE_FLOAT64 => TagValue::Float(self.v_float64.unwrap_or(0.0)),
            VTYPE_BINARY => {
                use base64::{engine::general_purpose::STANDARD, Engine as _};
                let bytes = self
                    .v_binary
                    .as_deref()
                    .and_then(|s| STANDARD.decode(s).ok())
                    .unwrap_or_default();
                TagValue::Binary(bytes)
            }
            // STRING and unknown fall back to string
            _ => TagValue::String(self.v_str.clone().unwrap_or_default()),
        };
        (self.key.clone(), value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KafkaReference {
    #[serde(rename = "refType")]
    ref_type: String,
    #[serde(rename = "traceID")]
    trace_id: String,
    #[serde(rename = "spanID")]
    span_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct KafkaProcess {
    #[serde(rename = "serviceName")]
    service_name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<KafkaKeyValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KafkaSpanDoc {
    #[serde(rename = "traceID")]
    trace_id: String,
    #[serde(rename = "spanID")]
    span_id: String,
    #[serde(rename = "operationName")]
    operation_name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    references: Vec<KafkaReference>,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    flags: u32,
    #[serde(rename = "startTime")]
    start_time: String,
    duration: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<KafkaKeyValue>,
    process: KafkaProcess,
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

// ─── Codec ────────────────────────────────────────────────────────────────────

/// Stateless Kafka span JSON marshaller / unmarshaller.
pub struct KafkaSpanCodec;

impl KafkaSpanCodec {
    /// `jsonMarshaller.Marshal` — encode a domain [`Span`] as topic bytes.
    pub fn marshal_json(span: &Span) -> Vec<u8> {
        let mut references = Vec::new();
        if let Some(parent) = span.parent_span_id {
            references.push(KafkaReference {
                ref_type: CHILD_OF.to_owned(),
                trace_id: trace_id_string(span.trace_id),
                span_id: span_id_string(parent),
            });
        }
        for link in &span.links {
            references.push(KafkaReference {
                ref_type: FOLLOWS_FROM.to_owned(),
                trace_id: trace_id_string(link.trace_id),
                span_id: span_id_string(link.span_id),
            });
        }

        let doc = KafkaSpanDoc {
            trace_id: trace_id_string(span.trace_id),
            span_id: span_id_string(span.span_id),
            operation_name: span.operation_name.clone(),
            references,
            flags: 0,
            start_time: format_rfc3339_nanos(span.start_time_unix_nano),
            duration: format_proto_duration(span.duration_ns),
            tags: span
                .tags
                .iter()
                .map(|(k, v)| KafkaKeyValue::from_tag(k, v))
                .collect(),
            process: KafkaProcess {
                service_name: span.service_name.clone(),
                tags: span
                    .resource_attributes
                    .iter()
                    .map(|(k, v)| KafkaKeyValue::from_tag(k, v))
                    .collect(),
            },
        };
        serde_json::to_vec(&doc).unwrap_or_default()
    }

    /// `jsonUnmarshaller.Unmarshal` — decode topic bytes back to a [`Span`].
    pub fn unmarshal_json(bytes: &[u8]) -> Result<Span> {
        let doc: KafkaSpanDoc = serde_json::from_slice(bytes)
            .map_err(|e| TraceError::ParseError(format!("kafka span json: {}", e)))?;

        let trace_id = parse_hex_u128(&doc.trace_id)
            .ok_or_else(|| TraceError::InvalidTraceId(doc.trace_id.clone(), "not hex".into()))?;
        let span_id = parse_hex_u64(&doc.span_id)
            .ok_or_else(|| TraceError::InvalidSpanId(doc.span_id.clone(), "not hex".into()))?;

        // First CHILD_OF reference becomes the parent.
        let parent_span_id = doc
            .references
            .iter()
            .find(|r| r.ref_type == CHILD_OF)
            .and_then(|r| parse_hex_u64(&r.span_id));
        let links = doc
            .references
            .iter()
            .filter(|r| r.ref_type == FOLLOWS_FROM)
            .filter_map(|r| {
                Some(crate::types::SpanLink {
                    trace_id: parse_hex_u128(&r.trace_id)?,
                    span_id: parse_hex_u64(&r.span_id)?,
                    trace_state: String::new(),
                    attributes: HashMap::new(),
                })
            })
            .collect();

        let duration_ns = parse_proto_duration(&doc.duration).unwrap_or(0);
        let start_ns = parse_rfc3339_nanos(&doc.start_time).unwrap_or(0);

        let tags: HashMap<String, TagValue> = doc.tags.iter().map(|t| t.to_tag()).collect();
        let resource_attributes: HashMap<String, TagValue> =
            doc.process.tags.iter().map(|t| t.to_tag()).collect();

        let status = if tags
            .get("error")
            .map(|v| v.display() != "false")
            .unwrap_or(false)
        {
            SpanStatus::Error
        } else {
            SpanStatus::Unset
        };

        Ok(Span {
            trace_id,
            span_id,
            parent_span_id,
            operation_name: doc.operation_name,
            service_name: doc.process.service_name,
            start_time_unix_nano: start_ns,
            end_time_unix_nano: start_ns.saturating_add(duration_ns),
            duration_ns,
            status,
            kind: SpanKind::Internal,
            tags,
            events: vec![],
            links,
            resource_attributes,
            tenant_id: "default".into(),
            baggage: HashMap::new(),
            log_labels: HashMap::new(),
        })
    }

    /// `writer.go` partition key — `span.TraceID.String()`. Keying by trace ID
    /// keeps a trace's spans on one partition for ordered consumption.
    pub fn partition_key(span: &Span) -> String {
        trace_id_string(span.trace_id)
    }
}

// ─── google.protobuf.Duration jsonpb formatting ──────────────────────────────

/// Format nanoseconds as the jsonpb `Duration` string: an integer-second
/// `"<s>s"` when there is no fractional part, otherwise `"<s>.<frac>s"` with
/// the fraction trimmed of trailing zeros and padded up to a group of three
/// digits (3/6/9), mirroring `protojson`.
pub fn format_proto_duration(ns: u64) -> String {
    let secs = ns / 1_000_000_000;
    let nanos = ns % 1_000_000_000;
    if nanos == 0 {
        return format!("{}s", secs);
    }
    let frac = format!("{:09}", nanos);
    let trimmed_len = frac.trim_end_matches('0').len().max(1);
    let group_len = trimmed_len.div_ceil(3) * 3; // round up to 3/6/9
    format!("{}.{}s", secs, &frac[..group_len])
}

/// Parse a jsonpb `Duration` string back to nanoseconds.
pub fn parse_proto_duration(s: &str) -> Option<u64> {
    let body = s.strip_suffix('s').unwrap_or(s);
    let (secs_str, frac_str) = match body.split_once('.') {
        Some((a, b)) => (a, b),
        None => (body, ""),
    };
    let secs: u64 = secs_str.parse().ok()?;
    let frac_ns: u64 = if frac_str.is_empty() {
        0
    } else {
        let mut padded = frac_str.to_owned();
        while padded.len() < 9 {
            padded.push('0');
        }
        padded.truncate(9);
        padded.parse().ok()?
    };
    Some(secs.saturating_mul(1_000_000_000).saturating_add(frac_ns))
}

// ─── RFC3339 timestamp (jsonpb google.protobuf.Timestamp) ────────────────────

fn format_rfc3339_nanos(start_unix_nano: u64) -> String {
    let secs = (start_unix_nano / 1_000_000_000) as i64;
    let nanos = (start_unix_nano % 1_000_000_000) as u32;
    chrono::DateTime::from_timestamp(secs, nanos)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
}

fn parse_rfc3339_nanos(s: &str) -> Option<u64> {
    let dt = chrono::DateTime::parse_from_rfc3339(s).ok()?;
    let secs = dt.timestamp();
    let nanos = dt.timestamp_subsec_nanos();
    if secs < 0 {
        return Some(0);
    }
    Some((secs as u64).saturating_mul(1_000_000_000).saturating_add(nanos as u64))
}

// ─── hex id parsing ───────────────────────────────────────────────────────────

fn parse_hex_u128(s: &str) -> Option<TraceId> {
    u128::from_str_radix(s.trim(), 16).ok()
}

fn parse_hex_u64(s: &str) -> Option<SpanId> {
    u64::from_str_radix(s.trim(), 16).ok()
}
