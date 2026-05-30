// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Elasticsearch storage db-model codec — line-port of jaeger
//! `plugin/storage/es/spanstore/dbmodel/` (model.go + from_domain.go),
//! pinned v1.52.0.
//!
//! The **pure** span → ES-document encoder. A domain [`Span`] becomes an
//! [`EsSpan`] whose identifiers are the canonical `model.TraceID.String()` /
//! `SpanID.String()` hex renderings, whose `start_time` / `duration` are in
//! microseconds with the dedicated `start_time_millis` routing field, whose
//! references use the UPPERCASE `CHILD_OF` / `FOLLOWS_FROM` names, whose tags
//! live both as a structured `tags` array and (when `all_tags_as_fields` or a
//! `tag_keys_as_fields` allow-list opts them in) a Kibana-friendly flattened
//! `tag` map with dots replaced by `@` and binary values dropped. Index
//! routing is the date-rotated `[prefix-]jaeger-span-YYYY-MM-DD` name.
//!
//! The live ES HTTP bulk client + index templates + ILM rollover stay
//! scope_cut (operational-storage-backends, Phase 3 cave-search); this is the
//! document codec a writer serialises before issuing the bulk request.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::storage_cassandra::{
    BINARY_TYPE, BOOL_TYPE, FLOAT64_TYPE, INT64_TYPE, STRING_TYPE,
};
use crate::types::{Span, TagValue};

// ─── Index naming + reference + dot-replacement constants ───────────────────

/// ES span index base name (`jaeger-span-` + date).
pub const SPAN_INDEX_BASE: &str = "jaeger-span-";
/// ES service index base name (`jaeger-service-` + date).
pub const SERVICE_INDEX_BASE: &str = "jaeger-service-";
/// ES dependencies index base name (`jaeger-dependencies-` + date).
pub const DEPENDENCY_INDEX_BASE: &str = "jaeger-dependencies-";

/// ES reference type for a parent edge.
pub const CHILD_OF: &str = "CHILD_OF";
/// ES reference type for a follows-from edge.
pub const FOLLOWS_FROM: &str = "FOLLOWS_FROM";

/// Default `--es.tags-as-fields.dot-replacement`.
pub const DEFAULT_TAG_DOT_REPLACEMENT: char = '@';

// ─── Config ──────────────────────────────────────────────────────────────────

/// ES dbmodel encoding options (subset of the `--es.*` flags that influence
/// the document shape).
#[derive(Debug, Clone)]
pub struct EsConfig {
    /// Promote every non-binary tag into the flattened `tag` map.
    pub all_tags_as_fields: bool,
    /// Explicit allow-list of tag keys to promote into the `tag` map.
    pub tag_keys_as_fields: Vec<String>,
    /// Character that replaces `.` in flattened tag-map keys.
    pub tag_dot_replacement: char,
    /// Optional index prefix (joined with `-`).
    pub index_prefix: Option<String>,
}

impl Default for EsConfig {
    fn default() -> Self {
        EsConfig {
            all_tags_as_fields: false,
            tag_keys_as_fields: Vec::new(),
            tag_dot_replacement: DEFAULT_TAG_DOT_REPLACEMENT,
            index_prefix: None,
        }
    }
}

// ─── Document model (model.go) ──────────────────────────────────────────────

/// ES dbmodel `KeyValue` — `{key, type, value}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EsKeyValue {
    pub key: String,
    #[serde(rename = "type")]
    pub value_type: String,
    pub value: Value,
}

impl EsKeyValue {
    fn from_tag(key: &str, v: &TagValue) -> Self {
        let (value_type, value) = match v {
            TagValue::String(s) => (STRING_TYPE, Value::String(s.clone())),
            TagValue::Bool(b) => (BOOL_TYPE, Value::Bool(*b)),
            TagValue::Int(i) => (INT64_TYPE, Value::from(*i)),
            TagValue::Float(f) => (FLOAT64_TYPE, Value::from(*f)),
            TagValue::Binary(b) => {
                use base64::{engine::general_purpose::STANDARD, Engine as _};
                (BINARY_TYPE, Value::String(STANDARD.encode(b)))
            }
        };
        EsKeyValue {
            key: key.to_owned(),
            value_type: value_type.to_owned(),
            value,
        }
    }
}

/// ES dbmodel `Reference` — `{refType, traceID, spanID}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EsReference {
    #[serde(rename = "refType")]
    pub ref_type: String,
    #[serde(rename = "traceID")]
    pub trace_id: String,
    #[serde(rename = "spanID")]
    pub span_id: String,
}

