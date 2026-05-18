// SPDX-License-Identifier: AGPL-3.0-or-later
//! SQL type system and value coercion.

use serde_json::{json, Value};
use std::cmp::Ordering;

/// PostgreSQL OID (Object ID) constants for common types.
pub mod oid {
    pub const INT4: u32 = 23;       // int
    pub const INT8: u32 = 20;       // bigint
    pub const NUMERIC: u32 = 1700;  // numeric
    pub const TEXT: u32 = 25;       // text
    pub const VARCHAR: u32 = 1043;  // varchar
    pub const BOOL: u32 = 16;       // bool
    pub const DATE: u32 = 1082;     // date
    pub const TIMESTAMP: u32 = 1114; // timestamp without time zone
    pub const NULL: u32 = 0;        // null
}

/// Rust representation of SQL types.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SqlType {
    Int4,
    Int8,
    Numeric,
    Text,
    Varchar,
    Bool,
    Date,
    Timestamp,
    Null,
}

impl SqlType {
    pub fn oid(&self) -> u32 {
        match self {
            SqlType::Int4 => oid::INT4,
            SqlType::Int8 => oid::INT8,
            SqlType::Numeric => oid::NUMERIC,
            SqlType::Text => oid::TEXT,
            SqlType::Varchar => oid::VARCHAR,
            SqlType::Bool => oid::BOOL,
            SqlType::Date => oid::DATE,
            SqlType::Timestamp => oid::TIMESTAMP,
            SqlType::Null => oid::NULL,
        }
    }

    pub fn from_oid(oid: u32) -> Option<SqlType> {
        match oid {
            oid::INT4 => Some(SqlType::Int4),
            oid::INT8 => Some(SqlType::Int8),
            oid::NUMERIC => Some(SqlType::Numeric),
            oid::TEXT => Some(SqlType::Text),
            oid::VARCHAR => Some(SqlType::Varchar),
            oid::BOOL => Some(SqlType::Bool),
            oid::DATE => Some(SqlType::Date),
            oid::TIMESTAMP => Some(SqlType::Timestamp),
            oid::NULL => Some(SqlType::Null),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            SqlType::Int4 => "int4",
            SqlType::Int8 => "int8",
            SqlType::Numeric => "numeric",
            SqlType::Text => "text",
            SqlType::Varchar => "character varying",
            SqlType::Bool => "boolean",
            SqlType::Date => "date",
            SqlType::Timestamp => "timestamp without time zone",
            SqlType::Null => "void",
        }
    }
}

/// Rust representation of SQL values.
#[derive(Debug, Clone, PartialEq)]
pub enum SqlValue {
    Null,
    Int4(i32),
    Int8(i64),
    Numeric(f64),
    Text(String),
    Bool(bool),
    Date(String), // "YYYY-MM-DD"
    Timestamp(String), // "YYYY-MM-DD HH:MM:SS"
}

