//! PostgreSQL type system — OIDs, type descriptors, and runtime values.
//!
//! Covers every standard PostgreSQL data type including composites, arrays, enums,
//! range types, and domain types.

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use uuid::Uuid;

/// PostgreSQL Object Identifier — uniquely identifies every type, table, index, etc.
pub type Oid = u32;

/// Well-known type OIDs (from pg_type).
#[allow(dead_code)]
pub mod oid {
    use super::Oid;
    pub const BOOL: Oid = 16;
    pub const BYTEA: Oid = 17;
    pub const CHAR: Oid = 18;        // "char" (internal 1-byte type)
    pub const NAME: Oid = 19;
    pub const INT8: Oid = 20;        // bigint
    pub const INT2: Oid = 21;        // smallint
    pub const INT2VECTOR: Oid = 22;
    pub const INT4: Oid = 23;        // integer
    pub const REGPROC: Oid = 24;
    pub const TEXT: Oid = 25;
    pub const OID: Oid = 26;
    pub const TID: Oid = 27;
    pub const XID: Oid = 28;
    pub const CID: Oid = 29;
    pub const OIDVECTOR: Oid = 30;
    pub const PG_DDL_COMMAND: Oid = 32;
    pub const JSON: Oid = 114;
    pub const XML: Oid = 142;
    pub const PG_NODE_TREE: Oid = 194;
    pub const SMGR: Oid = 210;
    pub const INDEX_AM_HANDLER: Oid = 325;
    pub const POINT: Oid = 600;
    pub const LSEG: Oid = 601;
    pub const PATH: Oid = 602;
    pub const BOX: Oid = 603;
    pub const POLYGON: Oid = 604;
    pub const LINE: Oid = 628;
    pub const FLOAT4: Oid = 700;     // real
    pub const FLOAT8: Oid = 701;     // double precision
    pub const ABSTIME: Oid = 702;
    pub const RELTIME: Oid = 703;
    pub const TINTERVAL: Oid = 704;
    pub const UNKNOWN: Oid = 705;
    pub const CIRCLE: Oid = 718;
    pub const MACADDR8: Oid = 774;
    pub const MONEY: Oid = 790;
    pub const MACADDR: Oid = 829;
    pub const INET: Oid = 869;
    pub const CIDR: Oid = 650;
    pub const BOOL_ARRAY: Oid = 1000;
    pub const BYTEA_ARRAY: Oid = 1001;
    pub const CHAR_ARRAY: Oid = 1002;
    pub const NAME_ARRAY: Oid = 1003;
    pub const INT2_ARRAY: Oid = 1005;
    pub const INT4_ARRAY: Oid = 1007;
    pub const TEXT_ARRAY: Oid = 1009;
    pub const BPCHAR_ARRAY: Oid = 1014;
    pub const VARCHAR_ARRAY: Oid = 1015;
    pub const INT8_ARRAY: Oid = 1016;
    pub const FLOAT4_ARRAY: Oid = 1021;
    pub const FLOAT8_ARRAY: Oid = 1022;
    pub const OID_ARRAY: Oid = 1028;
    pub const ACLITEM: Oid = 1033;
    pub const CSTRING_ARRAY: Oid = 1263;
    pub const BPCHAR: Oid = 1042;    // char(n)
    pub const VARCHAR: Oid = 1043;   // varchar(n)
    pub const DATE: Oid = 1082;
    pub const TIME: Oid = 1083;
    pub const TIMESTAMP: Oid = 1114;
    pub const TIMESTAMP_ARRAY: Oid = 1115;
    pub const DATE_ARRAY: Oid = 1182;
    pub const TIME_ARRAY: Oid = 1183;
    pub const TIMESTAMPTZ: Oid = 1184;
    pub const TIMESTAMPTZ_ARRAY: Oid = 1185;
    pub const INTERVAL: Oid = 1186;
    pub const INTERVAL_ARRAY: Oid = 1187;
    pub const NUMERIC_ARRAY: Oid = 1231;
    pub const TIMETZ: Oid = 1266;
    pub const BIT: Oid = 1560;
    pub const BIT_ARRAY: Oid = 1561;
    pub const VARBIT: Oid = 1562;
    pub const VARBIT_ARRAY: Oid = 1563;
    pub const NUMERIC: Oid = 1700;   // decimal / numeric
    pub const REFCURSOR: Oid = 1790;
    pub const REGPROCEDURE: Oid = 2202;
    pub const REGOPER: Oid = 2203;
    pub const REGOPERATOR: Oid = 2204;
    pub const REGCLASS: Oid = 2205;
    pub const REGTYPE: Oid = 2206;
    pub const REGROLE: Oid = 4096;
    pub const REGTYPEARRAY: Oid = 2211;
    pub const UUID: Oid = 2950;
    pub const UUID_ARRAY: Oid = 2951;
    pub const LSN: Oid = 3220;
    pub const TSV_ECTOR: Oid = 3614;
    pub const TSQUERY: Oid = 3615;
    pub const REGCONFIG: Oid = 3734;
    pub const REGDICTIONARY: Oid = 3769;
    pub const JSONB: Oid = 3802;
    pub const JSONB_ARRAY: Oid = 3807;
    pub const JSON_ARRAY: Oid = 199;
    pub const JSONPATH: Oid = 4072;
    pub const TXID_SNAPSHOT: Oid = 2970;
    pub const INT4RANGE: Oid = 3904;
    pub const NUMRANGE: Oid = 3906;
    pub const TSRANGE: Oid = 3908;
    pub const TSTZRANGE: Oid = 3910;
    pub const DATERANGE: Oid = 3912;
    pub const INT8RANGE: Oid = 3926;
    pub const RECORD: Oid = 2249;
    pub const RECORD_ARRAY: Oid = 2287;
    pub const CSTRING: Oid = 2275;
    pub const ANY: Oid = 2276;
    pub const ANYARRAY: Oid = 2277;
    pub const VOID: Oid = 2278;
    pub const TRIGGER: Oid = 2279;
    pub const EVENT_TRIGGER: Oid = 3838;
    pub const LANGUAGE_HANDLER: Oid = 2280;
    pub const INTERNAL: Oid = 2281;
    pub const OPAQUE: Oid = 2282;
    pub const ANYELEMENT: Oid = 2283;
    pub const ANYNONARRAY: Oid = 2776;
    pub const ANYENUM: Oid = 3500;
    pub const FDW_HANDLER: Oid = 3115;
    pub const PG_ATTRIBUTE_P: Oid = 3232;
    pub const ANYRANGE: Oid = 3831;

