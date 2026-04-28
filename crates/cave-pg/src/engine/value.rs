//! Scalar values + types used by the catalog, storage, and executor.
//!
//! Each `ScalarType` maps to a PostgreSQL type OID (see `crate::wire::pg_types`).
//! `ScalarValue` is the in-memory representation with a `to_text()` helper for
//! the wire-protocol text format.

use crate::wire::pg_types;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScalarType {
    Bool,
    Int2,
    Int4,
    Int8,
    Float4,
    Float8,
    Text,
    Varchar,
    Numeric,
    Uuid,
    Json,
    Jsonb,
    Timestamp,
    TimestampTz,
    Date,
}

impl ScalarType {
    pub fn oid(self) -> u32 {
        match self {
            Self::Bool => pg_types::BOOL,
            Self::Int2 => pg_types::INT2,
            Self::Int4 => pg_types::INT4,
            Self::Int8 => pg_types::INT8,
            Self::Float4 => pg_types::FLOAT4,
            Self::Float8 => pg_types::FLOAT8,
            Self::Text => pg_types::TEXT,
            Self::Varchar => pg_types::VARCHAR,
            Self::Numeric => pg_types::NUMERIC,
            Self::Uuid => pg_types::UUID,
            Self::Json => pg_types::JSON,
            Self::Jsonb => pg_types::JSONB,
            Self::Timestamp => pg_types::TIMESTAMP,
            Self::TimestampTz => pg_types::TIMESTAMPTZ,
            Self::Date => pg_types::DATE,
        }
    }

    pub fn parse_sql_type(name: &str) -> Option<Self> {
        let s = name.to_ascii_uppercase();
        Some(match s.as_str() {
            "BOOL" | "BOOLEAN" => Self::Bool,
            "INT2" | "SMALLINT" => Self::Int2,
            "INT" | "INT4" | "INTEGER" => Self::Int4,
            "INT8" | "BIGINT" => Self::Int8,
            "FLOAT4" | "REAL" => Self::Float4,
            "FLOAT8" | "DOUBLE" | "DOUBLE PRECISION" => Self::Float8,
            "TEXT" => Self::Text,
            "VARCHAR" => Self::Varchar,
            "NUMERIC" | "DECIMAL" => Self::Numeric,
            "UUID" => Self::Uuid,
            "JSON" => Self::Json,
            "JSONB" => Self::Jsonb,
            "TIMESTAMP" => Self::Timestamp,
            "TIMESTAMPTZ" | "TIMESTAMP WITH TIME ZONE" => Self::TimestampTz,
            "DATE" => Self::Date,
            _ => return None,
        })
    }

