// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Badger storage key-encoding store — line-port of jaeger
//! `plugin/storage/badger/spanstore/` (writer.go key layout + reader prefix
//! scans), pinned v1.52.0.
//!
//! Badger is an embedded sorted-LSM key/value store; Jaeger's plugin encodes
//! every span into a byte-exact **primary key** plus a fan of **secondary
//! index keys** and relies on the store's sorted-key ordering for range
//! scans. This module ports that *pure* key codec —
//!
//!   primary: `[0x80 | traceID.High | traceID.Low | startTime µs | spanID]`
//!   index:   `[(prefix & 0x0F) | 0x80 | value | startTime µs |
//!             traceID.High | traceID.Low]`
//!
//! — together with the per-index value bytes (service / service+operation /
//! service+operation+duration / service+key+AsString) and backs them with an
//! in-process [`std::collections::BTreeMap`] that reproduces Badger's sorted
//! ordering, so `get_trace` and `trace_ids_by_service` genuinely read through
//! the real layout.
//!
//! The live Badger mmap LSM, value-log, and GC stay scope_cut
//! (operational-storage-backends, Phase 3 cave-store); this is the encoder +
//! ordered store the writer/reader sit on.

use std::collections::BTreeMap;

use crate::types::{Span, TagValue};

// ─── Key prefix bytes (writer.go) ───────────────────────────────────────────

/// Primary (trace) key prefix.
pub const SPAN_KEY_PREFIX: u8 = 0x80;
/// Mask applied to secondary index prefixes before OR-ing in [`SPAN_KEY_PREFIX`].
pub const INDEX_KEY_RANGE: u8 = 0x0f;
/// Service-name index prefix.
pub const SERVICE_NAME_INDEX_KEY: u8 = 0x81;
/// Service+operation index prefix.
pub const OPERATION_NAME_INDEX_KEY: u8 = 0x82;
/// Tag index prefix.
pub const TAG_INDEX_KEY: u8 = 0x83;
/// Duration index prefix.
pub const DURATION_INDEX_KEY: u8 = 0x84;

const SIZE_OF_TRACE_ID: usize = 16;

// ─── Key encoders ────────────────────────────────────────────────────────────

/// Build the byte-exact primary key
/// `[0x80 | traceID.High | traceID.Low | startTime µs | spanID]`.
pub fn primary_key(trace_id: u128, start_micros: u64, span_id: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(1 + SIZE_OF_TRACE_ID + 8 + 8);
    key.push(SPAN_KEY_PREFIX);
    let high = (trace_id >> 64) as u64;
    let low = trace_id as u64;
    key.extend_from_slice(&high.to_be_bytes());
    key.extend_from_slice(&low.to_be_bytes());
    key.extend_from_slice(&start_micros.to_be_bytes());
    key.extend_from_slice(&span_id.to_be_bytes());
    key
}

/// Build a secondary index key
/// `[(prefix & 0x0F)|0x80 | value | startTime µs | traceID.High | traceID.Low]`.
pub fn index_key(index_prefix: u8, value: &[u8], start_micros: u64, trace_id: u128) -> Vec<u8> {
    let mut key = Vec::with_capacity(1 + value.len() + 8 + SIZE_OF_TRACE_ID);
    key.push((index_prefix & INDEX_KEY_RANGE) | SPAN_KEY_PREFIX);
    key.extend_from_slice(value);
    key.extend_from_slice(&start_micros.to_be_bytes());
    let high = (trace_id >> 64) as u64;
    let low = trace_id as u64;
    key.extend_from_slice(&high.to_be_bytes());
    key.extend_from_slice(&low.to_be_bytes());
    key
}

/// Service-name index value: the service name bytes.
pub fn service_index_value(service: &str) -> Vec<u8> {
    service.as_bytes().to_vec()
}

/// Service+operation index value: `service ++ operation`.
pub fn operation_index_value(service: &str, operation: &str) -> Vec<u8> {
    let mut v = service.as_bytes().to_vec();
    v.extend_from_slice(operation.as_bytes());
    v
}

/// Duration index value: `service ++ operation ++ duration µs (8 bytes BE)`.
pub fn duration_index_value(service: &str, operation: &str, duration_micros: u64) -> Vec<u8> {
    let mut v = operation_index_value(service, operation);
    v.extend_from_slice(&duration_micros.to_be_bytes());
    v
}

/// Tag index value: `service ++ key ++ AsString(value)`.
pub fn tag_index_value(service: &str, key: &str, value: &str) -> Vec<u8> {
    let mut v = service.as_bytes().to_vec();
    v.extend_from_slice(key.as_bytes());
    v.extend_from_slice(value.as_bytes());
    v
}

/// `model.KeyValue.AsString()` for a domain tag value.
fn tag_as_string(v: &TagValue) -> String {
    match v {
        TagValue::String(s) => s.clone(),
        TagValue::Bool(b) => if *b { "true" } else { "false" }.to_owned(),
        TagValue::Int(i) => i.to_string(),
        TagValue::Float(f) => f.to_string(),
        TagValue::Binary(b) => b.iter().map(|x| format!("{:02x}", x)).collect(),
    }
}