    // Dynamic OID range for user-defined types (starting above pg built-ins)
    pub const USER_DEFINED_START: Oid = 16384;
}

/// PostgreSQL column format code
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatCode {
    Text = 0,
    Binary = 1,
}

impl FormatCode {
    pub fn from_i16(v: i16) -> Self {
        if v == 1 { Self::Binary } else { Self::Text }
    }
}

/// A PostgreSQL interval value.
#[derive(Debug, Clone, PartialEq)]
pub struct Interval {
    /// Months component
    pub months: i32,
    /// Days component
    pub days: i32,
    /// Microseconds component
    pub microseconds: i64,
}

impl Interval {
    pub fn zero() -> Self {
        Self { months: 0, days: 0, microseconds: 0 }
    }

    pub fn from_seconds(secs: f64) -> Self {
        let total_us = (secs * 1_000_000.0) as i64;
        Self { months: 0, days: 0, microseconds: total_us }
    }

    pub fn from_components(years: i32, months: i32, days: i32, hours: i32, minutes: i32, seconds: f64) -> Self {
        let total_months = years * 12 + months;
        let total_us = (hours as i64 * 3_600_000_000)
            + (minutes as i64 * 60_000_000)
            + (seconds * 1_000_000.0) as i64;
        Self { months: total_months, days, microseconds: total_us }
    }

    pub fn to_text(&self) -> String {
        let mut parts = Vec::new();
        let years = self.months / 12;
        let months = self.months % 12;
        if years != 0 { parts.push(format!("{years} year{}", if years.abs() != 1 { "s" } else { "" })); }
        if months != 0 { parts.push(format!("{months} month{}", if months.abs() != 1 { "s" } else { "" })); }
        if self.days != 0 { parts.push(format!("{} day{}", self.days, if self.days.abs() != 1 { "s" } else { "" })); }
        let us = self.microseconds.abs();
        let hours = us / 3_600_000_000;
        let mins = (us % 3_600_000_000) / 60_000_000;
        let secs = (us % 60_000_000) as f64 / 1_000_000.0;
        let sign = if self.microseconds < 0 { "-" } else { "" };
        if hours != 0 || mins != 0 || secs != 0.0 {
            parts.push(format!("{sign}{hours:02}:{mins:02}:{secs:09.6}"));
        }
        if parts.is_empty() { "00:00:00".to_string() } else { parts.join(" ") }
    }
}