    pub fn type_size(self) -> i16 {
        match self {
            Self::Bool => 1,
            Self::Int2 => 2,
            Self::Int4 => 4,
            Self::Int8 => 8,
            Self::Float4 => 4,
            Self::Float8 => 8,
            _ => -1,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScalarValue {
    Null,
    Bool(bool),
    Int2(i16),
    Int4(i32),
    Int8(i64),
    Float4(f32),
    Float8(f64),
    Text(String),
}

impl ScalarValue {
    pub fn to_text(&self) -> Option<String> {
        match self {
            Self::Null => None,
            Self::Bool(true) => Some("t".into()),
            Self::Bool(false) => Some("f".into()),
            Self::Int2(v) => Some(v.to_string()),
            Self::Int4(v) => Some(v.to_string()),
            Self::Int8(v) => Some(v.to_string()),
            Self::Float4(v) => Some(v.to_string()),
            Self::Float8(v) => Some(v.to_string()),
            Self::Text(s) => Some(s.clone()),
        }
    }

    pub fn type_of(&self) -> Option<ScalarType> {
        Some(match self {
            Self::Null => return None,
            Self::Bool(_) => ScalarType::Bool,
            Self::Int2(_) => ScalarType::Int2,
            Self::Int4(_) => ScalarType::Int4,
            Self::Int8(_) => ScalarType::Int8,
            Self::Float4(_) => ScalarType::Float4,
            Self::Float8(_) => ScalarType::Float8,
            Self::Text(_) => ScalarType::Text,
        })
    }

    pub fn parse_text(ty: ScalarType, text: &str) -> Option<Self> {
        Some(match ty {
            ScalarType::Bool => match text.to_ascii_lowercase().as_str() {
                "t" | "true" | "1" | "yes" | "on" => Self::Bool(true),
                "f" | "false" | "0" | "no" | "off" => Self::Bool(false),
                _ => return None,
            },
            ScalarType::Int2 => Self::Int2(text.parse().ok()?),
            ScalarType::Int4 => Self::Int4(text.parse().ok()?),
            ScalarType::Int8 => Self::Int8(text.parse().ok()?),
            ScalarType::Float4 => Self::Float4(text.parse().ok()?),
            ScalarType::Float8 => Self::Float8(text.parse().ok()?),
            ScalarType::Text | ScalarType::Varchar => Self::Text(text.to_string()),
            _ => Self::Text(text.to_string()),
        })
    }

    pub fn cmp_loose(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        match (self, other) {
            (Self::Null, Self::Null) => Ordering::Equal,
            (Self::Null, _) => Ordering::Less,
            (_, Self::Null) => Ordering::Greater,
            (Self::Bool(a), Self::Bool(b)) => a.cmp(b),
            (Self::Int2(a), Self::Int2(b)) => a.cmp(b),
            (Self::Int4(a), Self::Int4(b)) => a.cmp(b),
            (Self::Int8(a), Self::Int8(b)) => a.cmp(b),
            (Self::Int4(a), Self::Int8(b)) => (*a as i64).cmp(b),
            (Self::Int8(a), Self::Int4(b)) => a.cmp(&(*b as i64)),
            (Self::Int2(a), Self::Int4(b)) => (*a as i32).cmp(b),
            (Self::Int4(a), Self::Int2(b)) => a.cmp(&(*b as i32)),
            (Self::Float4(a), Self::Float4(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
            (Self::Float8(a), Self::Float8(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
            (Self::Text(a), Self::Text(b)) => a.cmp(b),
            _ => Ordering::Equal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_oids_match_pg_types() {
        assert_eq!(ScalarType::Int4.oid(), pg_types::INT4);
        assert_eq!(ScalarType::Text.oid(), pg_types::TEXT);
        assert_eq!(ScalarType::Jsonb.oid(), pg_types::JSONB);
    }

    #[test]
    fn parse_sql_type_handles_aliases() {
        assert_eq!(ScalarType::parse_sql_type("INT"), Some(ScalarType::Int4));
        assert_eq!(ScalarType::parse_sql_type("integer"), Some(ScalarType::Int4));
        assert_eq!(ScalarType::parse_sql_type("BIGINT"), Some(ScalarType::Int8));
        assert_eq!(ScalarType::parse_sql_type("TEXT"), Some(ScalarType::Text));
        assert_eq!(ScalarType::parse_sql_type("nope"), None);
    }

    #[test]
    fn type_size_matches_postgres() {
        assert_eq!(ScalarType::Int4.type_size(), 4);
        assert_eq!(ScalarType::Int8.type_size(), 8);
        assert_eq!(ScalarType::Text.type_size(), -1);
    }

    #[test]
    fn to_text_bool_uses_t_f() {
        assert_eq!(ScalarValue::Bool(true).to_text().as_deref(), Some("t"));
        assert_eq!(ScalarValue::Bool(false).to_text().as_deref(), Some("f"));
    }

    #[test]
    fn to_text_null_returns_none() {
        assert_eq!(ScalarValue::Null.to_text(), None);
    }

    #[test]
    fn parse_text_int4_round_trip() {
        let v = ScalarValue::parse_text(ScalarType::Int4, "42").unwrap();
        assert_eq!(v, ScalarValue::Int4(42));
        assert_eq!(v.to_text().as_deref(), Some("42"));
    }

    #[test]
    fn parse_text_bool_accepts_aliases() {
        assert_eq!(
            ScalarValue::parse_text(ScalarType::Bool, "TRUE"),
            Some(ScalarValue::Bool(true))
        );
        assert_eq!(
            ScalarValue::parse_text(ScalarType::Bool, "0"),
            Some(ScalarValue::Bool(false))
        );
    }

    #[test]
    fn cmp_loose_orders_int_mixed_widths() {
        use std::cmp::Ordering;
        assert_eq!(
            ScalarValue::Int4(3).cmp_loose(&ScalarValue::Int8(5)),
            Ordering::Less
        );
    }

    #[test]
    fn cmp_loose_null_is_less() {
        use std::cmp::Ordering;
        assert_eq!(
            ScalarValue::Null.cmp_loose(&ScalarValue::Int4(0)),
            Ordering::Less
        );
    }

    #[test]
    fn type_of_returns_scalar_type() {
        assert_eq!(ScalarValue::Int4(0).type_of(), Some(ScalarType::Int4));
        assert_eq!(ScalarValue::Null.type_of(), None);
    }
}
