// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Core data model — `TraceId`, `SpanId`, `SpanContext`, `SpanData`,
//! `SpanKind`, `Status`, attribute values.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A unique identifier for a trace, represented as a 128-bit unsigned integer.
pub type TraceId = u128;

/// A unique identifier for a span, represented as a 64-bit unsigned integer.
pub type SpanId = u64;

/// Formats a `TraceId` into a 32-character lowercase hexadecimal string.
///
/// # Arguments
///
/// * `id` - The trace ID to format.
///
/// # Returns
///
/// A string containing the hex representation of the trace ID.
pub fn format_trace_id(id: TraceId) -> String {
    format!("{:032x}", id)
}

/// Formats a `SpanId` into a 16-character lowercase hexadecimal string.
///
/// # Arguments
///
/// * `id` - The span ID to format.
///
/// # Returns
///
/// A string containing the hex representation of the span ID.
pub fn format_span_id(id: SpanId) -> String {
    format!("{:016x}", id)
}

/// Parses a hexadecimal string into a `TraceId`.
///
/// # Arguments
///
/// * `s` - A 32-character string containing only ASCII hexadecimal digits.
///
/// # Returns
///
/// `Some(TraceId)` if the string is valid, `None` otherwise.
pub fn parse_trace_id(s: &str) -> Option<TraceId> {
    if s.len() != 32 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u128::from_str_radix(s, 16).ok()
}

/// Parses a hexadecimal string into a `SpanId`.
///
/// # Arguments
///
/// * `s` - A 16-character string containing only ASCII hexadecimal digits.
///
/// # Returns
///
/// `Some(SpanId)` if the string is valid, `None` otherwise.
pub fn parse_span_id(s: &str) -> Option<SpanId> {
    if s.len() != 16 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(s, 16).ok()
}

/// The kind of span, indicating the span's role in the trace.
///
/// This enum defines the standard OpenTelemetry span kinds.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SpanKind {
    /// An internal span within the service.
    Internal,
    /// A span representing a client request.
    Client,
    /// A span representing a server request.
    Server,
    /// A span representing a producer sending a message.
    Producer,
    /// A span representing a consumer receiving a message.
    Consumer,
}

/// Default implementation for `SpanKind`, returning `Internal`.
impl Default for SpanKind {
    fn default() -> Self {
        SpanKind::Internal
    }
}

/// The status of a span, indicating success or failure.
///
/// This enum represents the standard OpenTelemetry status codes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "code", content = "message")]
pub enum Status {
    /// The status is unset.
    Unset,
    /// The span completed successfully.
    Ok,
    /// The span ended with an error.
    Error(String),
}

/// Default implementation for `Status`, returning `Unset`.
impl Default for Status {
    fn default() -> Self {
        Status::Unset
    }
}

/// A single attribute value, supporting various data types.
///
/// This enum represents the possible types for span attributes,
/// including strings, integers, floats, booleans, and their array variants.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum AttrValue {
    /// A string value.
    String(String),
    /// A 64-bit integer value.
    Int(i64),
    /// A 64-bit float value.
    Float(f64),
    /// A boolean value.
    Bool(bool),
    /// An array of string values.
    StringArray(Vec<String>),
    /// An array of 64-bit integer values.
    IntArray(Vec<i64>),
    /// An array of 64-bit float values.
    FloatArray(Vec<f64>),
    /// An array of boolean values.
    BoolArray(Vec<bool>),
}

/// Converts a string slice into an `AttrValue::String`.
impl From<&str> for AttrValue {
    fn from(v: &str) -> Self {
        AttrValue::String(v.to_string())
    }
}

/// Converts a `String` into an `AttrValue::String`.
impl From<String> for AttrValue {
    fn from(v: String) -> Self {
        AttrValue::String(v)
    }
}

/// Converts an `i64` into an `AttrValue::Int`.
impl From<i64> for AttrValue {
    fn from(v: i64) -> Self {
        AttrValue::Int(v)
    }
}