/// Runtime value of any PostgreSQL type.
#[derive(Debug, Clone, PartialEq)]
pub enum PgValue {
    Null,
    Bool(bool),
    Int2(i16),
    Int4(i32),
    Int8(i64),
    Float4(f32),
    Float8(f64),
    Numeric(rust_decimal::Decimal),
    Text(String),
    Bytea(Vec<u8>),
    Char(String),     // bpchar — blank-padded char(n)
    Varchar(String),  // varchar(n)
    Date(NaiveDate),
    Time(NaiveTime),
    Timestamp(NaiveDateTime),
    TimestampTz(DateTime<Utc>),
    Interval(Interval),
    Uuid(Uuid),
    Json(JsonValue),
    Jsonb(JsonValue),
    Array { element_oid: Oid, elements: Vec<PgValue> },
    Composite { type_oid: Oid, fields: Vec<(String, PgValue)> },
    Enum { type_oid: Oid, label: String },
    Inet(String),
    Cidr(String),
    Macaddr(String),
    Bit(Vec<u8>, usize),   // bits, bit_length
    Varbit(Vec<u8>, usize),
    Xml(String),
    Oid(Oid),
    Tid(u32, u16),         // page, offset
    Xid(u32),
    Cid(u32),
    Record(Vec<PgValue>),
    Range { element_oid: Oid, lower: Option<Box<PgValue>>, upper: Option<Box<PgValue>>, lower_inc: bool, upper_inc: bool },
    Void,
}

impl PgValue {
    /// Return the OID for this value.
    pub fn oid(&self) -> Oid {
        match self {
            PgValue::Null => oid::TEXT,  // unknown/null defaults to text
            PgValue::Bool(_) => oid::BOOL,
            PgValue::Int2(_) => oid::INT2,
            PgValue::Int4(_) => oid::INT4,
            PgValue::Int8(_) => oid::INT8,
            PgValue::Float4(_) => oid::FLOAT4,
            PgValue::Float8(_) => oid::FLOAT8,
            PgValue::Numeric(_) => oid::NUMERIC,
            PgValue::Text(_) | PgValue::Varchar(_) => oid::TEXT,
            PgValue::Bytea(_) => oid::BYTEA,
            PgValue::Char(_) => oid::BPCHAR,
            PgValue::Date(_) => oid::DATE,
            PgValue::Time(_) => oid::TIME,
            PgValue::Timestamp(_) => oid::TIMESTAMP,
            PgValue::TimestampTz(_) => oid::TIMESTAMPTZ,
            PgValue::Interval(_) => oid::INTERVAL,
            PgValue::Uuid(_) => oid::UUID,
            PgValue::Json(_) => oid::JSON,
            PgValue::Jsonb(_) => oid::JSONB,
            PgValue::Array { element_oid, .. } => array_oid_for(*element_oid),
            PgValue::Composite { type_oid, .. } => *type_oid,
            PgValue::Enum { type_oid, .. } => *type_oid,
            PgValue::Inet(_) => oid::INET,
            PgValue::Cidr(_) => oid::CIDR,
            PgValue::Macaddr(_) => oid::MACADDR,
            PgValue::Bit(..) => oid::BIT,
            PgValue::Varbit(..) => oid::VARBIT,
            PgValue::Xml(_) => oid::XML,
            PgValue::Oid(_) => oid::OID,
            PgValue::Tid(..) => oid::TID,
            PgValue::Xid(_) => oid::XID,
            PgValue::Cid(_) => oid::CID,
            PgValue::Record(_) => oid::RECORD,
            PgValue::Range { element_oid, .. } => range_oid_for(*element_oid),
            PgValue::Void => oid::VOID,
        }
    }