impl SqlValue {
    pub fn type_of(&self) -> SqlType {
        match self {
            SqlValue::Null => SqlType::Null,
            SqlValue::Int4(_) => SqlType::Int4,
            SqlValue::Int8(_) => SqlType::Int8,
            SqlValue::Numeric(_) => SqlType::Numeric,
            SqlValue::Text(_) => SqlType::Text,
            SqlValue::Bool(_) => SqlType::Bool,
            SqlValue::Date(_) => SqlType::Date,
            SqlValue::Timestamp(_) => SqlType::Timestamp,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            SqlValue::Text(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_i32(&self) -> Option<i32> {
        match self {
            SqlValue::Int4(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            SqlValue::Int8(n) => Some(*n),
            SqlValue::Int4(n) => Some(*n as i64),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            SqlValue::Numeric(f) => Some(*f),
            SqlValue::Int4(n) => Some(*n as f64),
            SqlValue::Int8(n) => Some(*n as f64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            SqlValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn to_string(&self) -> String {
        match self {
            SqlValue::Null => "NULL".to_string(),
            SqlValue::Int4(n) => n.to_string(),
            SqlValue::Int8(n) => n.to_string(),
            SqlValue::Numeric(f) => {
                if f.fract() == 0.0 {
                    format!("{:.1}", f)
                } else {
                    f.to_string()
                }
            }
            SqlValue::Text(s) => s.clone(),
            SqlValue::Bool(b) => (if *b { "true" } else { "false" }).to_string(),
            SqlValue::Date(s) => s.clone(),
            SqlValue::Timestamp(s) => s.clone(),
        }
    }

    pub fn to_json(&self) -> Value {
        match self {
            SqlValue::Null => Value::Null,
            SqlValue::Int4(n) => json!(n),
            SqlValue::Int8(n) => json!(n),
            SqlValue::Numeric(f) => {
                if f.is_finite() {
                    json!(f)
                } else {
                    Value::Null
                }
            }
            SqlValue::Text(s) => json!(s),
            SqlValue::Bool(b) => json!(b),
            SqlValue::Date(s) => json!(s),
            SqlValue::Timestamp(s) => json!(s),
        }
    }

    pub fn compare(&self, other: &SqlValue) -> Option<Ordering> {
        match (self, other) {
            (SqlValue::Null, SqlValue::Null) => Some(Ordering::Equal),
            (SqlValue::Null, _) => Some(Ordering::Less),
            (_, SqlValue::Null) => Some(Ordering::Greater),
            (SqlValue::Int4(a), SqlValue::Int4(b)) => Some(a.cmp(b)),
            (SqlValue::Int8(a), SqlValue::Int8(b)) => Some(a.cmp(b)),
            (SqlValue::Numeric(a), SqlValue::Numeric(b)) => a.partial_cmp(b),
            (SqlValue::Text(a), SqlValue::Text(b)) => Some(a.cmp(b)),
            (SqlValue::Bool(a), SqlValue::Bool(b)) => Some(a.cmp(b)),
            (SqlValue::Date(a), SqlValue::Date(b)) => Some(a.cmp(b)),
            (SqlValue::Timestamp(a), SqlValue::Timestamp(b)) => Some(a.cmp(b)),
            _ => None,
        }
    }

    pub fn coerce_to(&self, target: &SqlType) -> Result<SqlValue, String> {
        match (self, target) {
            (SqlValue::Null, _) => Ok(SqlValue::Null),
            (SqlValue::Int4(n), SqlType::Int8) => Ok(SqlValue::Int8(*n as i64)),
            (SqlValue::Int4(n), SqlType::Numeric) => Ok(SqlValue::Numeric(*n as f64)),
            (SqlValue::Int4(n), SqlType::Text) => Ok(SqlValue::Text(n.to_string())),
            (SqlValue::Int4(n), SqlType::Bool) => Ok(SqlValue::Bool(*n != 0)),
            (SqlValue::Int8(n), SqlType::Int4) => {
                if *n >= i32::MIN as i64 && *n <= i32::MAX as i64 {
                    Ok(SqlValue::Int4(*n as i32))
                } else {
                    Err("int8 out of int4 range".to_string())
                }
            }
            (SqlValue::Int8(n), SqlType::Numeric) => Ok(SqlValue::Numeric(*n as f64)),
            (SqlValue::Int8(n), SqlType::Text) => Ok(SqlValue::Text(n.to_string())),
            (SqlValue::Int8(n), SqlType::Bool) => Ok(SqlValue::Bool(*n != 0)),
            (SqlValue::Numeric(f), SqlType::Int4) => Ok(SqlValue::Int4(*f as i32)),
            (SqlValue::Numeric(f), SqlType::Int8) => Ok(SqlValue::Int8(*f as i64)),
            (SqlValue::Numeric(f), SqlType::Text) => Ok(SqlValue::Text(f.to_string())),
            (SqlValue::Text(s), SqlType::Int4) => s
                .parse::<i32>()
                .map(SqlValue::Int4)
                .map_err(|_| format!("cannot cast '{}' to int4", s)),
            (SqlValue::Text(s), SqlType::Int8) => s
                .parse::<i64>()
                .map(SqlValue::Int8)
                .map_err(|_| format!("cannot cast '{}' to int8", s)),
            (SqlValue::Text(s), SqlType::Numeric) => s
                .parse::<f64>()
                .map(SqlValue::Numeric)
                .map_err(|_| format!("cannot cast '{}' to numeric", s)),
            (SqlValue::Text(s), SqlType::Bool) => {
                let lower = s.to_lowercase();
                match lower.as_str() {
                    "t" | "true" | "y" | "yes" | "1" => Ok(SqlValue::Bool(true)),
                    "f" | "false" | "n" | "no" | "0" => Ok(SqlValue::Bool(false)),
                    _ => Err(format!("cannot cast '{}' to bool", s)),
                }
            }
            (SqlValue::Bool(b), SqlType::Int4) => Ok(SqlValue::Int4(if *b { 1 } else { 0 })),
            (SqlValue::Bool(b), SqlType::Int8) => Ok(SqlValue::Int8(if *b { 1 } else { 0 })),
            (SqlValue::Bool(b), SqlType::Text) => {
                Ok(SqlValue::Text(if *b { "true" } else { "false" }.to_string()))
            }
            (v, t) if v.type_of() == *t => Ok(v.clone()),
            _ => Err(format!("cannot coerce {:?} to {:?}", self.type_of(), target)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sqltype_oid_mapping() {
        assert_eq!(SqlType::Int4.oid(), oid::INT4);
        assert_eq!(SqlType::Int8.oid(), oid::INT8);
        assert_eq!(SqlType::Bool.oid(), oid::BOOL);
        assert_eq!(SqlType::Text.oid(), oid::TEXT);
    }

    #[test]
    fn test_sqlvalue_type_of() {
        assert_eq!(SqlValue::Int4(42).type_of(), SqlType::Int4);
        assert_eq!(SqlValue::Text("hello".to_string()).type_of(), SqlType::Text);
        assert_eq!(SqlValue::Bool(true).type_of(), SqlType::Bool);
        assert_eq!(SqlValue::Null.type_of(), SqlType::Null);
    }

    #[test]
    fn test_sqlvalue_coerce() {
        let v = SqlValue::Int4(42);
        let coerced = v.coerce_to(&SqlType::Int8).unwrap();
        assert_eq!(coerced, SqlValue::Int8(42i64));

        let text = SqlValue::Text("123".to_string());
        let coerced = text.coerce_to(&SqlType::Int4).unwrap();
        assert_eq!(coerced, SqlValue::Int4(123));
    }
}