/// Converts an `i32` into an `AttrValue::Int`.
impl From<i32> for AttrValue {
    fn from(v: i32) -> Self {
        AttrValue::Int(v as i64)
    }
}

/// Converts a `u32` into an `AttrValue::Int`.
impl From<u32> for AttrValue {
    fn from(v: u32) -> Self {
        AttrValue::Int(v as i64)
    }
}

/// Converts an `f64` into an `AttrValue::Float`.
impl From<f64> for AttrValue {
    fn from(v: f64) -> Self {
        AttrValue::Float(v)
    }
}

/// Converts a `bool` into an `AttrValue::Bool`.
impl From<bool> for AttrValue {
    fn from(v: bool) -> Self {
        AttrValue::Bool(v)
    }
}

/// A collection of span attributes, mapping attribute names to values.
pub type Attributes = HashMap<String, AttrValue>;

/// Context information for a span, including trace and span IDs.
///
/// This struct holds the essential identifiers and flags for a span,
/// such as whether it is sampled and whether it originated remotely.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct SpanContext {
    /// The trace ID associated with this span.
    pub trace_id: TraceId,
    /// The unique ID of this span.
    pub span_id: SpanId,
    /// Flags associated with the trace, such as sampling status.
    pub trace_flags: u8,
    /// Indicates if this span context is from a remote service.
    pub is_remote: bool,
}

impl SpanContext {
    /// The flag bit indicating that a span is sampled.
    pub const FLAG_SAMPLED: u8 = 0x01;

    /// Creates a new `SpanContext` with the given trace and span IDs.
    ///
    /// # Arguments
    ///
    /// * `trace_id` - The trace ID.
    /// * `span_id` - The span ID.
    /// * `sampled` - Whether the span is sampled.
    ///
    /// # Returns
    ///
    /// A new `SpanContext` instance.
    pub fn new(trace_id: TraceId, span_id: SpanId, sampled: bool) -> Self {
        SpanContext {
            trace_id,
            span_id,
            trace_flags: if sampled { Self::FLAG_SAMPLED } else { 0 },
            is_remote: false,
        }
    }

    /// Creates an invalid `SpanContext` with zeroed IDs and flags.
    ///
    /// # Returns
    ///
    /// A `SpanContext` with all fields set to zero/false.
    pub fn invalid() -> Self {
        SpanContext {
            trace_id: 0,
            span_id: 0,
            trace_flags: 0,
            is_remote: false,
        }
    }

    /// Checks if the span context is valid (non-zero IDs).
    ///
    /// # Returns
    ///
    /// `true` if both `trace_id` and `span_id` are non-zero, `false` otherwise.
    pub fn is_valid(&self) -> bool {
        self.trace_id != 0 && self.span_id != 0
    }

    /// Checks if the span is sampled based on trace flags.
    ///
    /// # Returns
    ///
    /// `true` if the `FLAG_SAMPLED` bit is set, `false` otherwise.
    pub fn is_sampled(&self) -> bool {
        self.trace_flags & Self::FLAG_SAMPLED == Self::FLAG_SAMPLED
    }
}

/// Represents an event within a span.
///
/// Events are timestamped records of significant occurrences during a span's lifetime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// The name of the event.
    pub name: String,
    /// The time at which the event occurred.
    pub time: DateTime<Utc>,
    /// Optional attributes associated with the event.
    #[serde(default)]
    pub attributes: Attributes,
}

/// Represents a link to another span.
///
/// Links connect spans across different traces or services.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Link {
    /// The context of the linked span.
    pub context: SpanContext,
    /// Optional attributes associated with the link.
    #[serde(default)]
    pub attributes: Attributes,
}