    /// Render value as PostgreSQL text representation.
    pub fn to_text(&self) -> String {
        match self {
            PgValue::Null => String::new(),
            PgValue::Bool(b) => if *b { "t".to_string() } else { "f".to_string() },
            PgValue::Int2(v) => v.to_string(),
            PgValue::Int4(v) => v.to_string(),
            PgValue::Int8(v) => v.to_string(),
            PgValue::Float4(v) => {
                if v.is_nan() { "NaN".to_string() }
                else if v.is_infinite() { if *v > 0.0 { "Infinity".to_string() } else { "-Infinity".to_string() } }
                else { format!("{v}") }
            }
            PgValue::Float8(v) => {
                if v.is_nan() { "NaN".to_string() }
                else if v.is_infinite() { if *v > 0.0 { "Infinity".to_string() } else { "-Infinity".to_string() } }
                else { format!("{v}") }
            }
            PgValue::Numeric(v) => v.to_string(),
            PgValue::Text(s) | PgValue::Varchar(s) | PgValue::Char(s) => s.clone(),
            PgValue::Bytea(b) => {
                let mut s = String::from("\\x");
                for byte in b { s.push_str(&format!("{byte:02x}")); }
                s
            }
            PgValue::Date(d) => d.format("%Y-%m-%d").to_string(),
            PgValue::Time(t) => t.format("%H:%M:%S%.6f").to_string(),
            PgValue::Timestamp(ts) => ts.format("%Y-%m-%d %H:%M:%S%.6f").to_string(),
            PgValue::TimestampTz(ts) => ts.format("%Y-%m-%d %H:%M:%S%.6f%z").to_string(),
            PgValue::Interval(iv) => iv.to_text(),
            PgValue::Uuid(u) => u.to_string(),
            PgValue::Json(v) | PgValue::Jsonb(v) => v.to_string(),
            PgValue::Array { elements, .. } => {
                let inner: Vec<String> = elements.iter().map(|e| {
                    if *e == PgValue::Null { "NULL".to_string() }
                    else {
                        let s = e.to_text();
                        // Quote if needed
                        if s.contains(',') || s.contains('"') || s.contains('{') || s.contains('}') || s.contains('\\') {
                            format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
                        } else {
                            s
                        }
                    }
                }).collect();
                format!("{{{}}}", inner.join(","))
            }
            PgValue::Composite { fields, .. } => {
                let inner: Vec<String> = fields.iter().map(|(_, v)| v.to_text()).collect();
                format!("({})", inner.join(","))
            }
            PgValue::Enum { label, .. } => label.clone(),
            PgValue::Inet(s) | PgValue::Cidr(s) | PgValue::Macaddr(s) => s.clone(),
            PgValue::Bit(bytes, len) | PgValue::Varbit(bytes, len) => {
                let mut s = String::new();
                for i in 0..*len {
                    let byte = bytes[i / 8];
                    let bit = (byte >> (7 - (i % 8))) & 1;
                    s.push(if bit == 1 { '1' } else { '0' });
                }
                s
            }
            PgValue::Xml(s) => s.clone(),
            PgValue::Oid(v) => v.to_string(),
            PgValue::Tid(page, off) => format!("({page},{off})"),
            PgValue::Xid(v) => v.to_string(),
            PgValue::Cid(v) => v.to_string(),
            PgValue::Record(fields) => {
                let inner: Vec<String> = fields.iter().map(|v| v.to_text()).collect();
                format!("({})", inner.join(","))
            }
            PgValue::Range { lower, upper, lower_inc, upper_inc, .. } => {
                let lb = if *lower_inc { "[" } else { "(" };
                let ub = if *upper_inc { "]" } else { ")" };
                let lv = lower.as_ref().map(|v| v.to_text()).unwrap_or_default();
                let uv = upper.as_ref().map(|v| v.to_text()).unwrap_or_default();
                format!("{lb}{lv},{uv}{ub}")
            }
            PgValue::Void => String::new(),
        }
    }

    /// Is this value truthy?
    pub fn is_true(&self) -> bool {
        match self {
            PgValue::Bool(b) => *b,
            PgValue::Int2(v) => *v != 0,
            PgValue::Int4(v) => *v != 0,
            PgValue::Int8(v) => *v != 0,
            PgValue::Null => false,
            _ => true,
        }
    }

    /// Is this value NULL?
    pub fn is_null(&self) -> bool {
        matches!(self, PgValue::Null)
    }

