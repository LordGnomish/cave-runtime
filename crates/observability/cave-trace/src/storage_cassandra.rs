// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cassandra storage db-model codec — line-port of jaeger
//! `plugin/storage/cassandra/spanstore/dbmodel/` (model.go + converter.go +
//! tag_filter.go), pinned v1.52.0.
//!
//! This is the **pure** span ↔ Cassandra-schema conversion layer, the
//! algorithmic heart of Jaeger's Cassandra storage plugin. A domain [`Span`]
//! is encoded into a [`DbSpan`] whose `start_time` / `duration` are in
//! **microseconds** (`model.TimeAsEpochMicroseconds` /
//! `model.DurationAsMicroseconds`), whose trace identifier is the canonical
//! 16-byte big-endian blob, whose tags become typed [`DbKeyValue`]s carrying
//! the upstream value-type strings, and whose index-tag set is produced by
//! [`get_all_unique_tags`] exactly as `GetAllUniqueTags` does: combine
//! process + span + log tags, sort, skip binary-typed values, dedupe adjacent
//! identical entries, and emit `TagInsertion{service, key, AsString()}`.
//!
//! The operational surface — the gocql session, the CQL DDL, the
//! service/operation/duration index *table writes* — is deliberately
//! scope_cut (operational-storage-backends, Phase 3 cave-store). What lives
//! here is the encoder a Cassandra writer would call before issuing CQL.

use crate::types::{Span, SpanId, TagValue};

// ─── Value-type + reference-type constants (dbmodel/model.go) ───────────────

/// Canonical Cassandra dbmodel value-type tag for string values.
pub const STRING_TYPE: &str = "string";
/// Canonical Cassandra dbmodel value-type tag for boolean values.
pub const BOOL_TYPE: &str = "bool";
/// Canonical Cassandra dbmodel value-type tag for 64-bit integer values.
pub const INT64_TYPE: &str = "int64";
/// Canonical Cassandra dbmodel value-type tag for 64-bit float values.
pub const FLOAT64_TYPE: &str = "float64";
/// Canonical Cassandra dbmodel value-type tag for binary values.
pub const BINARY_TYPE: &str = "binary";

/// Cassandra dbmodel reference type for `CHILD_OF`.
pub const CHILD_OF: &str = "child-of";
/// Cassandra dbmodel reference type for `FOLLOWS_FROM`.
pub const FOLLOWS_FROM: &str = "follows-from";

// ─── DbKeyValue ──────────────────────────────────────────────────────────────

/// A Cassandra dbmodel `KeyValue` — a tag with a discriminated value type.
#[derive(Debug, Clone, PartialEq)]
pub struct DbKeyValue {
    pub key: String,
    pub value_type: &'static str,
    pub value_string: String,
    pub value_bool: bool,
    pub value_int64: i64,
    pub value_float64: f64,
    pub value_binary: Vec<u8>,
}

impl DbKeyValue {
    /// Encode a domain [`TagValue`] into its dbmodel `KeyValue` form.
    pub fn from_tag(key: &str, value: &TagValue) -> Self {
        let mut kv = DbKeyValue {
            key: key.to_owned(),
            value_type: STRING_TYPE,
            value_string: String::new(),
            value_bool: false,
            value_int64: 0,
            value_float64: 0.0,
            value_binary: Vec::new(),
        };
        match value {
            TagValue::String(s) => {
                kv.value_type = STRING_TYPE;
                kv.value_string = s.clone();
            }
            TagValue::Bool(b) => {
                kv.value_type = BOOL_TYPE;
                kv.value_bool = *b;
            }
            TagValue::Int(i) => {
                kv.value_type = INT64_TYPE;
                kv.value_int64 = *i;
            }
            TagValue::Float(f) => {
                kv.value_type = FLOAT64_TYPE;
                kv.value_float64 = *f;
            }
            TagValue::Binary(b) => {
                kv.value_type = BINARY_TYPE;
                kv.value_binary = b.clone();
            }
        }
        kv
    }

    /// Reconstruct the domain [`TagValue`] from this dbmodel `KeyValue`.
    pub fn to_tag(&self) -> TagValue {
        match self.value_type {
            BOOL_TYPE => TagValue::Bool(self.value_bool),
            INT64_TYPE => TagValue::Int(self.value_int64),
            FLOAT64_TYPE => TagValue::Float(self.value_float64),
            BINARY_TYPE => TagValue::Binary(self.value_binary.clone()),
            _ => TagValue::String(self.value_string.clone()),
        }
    }

