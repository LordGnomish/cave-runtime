// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Iceberg Schema — typed column list with stable field ids.
//!
//! Mirrors apache/iceberg-rust crates/iceberg/src/spec/schema.rs and
//! the spec at https://iceberg.apache.org/spec/#schemas.

use crate::table_format::iceberg::error::{IcebergError, IcebergResult};
use crate::table_format::iceberg::tenant::{default_tenant_id, validate_tenant_id};
use serde::{Deserialize, Serialize};

/// Iceberg primitive types (subset shipped here — bool/int/long/float/double/
/// string/binary/uuid/date/time/timestamp/timestamptz).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrimitiveType {
    Boolean,
    Int,
    Long,
    Float,
    Double,
    String,
    Binary,
    Uuid,
    Date,
    Time,
    Timestamp,
    Timestamptz,
}

impl PrimitiveType {
    /// Iceberg spec name as found in JSON metadata
    /// (apache/iceberg spec/Types — `boolean`, `int`, …).
    pub const fn spec_name(self) -> &'static str {
        match self {
            PrimitiveType::Boolean => "boolean",
            PrimitiveType::Int => "int",
            PrimitiveType::Long => "long",
            PrimitiveType::Float => "float",
            PrimitiveType::Double => "double",
            PrimitiveType::String => "string",
            PrimitiveType::Binary => "binary",
            PrimitiveType::Uuid => "uuid",
            PrimitiveType::Date => "date",
            PrimitiveType::Time => "time",
            PrimitiveType::Timestamp => "timestamp",
            PrimitiveType::Timestamptz => "timestamptz",
        }
    }
}

/// One Iceberg field — id + name + type + required flag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    pub id: i32,
    pub name: String,
    pub required: bool,
    #[serde(rename = "type")]
    pub field_type: PrimitiveType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
}

impl Field {
    pub fn required(id: i32, name: impl Into<String>, t: PrimitiveType) -> Self {
        Self {
            id,
            name: name.into(),
            required: true,
            field_type: t,
            doc: None,
        }
    }

    pub fn optional(id: i32, name: impl Into<String>, t: PrimitiveType) -> Self {
        Self {
            id,
            name: name.into(),
            required: false,
            field_type: t,
            doc: None,
        }
    }
}

/// Iceberg Schema — a struct of fields with a unique schema id.
///
/// `identifier_field_ids` is the row-uniqueness primary key
/// (apache/iceberg-rust spec/schema.rs `identifier_field_ids`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schema {
    pub schema_id: i32,
    pub fields: Vec<Field>,
    #[serde(default)]
    pub identifier_field_ids: Vec<i32>,
    #[serde(default = "default_tenant_id")]
    pub tenant_id: String,
}

impl Schema {
    pub fn new(schema_id: i32, fields: Vec<Field>) -> Self {
        Self {
            schema_id,
            fields,
            identifier_field_ids: Vec::new(),
            tenant_id: default_tenant_id(),
        }
    }

    pub fn with_tenant(mut self, tenant: impl Into<String>) -> Self {
        self.tenant_id = tenant.into();
        self
    }

    pub fn with_identifier_fields(mut self, ids: Vec<i32>) -> Self {
        self.identifier_field_ids = ids;
        self
    }

    pub fn field_by_id(&self, id: i32) -> Option<&Field> {
        self.fields.iter().find(|f| f.id == id)
    }