    /// Cast to a different type by OID.
    pub fn cast_to(&self, target_oid: Oid) -> crate::error::Result<PgValue> {
        use crate::error::{Error, SqlState};
        if self.oid() == target_oid { return Ok(self.clone()); }
        match (self, target_oid) {
            (PgValue::Null, _) => Ok(PgValue::Null),
            // Numeric upcasts
            (PgValue::Int2(v), oid::INT4) => Ok(PgValue::Int4(*v as i32)),
            (PgValue::Int2(v), oid::INT8) => Ok(PgValue::Int8(*v as i64)),
            (PgValue::Int2(v), oid::FLOAT4) => Ok(PgValue::Float4(*v as f32)),
            (PgValue::Int2(v), oid::FLOAT8) => Ok(PgValue::Float8(*v as f64)),
            (PgValue::Int4(v), oid::INT8) => Ok(PgValue::Int8(*v as i64)),
            (PgValue::Int4(v), oid::FLOAT4) => Ok(PgValue::Float4(*v as f32)),
            (PgValue::Int4(v), oid::FLOAT8) => Ok(PgValue::Float8(*v as f64)),
            (PgValue::Int8(v), oid::FLOAT8) => Ok(PgValue::Float8(*v as f64)),
            (PgValue::Float4(v), oid::FLOAT8) => Ok(PgValue::Float8(*v as f64)),
            // Numeric downcasts (potentially lossy)
            (PgValue::Int4(v), oid::INT2) => Ok(PgValue::Int2(*v as i16)),
            (PgValue::Int8(v), oid::INT2) => Ok(PgValue::Int2(*v as i16)),
            (PgValue::Int8(v), oid::INT4) => Ok(PgValue::Int4(*v as i32)),
            (PgValue::Float8(v), oid::FLOAT4) => Ok(PgValue::Float4(*v as f32)),
            // Text conversions
            (PgValue::Text(s), oid::INT4) | (PgValue::Varchar(s), oid::INT4) => {
                s.trim().parse::<i32>()
                    .map(PgValue::Int4)
                    .map_err(|_| Error::Pg(crate::error::PgError::invalid_text_representation("integer", s)))
            }
            (PgValue::Text(s), oid::INT8) | (PgValue::Varchar(s), oid::INT8) => {
                s.trim().parse::<i64>()
                    .map(PgValue::Int8)
                    .map_err(|_| Error::Pg(crate::error::PgError::invalid_text_representation("bigint", s)))
            }
            (PgValue::Text(s), oid::FLOAT8) | (PgValue::Varchar(s), oid::FLOAT8) => {
                s.trim().parse::<f64>()
                    .map(PgValue::Float8)
                    .map_err(|_| Error::Pg(crate::error::PgError::invalid_text_representation("double precision", s)))
            }
            (PgValue::Text(s), oid::BOOL) | (PgValue::Varchar(s), oid::BOOL) => {
                match s.trim().to_lowercase().as_str() {
                    "t" | "true" | "yes" | "on" | "1" => Ok(PgValue::Bool(true)),
                    "f" | "false" | "no" | "off" | "0" => Ok(PgValue::Bool(false)),
                    _ => Err(Error::Pg(crate::error::PgError::invalid_text_representation("boolean", s))),
                }
            }
            (PgValue::Text(s), oid::UUID) | (PgValue::Varchar(s), oid::UUID) => {
                s.trim().parse::<Uuid>()
                    .map(PgValue::Uuid)
                    .map_err(|_| Error::Pg(crate::error::PgError::invalid_text_representation("uuid", s)))
            }
            (PgValue::Text(s), oid::JSON) | (PgValue::Varchar(s), oid::JSON) => {
                serde_json::from_str::<JsonValue>(s)
                    .map(PgValue::Json)
                    .map_err(|e| Error::Pg(crate::error::PgError::error(SqlState::INVALID_JSON_TEXT, e.to_string())))
            }
            (PgValue::Text(s), oid::JSONB) | (PgValue::Varchar(s), oid::JSONB) => {
                serde_json::from_str::<JsonValue>(s)
                    .map(PgValue::Jsonb)
                    .map_err(|e| Error::Pg(crate::error::PgError::error(SqlState::INVALID_JSON_TEXT, e.to_string())))
            }
            // Any → text
            (v, oid::TEXT) | (v, oid::VARCHAR) => Ok(PgValue::Text(v.to_text())),
            // Bool ↔ numeric
            (PgValue::Bool(b), oid::INT4) => Ok(PgValue::Int4(if *b { 1 } else { 0 })),
            (PgValue::Bool(b), oid::INT8) => Ok(PgValue::Int8(if *b { 1 } else { 0 })),
            (PgValue::Int4(v), oid::BOOL) => Ok(PgValue::Bool(*v != 0)),
            (PgValue::Int8(v), oid::BOOL) => Ok(PgValue::Bool(*v != 0)),
            // JSON ↔ JSONB
            (PgValue::Json(v), oid::JSONB) => Ok(PgValue::Jsonb(v.clone())),
            (PgValue::Jsonb(v), oid::JSON) => Ok(PgValue::Json(v.clone())),
            // OID conversions
            (PgValue::Int4(v), oid::OID) => Ok(PgValue::Oid(*v as u32)),
            (PgValue::Oid(v), oid::INT4) => Ok(PgValue::Int4(*v as i32)),
            (PgValue::Oid(v), oid::INT8) => Ok(PgValue::Int8(*v as i64)),
            _ => Err(Error::Type(format!(
                "cannot cast {} to OID {}",
                type_name_for_oid(self.oid()),
                type_name_for_oid(target_oid)
            ))),
        }
    }