/// ES dbmodel `Process` — `{serviceName, tags, tag}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EsProcess {
    #[serde(rename = "serviceName")]
    pub service_name: String,
    pub tags: Vec<EsKeyValue>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub tag: BTreeMap<String, Value>,
}

/// ES dbmodel `Span` document.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EsSpan {
    #[serde(rename = "traceID")]
    pub trace_id: String,
    #[serde(rename = "spanID")]
    pub span_id: String,
    #[serde(rename = "operationName")]
    pub operation_name: String,
    pub references: Vec<EsReference>,
    #[serde(skip_serializing_if = "is_zero", default)]
    pub flags: u32,
    #[serde(rename = "startTime")]
    pub start_time: u64,
    #[serde(rename = "startTimeMillis")]
    pub start_time_millis: u64,
    pub duration: u64,
    pub tags: Vec<EsKeyValue>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub tag: BTreeMap<String, Value>,
    pub process: EsProcess,
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

impl EsSpan {
    /// `FromDomain` — encode a domain [`Span`] into its ES document form.
    pub fn from_domain(span: &Span, cfg: &EsConfig) -> Self {
        let tags: Vec<EsKeyValue> = span
            .tags
            .iter()
            .map(|(k, v)| EsKeyValue::from_tag(k, v))
            .collect();
        let tag = build_tag_map(&span.tags, cfg);

        let process_tags: Vec<EsKeyValue> = span
            .resource_attributes
            .iter()
            .map(|(k, v)| EsKeyValue::from_tag(k, v))
            .collect();
        let process_tag = build_tag_map(&span.resource_attributes, cfg);

        let mut references = Vec::new();
        if let Some(parent) = span.parent_span_id {
            references.push(EsReference {
                ref_type: CHILD_OF.to_owned(),
                trace_id: trace_id_string(span.trace_id),
                span_id: span_id_string(parent),
            });
        }
        for link in &span.links {
            references.push(EsReference {
                ref_type: FOLLOWS_FROM.to_owned(),
                trace_id: trace_id_string(link.trace_id),
                span_id: span_id_string(link.span_id),
            });
        }

        let start_micros = span.start_time_unix_nano / 1_000;

        EsSpan {
            trace_id: trace_id_string(span.trace_id),
            span_id: span_id_string(span.span_id),
            operation_name: span.operation_name.clone(),
            references,
            flags: 0,
            start_time: start_micros,
            start_time_millis: start_micros / 1_000,
            duration: span.duration_ns / 1_000,
            tags,
            tag,
            process: EsProcess {
                service_name: span.service_name.clone(),
                tags: process_tags,
                tag: process_tag,
            },
        }
    }
}

/// Build the flattened Kibana `tag` map: include a tag iff it is non-binary
/// and (`all_tags_as_fields` || key ∈ `tag_keys_as_fields`); replace dots in
/// the key with `tag_dot_replacement`.
fn build_tag_map(
    tags: &std::collections::HashMap<String, TagValue>,
    cfg: &EsConfig,
) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    for (k, v) in tags {
        if matches!(v, TagValue::Binary(_)) {
            continue;
        }
        let included = cfg.all_tags_as_fields || cfg.tag_keys_as_fields.iter().any(|t| t == k);
        if !included {
            continue;
        }
        let field = k.replace('.', &cfg.tag_dot_replacement.to_string());
        out.insert(field, EsKeyValue::from_tag(k, v).value);
    }
    out
}

// ─── Identifier rendering (model.TraceID/SpanID.String) ─────────────────────

/// `model.TraceID.String()` — `%016x%016x`, dropping the high word when zero.
pub fn trace_id_string(id: u128) -> String {
    let high = (id >> 64) as u64;
    let low = id as u64;
    if high == 0 {
        format!("{:016x}", low)
    } else {
        format!("{:016x}{:016x}", high, low)
    }
}

/// `model.SpanID.String()` — `%016x`.
pub fn span_id_string(id: u64) -> String {
    format!("{:016x}", id)
}

// ─── Date-rotated index naming (spanstore writer) ───────────────────────────

/// Build a date-rotated index name: `[prefix-]base + YYYY-MM-DD` in UTC from
/// the span's start time.
pub fn index_name(base: &str, start_time_unix_nano: u64, prefix: Option<&str>) -> String {
    let secs = (start_time_unix_nano / 1_000_000_000) as i64;
    let nanos = (start_time_unix_nano % 1_000_000_000) as u32;
    let date = chrono::DateTime::from_timestamp(secs, nanos)
        .unwrap_or_default()
        .format("%Y-%m-%d");
    match prefix {
        Some(p) if !p.is_empty() => format!("{}-{}{}", p, base, date),
        _ => format!("{}{}", base, date),
    }
}
