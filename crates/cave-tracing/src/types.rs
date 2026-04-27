//! Core data model — `TraceId`, `SpanId`, `SpanContext`, `SpanData`,
//! `SpanKind`, `Status`, attribute values.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type TraceId = u128;
pub type SpanId = u64;

pub fn format_trace_id(id: TraceId) -> String {
    format!("{:032x}", id)
}

pub fn format_span_id(id: SpanId) -> String {
    format!("{:016x}", id)
}

pub fn parse_trace_id(s: &str) -> Option<TraceId> {
    if s.len() != 32 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u128::from_str_radix(s, 16).ok()
}

pub fn parse_span_id(s: &str) -> Option<SpanId> {
    if s.len() != 16 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(s, 16).ok()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SpanKind {
    Internal,
    Client,
    Server,
    Producer,
    Consumer,
}

impl Default for SpanKind {
    fn default() -> Self {
        SpanKind::Internal
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "code", content = "message")]
pub enum Status {
    Unset,
    Ok,
    Error(String),
}

impl Default for Status {
    fn default() -> Self {
        Status::Unset
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum AttrValue {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    StringArray(Vec<String>),
    IntArray(Vec<i64>),
    FloatArray(Vec<f64>),
    BoolArray(Vec<bool>),
}

impl From<&str> for AttrValue {
    fn from(v: &str) -> Self { AttrValue::String(v.to_string()) }
}
impl From<String> for AttrValue {
    fn from(v: String) -> Self { AttrValue::String(v) }
}
impl From<i64> for AttrValue {
    fn from(v: i64) -> Self { AttrValue::Int(v) }
}
impl From<i32> for AttrValue {
    fn from(v: i32) -> Self { AttrValue::Int(v as i64) }
}
impl From<u32> for AttrValue {
    fn from(v: u32) -> Self { AttrValue::Int(v as i64) }
}
impl From<f64> for AttrValue {
    fn from(v: f64) -> Self { AttrValue::Float(v) }
}
impl From<bool> for AttrValue {
    fn from(v: bool) -> Self { AttrValue::Bool(v) }
}

pub type Attributes = HashMap<String, AttrValue>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct SpanContext {
    pub trace_id: TraceId,
    pub span_id: SpanId,
    pub trace_flags: u8,
    pub is_remote: bool,
}

impl SpanContext {
    pub const FLAG_SAMPLED: u8 = 0x01;

    pub fn new(trace_id: TraceId, span_id: SpanId, sampled: bool) -> Self {
        SpanContext {
            trace_id,
            span_id,
            trace_flags: if sampled { Self::FLAG_SAMPLED } else { 0 },
            is_remote: false,
        }
    }

    pub fn invalid() -> Self {
        SpanContext { trace_id: 0, span_id: 0, trace_flags: 0, is_remote: false }
    }

    pub fn is_valid(&self) -> bool {
        self.trace_id != 0 && self.span_id != 0
    }

    pub fn is_sampled(&self) -> bool {
        self.trace_flags & Self::FLAG_SAMPLED == Self::FLAG_SAMPLED
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub name: String,
    pub time: DateTime<Utc>,
    #[serde(default)]
    pub attributes: Attributes,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Link {
    pub context: SpanContext,
    #[serde(default)]
    pub attributes: Attributes,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpanData {
    pub name: String,
    pub context: SpanContext,
    pub parent_span_id: Option<SpanId>,
    pub kind: SpanKind,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    #[serde(default)]
    pub attributes: Attributes,
    #[serde(default)]
    pub events: Vec<Event>,
    #[serde(default)]
    pub links: Vec<Link>,
    #[serde(default)]
    pub status: Status,
    pub instrumentation_scope: String,
    #[serde(default = "default_tenant")]
    pub tenant_id: String,
    pub resource: HashMap<String, String>,
}

pub const DEFAULT_TENANT: &str = "anonymous";
pub const TENANT_LABEL: &str = "tenant_id";

fn default_tenant() -> String { DEFAULT_TENANT.to_string() }

impl SpanData {
    pub fn duration(&self) -> chrono::Duration {
        self.end_time.signed_duration_since(self.start_time)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_trace_id_pads_to_32() {
        assert_eq!(format_trace_id(0xdeadbeef), "000000000000000000000000deadbeef");
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
