// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Iceberg Schema (v2/v3 spec).
//!
//! Upstream:
//! * `crates/iceberg/src/spec/schema.rs` — `Schema` + `SchemaBuilder`
//! * `crates/iceberg/src/spec/datatypes.rs` — `Type`, `NestedField`, primitives
//!
//! Mirrors the v2 schema layout: a list of `NestedField`s, each carrying
//! its own field-id, name, required-ness, doc, and an inner `Type`.
//! Iceberg distinguishes types into Primitive / Struct / List / Map.
//! Field-ids are stable across schema evolution; we mirror that
//! invariant in the builder by demanding the caller assign them.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Type {
    Primitive(PrimitiveType),
    Struct(StructType),
    List(ListType),
    Map(MapType),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PrimitiveType {
    Boolean,
    Int,
    Long,
    Float,
    Double,
    Date,
    Time,
    Timestamp,
    Timestamptz,
    String,
    Uuid,
    Fixed(u32),
    Binary,
    Decimal { precision: u32, scale: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StructType {
    pub fields: Vec<NestedField>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ListType {
    pub element_id: i32,
    pub element_required: bool,
    pub element: Box<Type>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MapType {
    pub key_id: i32,
    pub value_id: i32,
    pub value_required: bool,
    pub key: Box<Type>,
    pub value: Box<Type>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NestedField {
    pub id: i32,
    pub name: String,
    pub required: bool,
    pub field_type: Type,
    pub doc: Option<String>,
}

impl NestedField {
    pub fn required(id: i32, name: impl Into<String>, ty: Type) -> Self {
        Self {
            id,
            name: name.into(),
            required: true,
            field_type: ty,
            doc: None,
        }
    }

    pub fn optional(id: i32, name: impl Into<String>, ty: Type) -> Self {
        Self {
            id,
            name: name.into(),
            required: false,
            field_type: ty,
            doc: None,
        }
    }

    pub fn with_doc(mut self, doc: impl Into<String>) -> Self {
        self.doc = Some(doc.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Schema {
    /// Iceberg schema-id. Defaults to 0 for the initial schema.
    pub schema_id: i32,
    /// Field-ids that, when combined, uniquely identify a row.
    pub identifier_field_ids: Vec<i32>,
    pub fields: Vec<NestedField>,
}

impl Schema {
    pub fn builder() -> SchemaBuilder {
        SchemaBuilder::default()
    }

    /// Look up a top-level field by name.
    pub fn field_by_name(&self, name: &str) -> Option<&NestedField> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// Look up a top-level field by its stable field-id.
    pub fn field_by_id(&self, id: i32) -> Option<&NestedField> {
        self.fields.iter().find(|f| f.id == id)
    }

    /// Strict-check that every identifier-field-id resolves to an
    /// existing required field — Iceberg requires identifier columns
    /// to be non-null.
    pub fn validate(&self) -> Result<()> {
        for id in &self.identifier_field_ids {
            let f = self
                .field_by_id(*id)
                .ok_or_else(|| Error::InvalidSchema(format!("identifier field {} missing", id)))?;
            if !f.required {
                return Err(Error::InvalidSchema(format!(
                    "identifier field {} ('{}') must be required",
                    id, f.name
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct SchemaBuilder {
    schema_id: i32,
    identifier_field_ids: Vec<i32>,
    fields: Vec<NestedField>,
}

impl SchemaBuilder {
    pub fn schema_id(mut self, id: i32) -> Self {
        self.schema_id = id;
        self
    }

    pub fn identifier_field_ids(mut self, ids: Vec<i32>) -> Self {
        self.identifier_field_ids = ids;
        self
    }

    pub fn with_field(mut self, f: NestedField) -> Self {
        self.fields.push(f);
        self
    }

    pub fn build(self) -> Result<Schema> {
        // Detect duplicate field-ids.
        let mut seen = std::collections::HashSet::new();
        for f in &self.fields {
            if !seen.insert(f.id) {
                return Err(Error::InvalidSchema(format!(
                    "duplicate field-id {} in schema",
                    f.id
                )));
            }
        }
        let s = Schema {
            schema_id: self.schema_id,
            identifier_field_ids: self.identifier_field_ids,
            fields: self.fields,
        };
        s.validate()?;
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_field_required_and_optional() {
        let r = NestedField::required(1, "id", Type::Primitive(PrimitiveType::Long));
        assert!(r.required);
        let o = NestedField::optional(2, "name", Type::Primitive(PrimitiveType::String));
        assert!(!o.required);
    }

    #[test]
    fn schema_builder_succeeds_for_unique_ids() {
        let schema = Schema::builder()
            .with_field(NestedField::required(
                1,
                "id",
                Type::Primitive(PrimitiveType::Long),
            ))
            .with_field(NestedField::optional(
                2,
                "name",
                Type::Primitive(PrimitiveType::String),
            ))
            .identifier_field_ids(vec![1])
            .build()
            .unwrap();
        assert_eq!(schema.fields.len(), 2);
        assert_eq!(schema.field_by_name("id").unwrap().id, 1);
        assert_eq!(schema.field_by_id(2).unwrap().name, "name");
    }

    #[test]
    fn schema_builder_rejects_duplicate_ids() {
        let r = Schema::builder()
            .with_field(NestedField::required(
                1,
                "id",
                Type::Primitive(PrimitiveType::Long),
            ))
            .with_field(NestedField::required(
                1,
                "x",
                Type::Primitive(PrimitiveType::Long),
            ))
            .build();
        assert!(matches!(r, Err(Error::InvalidSchema(_))));
    }

    #[test]
    fn schema_validate_requires_identifier_to_be_required() {
        let r = Schema::builder()
            .with_field(NestedField::optional(
                1,
                "id",
                Type::Primitive(PrimitiveType::Long),
            ))
            .identifier_field_ids(vec![1])
            .build();
        assert!(matches!(r, Err(Error::InvalidSchema(_))));
    }

    #[test]
    fn schema_validate_rejects_missing_identifier() {
        let r = Schema::builder()
            .with_field(NestedField::required(
                1,
                "id",
                Type::Primitive(PrimitiveType::Long),
            ))
            .identifier_field_ids(vec![99])
            .build();
        assert!(matches!(r, Err(Error::InvalidSchema(_))));
    }

    #[test]
    fn primitive_types_round_trip_json() {
        let p = PrimitiveType::Decimal {
            precision: 10,
            scale: 2,
        };
        let j = serde_json::to_string(&p).unwrap();
        let back: PrimitiveType = serde_json::from_str(&j).unwrap();
        assert_eq!(p, back);
    }
}