    /// Numeric comparison helper — returns None if not comparable.
    pub fn compare(&self, other: &PgValue) -> Option<std::cmp::Ordering> {
        match (self, other) {
            (PgValue::Null, PgValue::Null) => None,
            (PgValue::Null, _) | (_, PgValue::Null) => None,
            (PgValue::Bool(a), PgValue::Bool(b)) => a.partial_cmp(b),
            (PgValue::Int2(a), PgValue::Int2(b)) => a.partial_cmp(b),
            (PgValue::Int4(a), PgValue::Int4(b)) => a.partial_cmp(b),
            (PgValue::Int8(a), PgValue::Int8(b)) => a.partial_cmp(b),
            (PgValue::Float4(a), PgValue::Float4(b)) => a.partial_cmp(b),
            (PgValue::Float8(a), PgValue::Float8(b)) => a.partial_cmp(b),
            (PgValue::Numeric(a), PgValue::Numeric(b)) => a.partial_cmp(b),
            (PgValue::Text(a), PgValue::Text(b)) | (PgValue::Varchar(a), PgValue::Varchar(b)) => a.partial_cmp(b),
            (PgValue::Date(a), PgValue::Date(b)) => a.partial_cmp(b),
            (PgValue::Time(a), PgValue::Time(b)) => a.partial_cmp(b),
            (PgValue::Timestamp(a), PgValue::Timestamp(b)) => a.partial_cmp(b),
            (PgValue::TimestampTz(a), PgValue::TimestampTz(b)) => a.partial_cmp(b),
            (PgValue::Uuid(a), PgValue::Uuid(b)) => a.partial_cmp(b),
            // Cross-type numeric comparisons (coerce to float8)
            _ => {
                let af = self.to_f64();
                let bf = other.to_f64();
                if let (Some(a), Some(b)) = (af, bf) {
                    a.partial_cmp(&b)
                } else {
                    None
                }
            }
        }
    }

    pub fn to_f64(&self) -> Option<f64> {
        match self {
            PgValue::Int2(v) => Some(*v as f64),
            PgValue::Int4(v) => Some(*v as f64),
            PgValue::Int8(v) => Some(*v as f64),
            PgValue::Float4(v) => Some(*v as f64),
            PgValue::Float8(v) => Some(*v),
            PgValue::Numeric(v) => Some(v.to_string().parse::<f64>().unwrap_or(f64::NAN)),
            _ => None,
        }
    }

    pub fn to_i64(&self) -> Option<i64> {
        match self {
            PgValue::Int2(v) => Some(*v as i64),
            PgValue::Int4(v) => Some(*v as i64),
            PgValue::Int8(v) => Some(*v),
            PgValue::Float4(v) => Some(*v as i64),
            PgValue::Float8(v) => Some(*v as i64),
            PgValue::Bool(b) => Some(if *b { 1 } else { 0 }),
            _ => None,
        }
    }
}

/// Return the array type OID for a given element OID.
pub fn array_oid_for(elem_oid: Oid) -> Oid {
    match elem_oid {
        oid::BOOL => oid::BOOL_ARRAY,
        oid::INT2 => oid::INT2_ARRAY,
        oid::INT4 => oid::INT4_ARRAY,
        oid::INT8 => oid::INT8_ARRAY,
        oid::FLOAT4 => oid::FLOAT4_ARRAY,
        oid::FLOAT8 => oid::FLOAT8_ARRAY,
        oid::TEXT => oid::TEXT_ARRAY,
        oid::VARCHAR => oid::VARCHAR_ARRAY,
        oid::BPCHAR => oid::BPCHAR_ARRAY,
        oid::UUID => oid::UUID_ARRAY,
        oid::JSONB => oid::JSONB_ARRAY,
        oid::JSON => oid::JSON_ARRAY,
        oid::TIMESTAMP => oid::TIMESTAMP_ARRAY,
        oid::TIMESTAMPTZ => oid::TIMESTAMPTZ_ARRAY,
        oid::DATE => oid::DATE_ARRAY,
        oid::TIME => oid::TIME_ARRAY,
        oid::INTERVAL => oid::INTERVAL_ARRAY,
        oid::NUMERIC => oid::NUMERIC_ARRAY,
        _ => 0, // unknown — clients will use array OID 0
    }
}