// ─── In-process ordered store ───────────────────────────────────────────────

/// An in-process Badger-equivalent: a sorted key/value map written through the
/// real Jaeger Badger key layout. Primary entries hold the JSON-encoded span;
/// index entries are key-only (empty value), exactly as Jaeger writes them.
#[derive(Debug, Default)]
pub struct BadgerStore {
    data: BTreeMap<Vec<u8>, Vec<u8>>,
}

impl BadgerStore {
    pub fn new() -> Self {
        BadgerStore {
            data: BTreeMap::new(),
        }
    }

    /// Number of keys in the store (1 primary + all index keys per span).
    pub fn key_count(&self) -> usize {
        self.data.len()
    }

    /// Write a span: one primary entry plus service / operation / duration /
    /// tag index keys (span tags + process tags + log fields).
    pub fn write_span(&mut self, span: &Span) {
        let start_micros = span.start_time_unix_nano / 1_000;
        let duration_micros = span.duration_ns / 1_000;
        let trace_id = span.trace_id;
        let service = &span.service_name;

        // Primary: value = encoded span.
        let pk = primary_key(trace_id, start_micros, span.span_id);
        let payload = serde_json::to_vec(span).unwrap_or_default();
        self.data.insert(pk, payload);

        // Service index.
        self.data.insert(
            index_key(
                SERVICE_NAME_INDEX_KEY,
                &service_index_value(service),
                start_micros,
                trace_id,
            ),
            Vec::new(),
        );
        // Service+operation index.
        self.data.insert(
            index_key(
                OPERATION_NAME_INDEX_KEY,
                &operation_index_value(service, &span.operation_name),
                start_micros,
                trace_id,
            ),
            Vec::new(),
        );
        // Duration index.
        self.data.insert(
            index_key(
                DURATION_INDEX_KEY,
                &duration_index_value(service, &span.operation_name, duration_micros),
                start_micros,
                trace_id,
            ),
            Vec::new(),
        );
        // Tag indexes — span tags, process tags, and log fields.
        let tag_sources = span
            .tags
            .iter()
            .chain(span.resource_attributes.iter())
            .map(|(k, v)| (k.clone(), v.clone()))
            .chain(
                span.events
                    .iter()
                    .flat_map(|e| e.attributes.iter().map(|(k, v)| (k.clone(), v.clone()))),
            );
        for (k, v) in tag_sources {
            self.data.insert(
                index_key(
                    TAG_INDEX_KEY,
                    &tag_index_value(service, &k, &tag_as_string(&v)),
                    start_micros,
                    trace_id,
                ),
                Vec::new(),
            );
        }
    }

    /// Read every span of a trace by prefix-scanning the primary key space.
    pub fn get_trace(&self, trace_id: u128) -> Vec<Span> {
        let high = (trace_id >> 64) as u64;
        let low = trace_id as u64;
        let mut prefix = Vec::with_capacity(1 + SIZE_OF_TRACE_ID);
        prefix.push(SPAN_KEY_PREFIX);
        prefix.extend_from_slice(&high.to_be_bytes());
        prefix.extend_from_slice(&low.to_be_bytes());

        self.data
            .range(prefix.clone()..)
            .take_while(|(k, _)| k.starts_with(&prefix))
            .filter_map(|(_, v)| serde_json::from_slice::<Span>(v).ok())
            .collect()
    }

    /// Find trace IDs for a service via a sorted scan of the service index,
    /// filtered to spans whose start time (µs) falls in `[lo_micros,
    /// hi_micros]`. Preserves discovery order and dedupes.
    pub fn trace_ids_by_service(&self, service: &str, lo_micros: u64, hi_micros: u64) -> Vec<u128> {
        let prefix_byte = (SERVICE_NAME_INDEX_KEY & INDEX_KEY_RANGE) | SPAN_KEY_PREFIX;
        let want = service.as_bytes();
        let mut out: Vec<u128> = Vec::new();

        for key in self.data.keys() {
            if key.first() != Some(&prefix_byte) {
                continue;
            }
            // key = [prefix(1) | value | start(8) | high(8) | low(8)]
            if key.len() < 1 + 8 + SIZE_OF_TRACE_ID {
                continue;
            }
            let value_end = key.len() - 8 - SIZE_OF_TRACE_ID;
            let value = &key[1..value_end];
            if value != want {
                continue;
            }
            let start = u64::from_be_bytes(key[value_end..value_end + 8].try_into().unwrap());
            if start < lo_micros || start > hi_micros {
                continue;
            }
            let high = u64::from_be_bytes(
                key[value_end + 8..value_end + 16].try_into().unwrap(),
            );
            let low = u64::from_be_bytes(
                key[value_end + 16..value_end + 24].try_into().unwrap(),
            );
            let id = ((high as u128) << 64) | (low as u128);
            if !out.contains(&id) {
                out.push(id);
            }
        }
        out
    }
}