    /// `model.KeyValue.AsString()` — the index-friendly string rendering.
    pub fn as_string(&self) -> String {
        match self.value_type {
            BOOL_TYPE => {
                if self.value_bool {
                    "true".to_owned()
                } else {
                    "false".to_owned()
                }
            }
            INT64_TYPE => self.value_int64.to_string(),
            FLOAT64_TYPE => self.value_float64.to_string(),
            BINARY_TYPE => self.value_binary.iter().map(|b| format!("{:02x}", b)).collect(),
            _ => self.value_string.clone(),
        }
    }
}

// ─── DbProcess ───────────────────────────────────────────────────────────────

/// A Cassandra dbmodel `Process` — service name + process-level tags.
#[derive(Debug, Clone, PartialEq)]
pub struct DbProcess {
    pub service_name: String,
    pub tags: Vec<DbKeyValue>,
}

// ─── TagInsertion ────────────────────────────────────────────────────────────

/// A Cassandra dbmodel `TagInsertion` — one row in the `tag_index` table.
#[derive(Debug, Clone, PartialEq)]
pub struct TagInsertion {
    pub service_name: String,
    pub tag_key: String,
    pub tag_value: String,
}

impl TagInsertion {
    /// `TagInsertion.String()` — colon-joined `service:key:value`.
    pub fn display(&self) -> String {
        format!("{}:{}:{}", self.service_name, self.tag_key, self.tag_value)
    }
}

// ─── DbSpan ──────────────────────────────────────────────────────────────────

/// A Cassandra dbmodel `Span` — the row written to the `traces` table.
#[derive(Debug, Clone, PartialEq)]
pub struct DbSpan {
    /// 16-byte big-endian trace identifier blob.
    pub trace_id: [u8; 16],
    pub span_id: i64,
    pub parent_id: i64,
    pub operation_name: String,
    pub flags: i32,
    /// Epoch microseconds (`model.TimeAsEpochMicroseconds`).
    pub start_time: i64,
    /// Microseconds (`model.DurationAsMicroseconds`).
    pub duration: i64,
    pub tags: Vec<DbKeyValue>,
    pub service_name: String,
    pub process: DbProcess,
    pub span_hash: i64,
}

impl DbSpan {
    /// `FromDomain` — encode a domain [`Span`] into its Cassandra row form.
    pub fn from_domain(span: &Span) -> Self {
        let mut tags = tags_to_dbmodel(&span.tags);
        tags.sort_by(|a, b| a.key.cmp(&b.key));

        let mut process_tags = tags_to_dbmodel(&span.resource_attributes);
        process_tags.sort_by(|a, b| a.key.cmp(&b.key));

        DbSpan {
            trace_id: span.trace_id.to_be_bytes(),
            span_id: span.span_id as i64,
            parent_id: span.parent_span_id.map(|p| p as i64).unwrap_or(0),
            operation_name: span.operation_name.clone(),
            flags: 0,
            start_time: (span.start_time_unix_nano / 1_000) as i64,
            duration: (span.duration_ns / 1_000) as i64,
            tags,
            service_name: span.service_name.clone(),
            process: DbProcess {
                service_name: span.service_name.clone(),
                tags: process_tags,
            },
            span_hash: span_hash(span),
        }
    }