/// Return the range type OID for a given element OID.
pub fn range_oid_for(elem_oid: Oid) -> Oid {
    match elem_oid {
        oid::INT4 => oid::INT4RANGE,
        oid::INT8 => oid::INT8RANGE,
        oid::NUMERIC => oid::NUMRANGE,
        oid::TIMESTAMP => oid::TSRANGE,
        oid::TIMESTAMPTZ => oid::TSTZRANGE,
        oid::DATE => oid::DATERANGE,
        _ => 0,
    }
}

/// Return the human-readable type name for an OID.
pub fn type_name_for_oid(oid_val: Oid) -> &'static str {
    match oid_val {
        oid::BOOL => "boolean",
        oid::BYTEA => "bytea",
        oid::INT2 => "smallint",
        oid::INT4 => "integer",
        oid::INT8 => "bigint",
        oid::FLOAT4 => "real",
        oid::FLOAT8 => "double precision",
        oid::NUMERIC => "numeric",
        oid::TEXT => "text",
        oid::VARCHAR => "character varying",
        oid::BPCHAR => "character",
        oid::DATE => "date",
        oid::TIME => "time without time zone",
        oid::TIMESTAMP => "timestamp without time zone",
        oid::TIMESTAMPTZ => "timestamp with time zone",
        oid::INTERVAL => "interval",
        oid::UUID => "uuid",
        oid::JSON => "json",
        oid::JSONB => "jsonb",
        oid::OID => "oid",
        oid::VOID => "void",
        oid::RECORD => "record",
        oid::INET => "inet",
        oid::CIDR => "cidr",
        oid::MACADDR => "macaddr",
        _ => "unknown",
    }
}

/// Parse a SQL type name string into an OID.
pub fn oid_for_type_name(name: &str) -> Option<Oid> {
    match name.to_lowercase().trim() {
        "bool" | "boolean" => Some(oid::BOOL),
        "bytea" => Some(oid::BYTEA),
        "smallint" | "int2" | "int16" => Some(oid::INT2),
        "integer" | "int" | "int4" | "int32" => Some(oid::INT4),
        "bigint" | "int8" | "int64" => Some(oid::INT8),
        "real" | "float4" | "float" => Some(oid::FLOAT4),
        "double precision" | "float8" | "float64" => Some(oid::FLOAT8),
        "numeric" | "decimal" => Some(oid::NUMERIC),
        "text" => Some(oid::TEXT),
        "varchar" | "character varying" => Some(oid::VARCHAR),
        "char" | "bpchar" | "character" => Some(oid::BPCHAR),
        "date" => Some(oid::DATE),
        "time" | "time without time zone" => Some(oid::TIME),
        "timestamp" | "timestamp without time zone" => Some(oid::TIMESTAMP),
        "timestamptz" | "timestamp with time zone" => Some(oid::TIMESTAMPTZ),
        "interval" => Some(oid::INTERVAL),
        "uuid" => Some(oid::UUID),
        "json" => Some(oid::JSON),
        "jsonb" => Some(oid::JSONB),
        "oid" => Some(oid::OID),
        "void" => Some(oid::VOID),
        "inet" => Some(oid::INET),
        "cidr" => Some(oid::CIDR),
        "macaddr" => Some(oid::MACADDR),
        "bit" => Some(oid::BIT),
        "varbit" | "bit varying" => Some(oid::VARBIT),
        "xml" => Some(oid::XML),
        "serial" | "serial4" => Some(oid::INT4),   // serial is just int4 + sequence
        "bigserial" | "serial8" => Some(oid::INT8),
        "smallserial" | "serial2" => Some(oid::INT2),
        "name" => Some(oid::NAME),
        _ => None,
    }
}

/// A column descriptor — matches PostgreSQL's RowDescription field.
#[derive(Debug, Clone)]
pub struct ColumnDesc {
    pub name: String,
    pub table_oid: Oid,      // 0 if not from a table
    pub col_attr_num: i16,   // 0 if not from a table
    pub type_oid: Oid,
    pub type_size: i16,      // -1 for variable-length
    pub type_modifier: i32,  // -1 for no modifier
    pub format: FormatCode,
}

impl ColumnDesc {
    pub fn new(name: impl Into<String>, type_oid: Oid) -> Self {
        let type_size = type_size_for_oid(type_oid);
        Self {
            name: name.into(),
            table_oid: 0,
            col_attr_num: 0,
            type_oid,
            type_size,
            type_modifier: -1,
            format: FormatCode::Text,
        }
    }

