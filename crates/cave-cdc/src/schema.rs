//! Schema evolution + compatibility checks (Avro / Protobuf / JSON).
//!
//! Cite: debezium `EventDispatcher` schema-emission flow + Confluent
//! Schema Registry compatibility table (BACKWARD / FORWARD / FULL /
//! NONE / TRANSITIVE variants). cave keeps the Confluent semantics
//! verbatim because every cave-cdc consumer that wants to interoperate
//! with the Java ecosystem will read against the same rules.

use crate::error::{CdcError, CdcResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SchemaFormat { Avro, Protobuf, Json }

/// Cite: Confluent Schema Registry compatibility levels documentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Compatibility {
    None,
    Backward,
    BackwardTransitive,
    Forward,
    ForwardTransitive,
    Full,
    FullTransitive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schema {
    pub subject: String,
    pub format: SchemaFormat,
    pub version: u32,
    /// Field-name → field-type map. cave models a flat schema for the
    /// scaffold; nested records land in a follow-up batch.
    pub fields: Vec<FieldDef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldDef {
    pub name: String,
    pub field_type: String,        // "string", "int64", "bytes", ...
    pub nullable: bool,
    pub default: Option<serde_json::Value>,
}

impl Schema {
    /// Cite: Avro schema specification + Confluent BACKWARD rules:
    /// READER (new) MAY add OPTIONAL fields and remove fields with
    /// defaults. Adding a REQUIRED field, or removing one without a
    /// default, breaks BACKWARD compatibility.
    pub fn check_backward(reader: &Schema, writer: &Schema) -> CdcResult<()> {
        let writer_names: HashMap<&str, &FieldDef> =
            writer.fields.iter().map(|f| (f.name.as_str(), f)).collect();
        for r in &reader.fields {
            if let Some(w) = writer_names.get(r.name.as_str()) {
                if w.field_type != r.field_type {
                    return Err(CdcError::SchemaIncompatibility(format!(
                        "field '{}' type changed: writer {} → reader {}",
                        r.name, w.field_type, r.field_type,
                    )));
                }
            } else {
                // New required field on reader — needs a default.
                if !r.nullable && r.default.is_none() {
                    return Err(CdcError::SchemaIncompatibility(format!(
                        "reader added required field '{}' with no default",
                        r.name,
                    )));
                }
            }
        }
        Ok(())
    }

    /// Cite: Confluent FORWARD rules — WRITER (new) MAY add OPTIONAL
    /// fields and remove fields with defaults relative to the READER.
    pub fn check_forward(writer: &Schema, reader: &Schema) -> CdcResult<()> {
        let reader_names: HashMap<&str, &FieldDef> =
            reader.fields.iter().map(|f| (f.name.as_str(), f)).collect();
        for w in &writer.fields {
            if let Some(r) = reader_names.get(w.name.as_str()) {
                if w.field_type != r.field_type {
                    return Err(CdcError::SchemaIncompatibility(format!(
                        "field '{}' type changed: writer {} → reader {}",
                        w.name, w.field_type, r.field_type,
                    )));
                }
            } else if !w.nullable && w.default.is_none() {
                return Err(CdcError::SchemaIncompatibility(format!(
                    "writer added required field '{}' with no default",
                    w.name,
                )));
            }
        }
        Ok(())
    }

    /// Cite: Confluent FULL = BACKWARD ∧ FORWARD.
    pub fn check_full(a: &Schema, b: &Schema) -> CdcResult<()> {
        Self::check_backward(a, b)?;
        Self::check_forward(b, a)
    }
}

/// Cite: Confluent Schema Registry — `(subject, version)` is the
/// canonical addressing. cave's registry is in-memory + tenant-scoped.
#[derive(Debug, Default)]
pub struct SchemaRegistry {
    pub tenant_id: String,
    pub compatibility: Compatibility,
    /// (subject, version) → Schema. Versions are 1-based.
    schemas: HashMap<String, Vec<Schema>>,
}

impl Default for Compatibility {
    fn default() -> Self { Self::Backward }  // Confluent default.
}

impl SchemaRegistry {
    pub fn new(tenant_id: impl Into<String>, compatibility: Compatibility) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            compatibility,
            schemas: HashMap::new(),
        }
    }

    /// Cite: Confluent `register` API — when a schema is registered
    /// against an existing subject, the registry checks compatibility
    /// against the latest version.
    pub fn register(&mut self, schema: Schema) -> CdcResult<u32> {
        let next_version = self.schemas.get(&schema.subject)
            .map(|v| v.len() as u32 + 1)
            .unwrap_or(1);

        if let Some(latest) = self.schemas.get(&schema.subject)
            .and_then(|v| v.last())
            .cloned()
        {
            self.check_compat(&latest, &schema)?;
        }

        let mut s = schema;
        s.version = next_version;
        self.schemas.entry(s.subject.clone()).or_default().push(s);
        Ok(next_version)
    }

    fn check_compat(&self, prev: &Schema, next: &Schema) -> CdcResult<()> {
        use Compatibility::*;
        match self.compatibility {
            None                  => Ok(()),
            Backward | BackwardTransitive   => Schema::check_backward(next, prev),
            Forward  | ForwardTransitive    => Schema::check_forward(next, prev),
            Full     | FullTransitive       => Schema::check_full(next, prev),
        }
    }

    pub fn latest(&self, subject: &str) -> Option<&Schema> {
        self.schemas.get(subject).and_then(|v| v.last())
    }

    pub fn get_version(&self, subject: &str, version: u32) -> Option<&Schema> {
        self.schemas.get(subject)
            .and_then(|v| v.get(version.saturating_sub(1) as usize))
    }

    pub fn version_count(&self, subject: &str) -> u32 {
        self.schemas.get(subject).map(|v| v.len() as u32).unwrap_or(0)
    }
}