    /// `ToDomain` — decode this Cassandra row back into a domain [`Span`].
    ///
    /// Time fields restore at microsecond granularity (the encoding is lossy
    /// below 1µs, matching Jaeger's Cassandra schema).
    pub fn to_domain(&self) -> Span {
        use std::collections::HashMap;

        let start_ns = (self.start_time as u64) * 1_000;
        let duration_ns = (self.duration as u64) * 1_000;

        let tags: HashMap<String, TagValue> =
            self.tags.iter().map(|kv| (kv.key.clone(), kv.to_tag())).collect();
        let resource_attributes: HashMap<String, TagValue> = self
            .process
            .tags
            .iter()
            .map(|kv| (kv.key.clone(), kv.to_tag()))
            .collect();

        Span {
            trace_id: u128::from_be_bytes(self.trace_id),
            span_id: self.span_id as SpanId,
            parent_span_id: if self.parent_id != 0 {
                Some(self.parent_id as SpanId)
            } else {
                None
            },
            operation_name: self.operation_name.clone(),
            service_name: self.service_name.clone(),
            start_time_unix_nano: start_ns,
            end_time_unix_nano: start_ns + duration_ns,
            duration_ns,
            status: crate::types::SpanStatus::Unset,
            kind: crate::types::SpanKind::Internal,
            tags,
            events: Vec::new(),
            links: Vec::new(),
            resource_attributes,
            tenant_id: "default".to_owned(),
            baggage: HashMap::new(),
            log_labels: HashMap::new(),
        }
    }
}

fn tags_to_dbmodel(tags: &std::collections::HashMap<String, TagValue>) -> Vec<DbKeyValue> {
    tags.iter().map(|(k, v)| DbKeyValue::from_tag(k, v)).collect()
}

/// Deterministic 64-bit span hash (`model.Span.Hash`-equivalent identity used
/// for the `span_hash` clustering column — distinguishes re-emitted spans
/// sharing a (trace_id, span_id)).
fn span_hash(span: &Span) -> i64 {
    // FNV-1a over the identifying fields.
    let mut h: u64 = 0xcbf29ce484222325;
    let mut mix = |bytes: &[u8]| {
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    };
    mix(&span.trace_id.to_be_bytes());
    mix(&span.span_id.to_be_bytes());
    mix(span.operation_name.as_bytes());
    mix(&span.start_time_unix_nano.to_be_bytes());
    mix(&span.duration_ns.to_be_bytes());
    h as i64
}

// ─── GetAllUniqueTags (converter.go + tag_filter.go) ────────────────────────

/// `GetAllUniqueTags` — build the deduplicated, sorted set of tags to index
/// in the `tag_index` table.
///
/// Faithful to jaeger: combine the `DefaultTagFilter` pass-through of process
/// tags + span tags + every log field, sort by `(key, value_type, AsString)`,
/// drop binary-typed values (not indexable), drop adjacent identical entries,
/// and stamp each with the span's process service name.
pub fn get_all_unique_tags(span: &Span) -> Vec<TagInsertion> {
    let mut all: Vec<DbKeyValue> = Vec::new();

    // DefaultTagFilter.FilterProcessTags — process (resource) tags first.
    for (k, v) in &span.resource_attributes {
        all.push(DbKeyValue::from_tag(k, v));
    }
    // DefaultTagFilter.FilterTags — span tags.
    for (k, v) in &span.tags {
        all.push(DbKeyValue::from_tag(k, v));
    }
    // DefaultTagFilter.FilterLogFields — every field of every log/event.
    for event in &span.events {
        for (k, v) in &event.attributes {
            all.push(DbKeyValue::from_tag(k, v));
        }
    }

    // KeyValues.Sort — by key, then value type, then string value.
    all.sort_by(|a, b| {
        a.key
            .cmp(&b.key)
            .then_with(|| a.value_type.cmp(b.value_type))
            .then_with(|| a.as_string().cmp(&b.as_string()))
    });

    let service_name = span.service_name.clone();
    let mut unique: Vec<TagInsertion> = Vec::with_capacity(all.len());
    for (i, kv) in all.iter().enumerate() {
        // Binary tags are not indexed.
        if kv.value_type == BINARY_TYPE {
            continue;
        }
        // Skip adjacent identical (key, type, value).
        if i > 0 {
            let prev = &all[i - 1];
            if prev.key == kv.key
                && prev.value_type == kv.value_type
                && prev.as_string() == kv.as_string()
            {
                continue;
            }
        }
        unique.push(TagInsertion {
            service_name: service_name.clone(),
            tag_key: kv.key.clone(),
            tag_value: kv.as_string(),
        });
    }
    unique
}

/// The Cassandra schema reference type string for a domain span reference.
/// `child_of == true` selects [`CHILD_OF`], otherwise [`FOLLOWS_FROM`].
pub fn ref_type_string(child_of: bool) -> &'static str {
    if child_of {
        CHILD_OF
    } else {
        FOLLOWS_FROM
    }
}