    pub fn with_table(mut self, table_oid: Oid, col_attr_num: i16) -> Self {
        self.table_oid = table_oid;
        self.col_attr_num = col_attr_num;
        self
    }

    pub fn with_type_modifier(mut self, modifier: i32) -> Self {
        self.type_modifier = modifier;
        self
    }
}

fn type_size_for_oid(oid_val: Oid) -> i16 {
    match oid_val {
        oid::BOOL => 1,
        oid::INT2 => 2,
        oid::INT4 => 4,
        oid::INT8 => 8,
        oid::FLOAT4 => 4,
        oid::FLOAT8 => 8,
        oid::DATE => 4,
        oid::TIME => 8,
        oid::TIMESTAMP => 8,
        oid::TIMESTAMPTZ => 8,
        oid::OID => 4,
        oid::TID => 6,
        oid::XID | oid::CID => 4,
        oid::UUID => 16,
        _ => -1, // variable
    }
}

/// A result set — columns + rows.
#[derive(Debug, Clone, Default)]
pub struct ResultSet {
    pub columns: Vec<ColumnDesc>,
    pub rows: Vec<Vec<PgValue>>,
}

impl ResultSet {
    pub fn new(columns: Vec<ColumnDesc>) -> Self {
        Self { columns, rows: Vec::new() }
    }

    pub fn empty() -> Self {
        Self::default()
    }

    pub fn push_row(&mut self, row: Vec<PgValue>) {
        self.rows.push(row);
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }
}

/// Outcome of executing a SQL statement.
#[derive(Debug)]
pub enum CommandResult {
    /// SELECT or similar — columns + rows
    Rows(ResultSet),
    /// INSERT / UPDATE / DELETE
    Modified { tag: String, rows_affected: u64 },
    /// CREATE TABLE / CREATE INDEX / etc.
    Created(String),
    /// DROP TABLE / DROP INDEX / etc.
    Dropped(String),
    /// TRUNCATE
    Truncated(String),
    /// ALTER TABLE / etc.
    Altered(String),
    /// BEGIN / COMMIT / ROLLBACK / SAVEPOINT
    Transaction(String),
    /// LISTEN / NOTIFY
    Notify(String),
    /// COPY IN/OUT stats
    Copy { direction: String, rows: u64 },
    /// SET parameter
    Set,
    /// SHOW parameter
    Show(String),
    /// EXPLAIN output
    Explain(ResultSet),
    /// DO block / function call with void return
    Do,
    /// Empty result
    Empty,
}

impl CommandResult {
    /// Return the command completion tag string.
    pub fn completion_tag(&self) -> String {
        match self {
            CommandResult::Rows(rs) => format!("SELECT {}", rs.rows.len()),
            CommandResult::Modified { tag, .. } => tag.clone(),
            CommandResult::Created(tag) | CommandResult::Dropped(tag) |
            CommandResult::Truncated(tag) | CommandResult::Altered(tag) |
            CommandResult::Transaction(tag) | CommandResult::Notify(tag) => tag.clone(),
            CommandResult::Copy { direction, rows } => format!("COPY {rows}"),
            CommandResult::Set => "SET".to_string(),
            CommandResult::Show(_) => "SHOW".to_string(),
            CommandResult::Explain(rs) => format!("EXPLAIN"),
            CommandResult::Do => "DO".to_string(),
            CommandResult::Empty => String::new(),
        }
    }

    /// If this is a Rows result, return its ResultSet.
    pub fn into_rows(self) -> Option<ResultSet> {
        if let CommandResult::Rows(rs) = self { Some(rs) }
        else if let CommandResult::Explain(rs) = self { Some(rs) }
        else { None }
    }
}

/// A typed parameter value passed from the client.
#[derive(Debug, Clone)]
pub struct ParameterValue {
    pub oid: Oid,
    pub data: Option<Vec<u8>>,  // None = NULL
    pub format: FormatCode,
}

impl ParameterValue {
    pub fn null(oid: Oid) -> Self {
        Self { oid, data: None, format: FormatCode::Text }
    }

    pub fn text(oid: Oid, value: impl Into<String>) -> Self {
        Self { oid, data: Some(value.into().into_bytes()), format: FormatCode::Text }
    }
}

// Pull in rust_decimal for Numeric type support
// We declare it as an external dependency since Cargo.toml includes it.
// The actual feature set needed: serde + std