/// The core data structure representing a span's complete information.
///
/// This struct contains all the details about a span, including its context,
/// timing, attributes, events, links, and status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpanData {
    /// The name of the span.
    pub name: String,
    /// The context of the span.
    pub context: SpanContext,
    /// The ID of the parent span, if any.
    pub parent_span_id: Option<SpanId>,
    /// The kind of the span.
    pub kind: SpanKind,
    /// The start time of the span.
    pub start_time: DateTime<Utc>,
    /// The end time of the span.
    pub end_time: DateTime<Utc>,
    /// Optional attributes associated with the span.
    #[serde(default)]
    pub attributes: Attributes,
    /// Optional events recorded during the span.
    #[serde(default)]
    pub events: Vec<Event>,
    /// Optional links to other spans.
    #[serde(default)]
    pub links: Vec<Link>,
    /// The status of the span.
    #[serde(default)]
    pub status: Status,
    /// The instrumentation scope that created this span.
    pub instrumentation_scope: String,
    /// The tenant ID associated with this span.
    #[serde(default = "default_tenant")]
    pub tenant_id: String,
    /// Resource attributes associated with the span.
    pub resource: HashMap<String, String>,
}

/// The default tenant ID used when not specified.
pub const DEFAULT_TENANT: &str = "anonymous";

/// The label key used for tenant identification.
pub const TENANT_LABEL: &str = "tenant_id";

/// Default function to provide the default tenant ID.
fn default_tenant() -> String {
    DEFAULT_TENANT.to_string()
}

impl SpanData {
    /// Calculates the duration of the span.
    ///
    /// # Returns
    ///
    /// The duration between `start_time` and `end_time`.
    pub fn duration(&self) -> chrono::Duration {
        self.end_time.signed_duration_since(self.start_time)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_trace_id_pads_to_32() {
        assert_eq!(
            format_trace_id(0xdeadbeef),
            "000000000000000000000000deadbeef"
        );
    }

    #[test]
    fn test_format_span_id_pads_to_16() {
        assert_eq!(format_span_id(0xabcd), "000000000000abcd");
    }

    #[test]
    fn test_parse_trace_id_round_trip() {
        let id: TraceId = 0x0123456789abcdef_fedcba9876543210;
        assert_eq!(parse_trace_id(&format_trace_id(id)).unwrap(), id);
    }

    #[test]
    fn test_parse_trace_id_rejects_short() {
        assert!(parse_trace_id("abc").is_none());
    }

    #[test]
    fn test_parse_trace_id_rejects_non_hex() {
        assert!(parse_trace_id(&"g".repeat(32)).is_none());
    }

    #[test]
    fn test_parse_span_id_round_trip() {
        let id: SpanId = 0x1234_5678_9abc_def0;
        assert_eq!(parse_span_id(&format_span_id(id)).unwrap(), id);
    }

    #[test]
    fn test_span_context_validity() {
        assert!(!SpanContext::invalid().is_valid());
        assert!(SpanContext::new(1, 1, true).is_valid());
        assert!(SpanContext::new(1, 1, true).is_sampled());
        assert!(!SpanContext::new(1, 1, false).is_sampled());
    }

    #[test]
    fn test_attr_value_from_primitives() {
        assert_eq!(AttrValue::from("hi"), AttrValue::String("hi".into()));
        assert_eq!(AttrValue::from(42i64), AttrValue::Int(42));
        assert_eq!(AttrValue::from(true), AttrValue::Bool(true));
        assert_eq!(AttrValue::from(3.14), AttrValue::Float(3.14));
    }

    #[test]
    fn test_status_serde_unset() {
        let s = serde_json::to_string(&Status::Unset).unwrap();
        assert!(s.contains("unset"));
        let restored: Status = serde_json::from_str(&s).unwrap();
        assert_eq!(restored, Status::Unset);
    }

    #[test]
    fn test_status_serde_error_carries_message() {
        let s = serde_json::to_string(&Status::Error("boom".into())).unwrap();
        let restored: Status = serde_json::from_str(&s).unwrap();
        assert_eq!(restored, Status::Error("boom".into()));
    }

    #[test]
    fn test_span_kind_default_internal() {
        assert_eq!(SpanKind::default(), SpanKind::Internal);
    }
}