    pub fn field_by_name(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// Validate the schema:
    /// - tenant_id must be valid
    /// - field ids must be unique
    /// - field names must be unique
    /// - identifier_field_ids must reference existing required fields
    pub fn validate(&self) -> IcebergResult<()> {
        validate_tenant_id(&self.tenant_id)?;
        let mut seen_ids = std::collections::HashSet::new();
        let mut seen_names = std::collections::HashSet::new();
        for f in &self.fields {
            if !seen_ids.insert(f.id) {
                return Err(IcebergError::Schema(format!("duplicate field id {}", f.id)));
            }
            if !seen_names.insert(f.name.clone()) {
                return Err(IcebergError::Schema(format!(
                    "duplicate field name '{}'",
                    f.name
                )));
            }
        }
        for &id in &self.identifier_field_ids {
            let f = self.field_by_id(id).ok_or_else(|| {
                IcebergError::Schema(format!(
                    "identifier_field_ids references unknown field id {}",
                    id
                ))
            })?;
            if !f.required {
                return Err(IcebergError::Schema(format!(
                    "identifier field {} ('{}') must be required",
                    id, f.name
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_schema() -> Schema {
        Schema::new(
            1,
            vec![
                Field::required(1, "id", PrimitiveType::Long),
                Field::required(2, "name", PrimitiveType::String),
                Field::optional(3, "email", PrimitiveType::String),
            ],
        )
    }

    // ── PrimitiveType serde + spec names ───────────────────────────────────────

    #[test]
    fn primitive_serde_lowercase() {
        // citation: iceberg spec/Types — JSON uses lowercase names
        let j = serde_json::to_string(&PrimitiveType::Boolean).unwrap();
        assert_eq!(j, "\"boolean\"");
    }

    #[test]
    fn primitive_round_trip_all() {
        for t in [
            PrimitiveType::Boolean,
            PrimitiveType::Int,
            PrimitiveType::Long,
            PrimitiveType::Float,
            PrimitiveType::Double,
            PrimitiveType::String,
            PrimitiveType::Binary,
            PrimitiveType::Uuid,
            PrimitiveType::Date,
            PrimitiveType::Time,
            PrimitiveType::Timestamp,
            PrimitiveType::Timestamptz,
        ] {
            let j = serde_json::to_string(&t).unwrap();
            let back: PrimitiveType = serde_json::from_str(&j).unwrap();
            assert_eq!(back, t);
        }
    }

    #[test]
    fn primitive_spec_names() {
        assert_eq!(PrimitiveType::Long.spec_name(), "long");
        assert_eq!(PrimitiveType::Timestamptz.spec_name(), "timestamptz");
    }

    // ── Field constructors ─────────────────────────────────────────────────────

    #[test]
    fn field_required_constructor() {
        let f = Field::required(1, "id", PrimitiveType::Long);
        assert!(f.required);
        assert_eq!(f.name, "id");
        assert_eq!(f.id, 1);
    }

    #[test]
    fn field_optional_constructor() {
        let f = Field::optional(2, "email", PrimitiveType::String);
        assert!(!f.required);
    }

    #[test]
    fn field_serde_roundtrip() {
        let f = Field::required(1, "id", PrimitiveType::Long);
        let j = serde_json::to_string(&f).unwrap();
        let back: Field = serde_json::from_str(&j).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn field_doc_omitted_when_none() {
        let f = Field::required(1, "id", PrimitiveType::Long);
        let j = serde_json::to_string(&f).unwrap();
        assert!(!j.contains("doc"));
    }

    // ── Schema constructors + lookups ──────────────────────────────────────────

    #[test]
    fn schema_default_tenant() {
        let s = user_schema();
        assert_eq!(s.tenant_id, "default");
    }

    #[test]
    fn schema_with_tenant() {
        let s = user_schema().with_tenant("acme");
        assert_eq!(s.tenant_id, "acme");
    }

    #[test]
    fn schema_field_by_id() {
        let s = user_schema();
        assert_eq!(s.field_by_id(2).unwrap().name, "name");
        assert!(s.field_by_id(99).is_none());
    }

    #[test]
    fn schema_field_by_name() {
        let s = user_schema();
        assert_eq!(s.field_by_name("email").unwrap().id, 3);
        assert!(s.field_by_name("missing").is_none());
    }

    #[test]
    fn schema_with_identifier_fields() {
        let s = user_schema().with_identifier_fields(vec![1]);
        assert_eq!(s.identifier_field_ids, vec![1]);
    }

    // ── Schema validate ────────────────────────────────────────────────────────

    #[test]
    fn schema_validate_default_ok() {
        assert!(user_schema().validate().is_ok());
    }

    #[test]
    fn schema_validate_with_identifier_pk_ok() {
        let s = user_schema().with_identifier_fields(vec![1]);
        assert!(s.validate().is_ok());
    }

    #[test]
    fn schema_validate_duplicate_field_id_err() {
        let s = Schema::new(
            1,
            vec![
                Field::required(1, "a", PrimitiveType::Long),
                Field::required(1, "b", PrimitiveType::Long),
            ],
        );
        let e = s.validate().unwrap_err().to_string();
        assert!(e.contains("duplicate field id"));
    }

    #[test]
    fn schema_validate_duplicate_field_name_err() {
        let s = Schema::new(
            1,
            vec![
                Field::required(1, "a", PrimitiveType::Long),
                Field::required(2, "a", PrimitiveType::String),
            ],
        );
        let e = s.validate().unwrap_err().to_string();
        assert!(e.contains("duplicate field name"));
    }

    #[test]
    fn schema_validate_identifier_unknown_id_err() {
        let s = user_schema().with_identifier_fields(vec![999]);
        let e = s.validate().unwrap_err().to_string();
        assert!(e.contains("unknown"));
    }

    #[test]
    fn schema_validate_identifier_optional_field_err() {
        // identifier field must be required (PK can't be null)
        // citation: iceberg spec — identifier fields must be required
        let s = user_schema().with_identifier_fields(vec![3]); // email is optional
        let e = s.validate().unwrap_err().to_string();
        assert!(e.contains("must be required"));
    }

    #[test]
    fn schema_validate_invalid_tenant_err() {
        let mut s = user_schema();
        s.tenant_id = "INVALID".into();
        assert!(s.validate().is_err());
    }

    // ── Schema serde round-trip ────────────────────────────────────────────────

    #[test]
    fn schema_serde_roundtrip() {
        let s = user_schema()
            .with_tenant("acme")
            .with_identifier_fields(vec![1]);
        let j = serde_json::to_string(&s).unwrap();
        let back: Schema = serde_json::from_str(&j).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn schema_deserialize_omitted_tenant_defaults() {
        let j = r#"{"schema_id":1,"fields":[{"id":1,"name":"a","required":true,"type":"long"}]}"#;
        let s: Schema = serde_json::from_str(j).unwrap();
        assert_eq!(s.tenant_id, "default");
        assert_eq!(s.identifier_field_ids.len(), 0);
    }
}
