//! Schema registry — Avro, JSON Schema, and Protobuf schema management.
//!
//! Enforces compatibility policies between schema versions:
//!   * **BACKWARD**  — new schema can read data written by old schema.
//!   * **FORWARD**   — old schema can read data written by new schema.
//!   * **FULL**      — both directions.
//!   * **NONE**      — no compatibility checking.
//!
//! The registry stores schemas by *subject* (typically `<topic>-key` or
//! `<topic>-value`) and assigns a globally-unique integer ID to each version.

use crate::error::{StreamError, StreamResult};
use crate::models::{CompatibilityMode, Schema, SchemaType};
use crate::storage::StreamStorage;
use serde_json::Value as JsonValue;

// ─── Registry facade ─────────────────────────────────────────────────────────

pub struct SchemaRegistry<S: StreamStorage> {
    storage: S,
}

impl<S: StreamStorage> SchemaRegistry<S> {
    pub fn new(storage: S) -> Self {
        Self { storage }
    }

    // ─── Registration ────────────────────────────────────────────────────────

    /// Register a new schema version for a subject.
    ///
    /// If an identical schema (by fingerprint) already exists, the existing
    /// schema ID is returned without creating a duplicate.
    pub fn register(
        &self,
        subject: impl Into<String>,
        schema_type: SchemaType,
        definition: impl Into<String>,
    ) -> StreamResult<u32> {
        let subject = subject.into();
        let definition = definition.into();

        // Parse / validate the schema definition.
        validate_syntax(&schema_type, &definition)?;

        let fingerprint = fnv1a_64(definition.as_bytes());

        // Check for deduplication (same fingerprint → same schema).
        let existing_versions = self.storage.list_subject_versions(&subject)?;
        for version in existing_versions {
            if let Some(existing) = self.storage.get_schema_by_version(&subject, version)? {
                if existing.fingerprint == fingerprint {
                    return Ok(existing.id);
                }
            }
        }

        // Compatibility check against the latest schema for this subject.
        let latest = self.storage.get_latest_schema(&subject)?;
        let compat_mode = self.storage.get_subject_compat(&subject)?;

        if let Some(ref prev) = latest {
            check_compatibility(&compat_mode, prev, &definition, &schema_type)?;
        }

        let id = self.storage.next_schema_id()?;
        let version = latest.map(|s| s.version + 1).unwrap_or(1);

        let schema = Schema {
            id,
            subject: subject.clone(),
            version,
            schema_type,
            definition,
            fingerprint,
        };

        self.storage.register_schema(schema)?;
        Ok(id)
    }

    // ─── Lookup ──────────────────────────────────────────────────────────────

    pub fn get_by_id(&self, id: u32) -> StreamResult<Schema> {
        self.storage
            .get_schema(id)?
            .ok_or(StreamError::SchemaNotFound(id))
    }

    pub fn get_latest(&self, subject: &str) -> StreamResult<Schema> {
        self.storage
            .get_latest_schema(subject)?
            .ok_or_else(|| StreamError::SubjectNotFound(subject.into()))
    }

    pub fn get_version(&self, subject: &str, version: u32) -> StreamResult<Schema> {
        self.storage
            .get_schema_by_version(subject, version)?
            .ok_or_else(|| StreamError::SubjectNotFound(subject.into()))
    }

    pub fn list_versions(&self, subject: &str) -> StreamResult<Vec<u32>> {
        self.storage.list_subject_versions(subject)
    }

    pub fn list_subjects(&self) -> StreamResult<Vec<String>> {
        self.storage.list_subjects()
    }

    // ─── Delete ──────────────────────────────────────────────────────────────

    pub fn delete_version(&self, subject: &str, version: u32) -> StreamResult<()> {
        self.storage.delete_schema(subject, version)
    }

    // ─── Compatibility config ────────────────────────────────────────────────

    pub fn set_compatibility(
        &self,
        subject: &str,
        mode: CompatibilityMode,
    ) -> StreamResult<()> {
        self.storage.set_subject_compat(subject, mode)
    }

    pub fn get_compatibility(&self, subject: &str) -> StreamResult<CompatibilityMode> {
        self.storage.get_subject_compat(subject)
    }

    // ─── Validation ──────────────────────────────────────────────────────────

    /// Check whether `candidate_definition` is compatible with the current
    /// latest schema for `subject` under the subject's configured mode.
    pub fn check_compatibility(
        &self,
        subject: &str,
        schema_type: &SchemaType,
        candidate_definition: &str,
    ) -> StreamResult<CompatibilityCheckResult> {
        validate_syntax(schema_type, candidate_definition)?;

        let Some(latest) = self.storage.get_latest_schema(subject)? else {
            return Ok(CompatibilityCheckResult {
                compatible: true,
                messages: vec!["No existing schema; any schema is compatible".into()],
            });
        };

        let mode = self.storage.get_subject_compat(subject)?;

        match check_compatibility(&mode, &latest, candidate_definition, schema_type) {
            Ok(()) => Ok(CompatibilityCheckResult {
                compatible: true,
                messages: Vec::new(),
            }),
            Err(StreamError::SchemaCompatibility(msg)) => Ok(CompatibilityCheckResult {
                compatible: false,
                messages: vec![msg],
            }),
            Err(e) => Err(e),
        }
    }
}

// ─── Compatibility checking ───────────────────────────────────────────────────

/// Check whether `candidate` is compatible with `existing` under `mode`.
fn check_compatibility(
    mode: &CompatibilityMode,
    existing: &Schema,
    candidate_definition: &str,
    candidate_type: &SchemaType,
) -> StreamResult<()> {
    if *mode == CompatibilityMode::None {
        return Ok(());
    }

    // Type mismatch is always incompatible.
    if &existing.schema_type != candidate_type {
        return Err(StreamError::SchemaCompatibility(format!(
            "Schema type mismatch: existing={:?}, candidate={:?}",
            existing.schema_type, candidate_type
        )));
    }

    let backward = matches!(
        mode,
        CompatibilityMode::Backward
            | CompatibilityMode::BackwardTransitive
            | CompatibilityMode::Full
            | CompatibilityMode::FullTransitive
    );
    let forward = matches!(
        mode,
        CompatibilityMode::Forward
            | CompatibilityMode::ForwardTransitive
            | CompatibilityMode::Full
            | CompatibilityMode::FullTransitive
    );

    match existing.schema_type {
        SchemaType::JsonSchema => {
            let existing_json: JsonValue =
                serde_json::from_str(&existing.definition).map_err(|e| {
                    StreamError::SchemaValidation(format!("Existing schema invalid JSON: {e}"))
                })?;
            let candidate_json: JsonValue =
                serde_json::from_str(candidate_definition).map_err(|e| {
                    StreamError::SchemaValidation(format!("Candidate schema invalid JSON: {e}"))
                })?;
            check_json_schema_compatibility(
                &existing_json,
                &candidate_json,
                backward,
                forward,
            )
        }
        SchemaType::Avro => {
            // For Avro we do structural compatibility checking using the JSON
            // representation of the Avro schema.
            let existing_json: JsonValue =
                serde_json::from_str(&existing.definition).map_err(|e| {
                    StreamError::SchemaValidation(format!("Existing Avro schema invalid JSON: {e}"))
                })?;
            let candidate_json: JsonValue =
                serde_json::from_str(candidate_definition).map_err(|e| {
                    StreamError::SchemaValidation(format!(
                        "Candidate Avro schema invalid JSON: {e}"
                    ))
                })?;
            check_avro_compatibility(&existing_json, &candidate_json, backward, forward)
        }
        SchemaType::Protobuf => {
            // Protobuf compatibility: field numbers must be stable.
            check_protobuf_compatibility(
                &existing.definition,
                candidate_definition,
                backward,
                forward,
            )
        }
    }
}

// ─── JSON Schema compatibility ────────────────────────────────────────────────

fn check_json_schema_compatibility(
    existing: &JsonValue,
    candidate: &JsonValue,
    backward: bool,
    forward: bool,
) -> StreamResult<()> {
    // Required field analysis.
    let existing_required = json_required_fields(existing);
    let candidate_required = json_required_fields(candidate);

    if backward {
        // New schema must be able to read old data:
        // new schema MUST NOT add required fields (old data won't have them).
        for field in &candidate_required {
            if !existing_required.contains(field) {
                return Err(StreamError::SchemaCompatibility(format!(
                    "BACKWARD incompatible: candidate adds required field '{field}' \
                     not present in existing schema"
                )));
            }
        }
    }

    if forward {
        // Old schema must be able to read new data:
        // old schema MUST NOT have required fields missing in new schema.
        for field in &existing_required {
            if !json_has_field(candidate, field) {
                return Err(StreamError::SchemaCompatibility(format!(
                    "FORWARD incompatible: candidate removes field '{field}' \
                     that is required by existing schema"
                )));
            }
        }
    }

    Ok(())
}

fn json_required_fields(schema: &JsonValue) -> Vec<String> {
    schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn json_has_field(schema: &JsonValue, field: &str) -> bool {
    schema
        .get("properties")
        .and_then(|p| p.as_object())
        .map(|props| props.contains_key(field))
        .unwrap_or(false)
}

// ─── Avro compatibility ────────────────────────────────────────────────────────

fn check_avro_compatibility(
    existing: &JsonValue,
    candidate: &JsonValue,
    backward: bool,
    forward: bool,
) -> StreamResult<()> {
    let existing_fields = avro_fields(existing);
    let candidate_fields = avro_fields(candidate);

    if backward {
        // New schema must read old data → no new fields without defaults.
        for (name, field) in &candidate_fields {
            if !existing_fields.contains_key(name.as_str()) && field.get("default").is_none() {
                return Err(StreamError::SchemaCompatibility(format!(
                    "BACKWARD incompatible: new field '{name}' has no default value"
                )));
            }
        }
    }

    if forward {
        // Old schema must read new data → no removed fields without defaults.
        for (name, field) in &existing_fields {
            if !candidate_fields.contains_key(name.as_str()) && field.get("default").is_none() {
                return Err(StreamError::SchemaCompatibility(format!(
                    "FORWARD incompatible: existing field '{name}' removed without default"
                )));
            }
        }
    }

    Ok(())
}

fn avro_fields(schema: &JsonValue) -> std::collections::HashMap<String, &JsonValue> {
    let mut map = std::collections::HashMap::new();
    if let Some(fields) = schema.get("fields").and_then(|f| f.as_array()) {
        for field in fields {
            if let Some(name) = field.get("name").and_then(|n| n.as_str()) {
                map.insert(name.to_string(), field);
            }
        }
    }
    map
}

// ─── Protobuf compatibility ───────────────────────────────────────────────────

fn check_protobuf_compatibility(
    existing: &str,
    candidate: &str,
    backward: bool,
    forward: bool,
) -> StreamResult<()> {
    // Extract (field_number, field_name, type) triples from the text IDL.
    let existing_fields = parse_proto_fields(existing);
    let candidate_fields = parse_proto_fields(candidate);

    if backward || forward {
        // Field numbers must remain stable (same number → same type).
        for (num, (name, typ)) in &existing_fields {
            if let Some((cname, ctyp)) = candidate_fields.get(num) {
                if typ != ctyp {
                    return Err(StreamError::SchemaCompatibility(format!(
                        "Field {num} ({name}) type changed from '{typ}' to '{ctyp}'"
                    )));
                }
                if cname != name {
                    // Name change is allowed (wire format uses numbers).
                    let _ = cname;
                }
            } else if forward {
                // Field removed — old schema can't read new data.
                return Err(StreamError::SchemaCompatibility(format!(
                    "FORWARD incompatible: field {num} ({name}) removed"
                )));
            }
        }
    }

    Ok(())
}

/// Very minimal proto3 field parser: extracts `field_number → (name, type)`.
fn parse_proto_fields(proto: &str) -> std::collections::HashMap<u32, (String, String)> {
    let mut map = std::collections::HashMap::new();
    for line in proto.lines() {
        let line = line.trim();
        // Pattern: `<type> <name> = <number>;`
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 4 && parts[parts.len() - 1].ends_with(';') {
            let typ = parts[0];
            let name = parts[1];
            let num_part = parts[parts.len() - 1].trim_end_matches(';');
            let num_part = parts[parts.len() - 2].trim_end_matches('=');
            if let Ok(num) = num_part.parse::<u32>() {
                map.insert(num, (name.to_string(), typ.to_string()));
            }
        }
    }
    map
}

// ─── Syntax validation ────────────────────────────────────────────────────────

fn validate_syntax(schema_type: &SchemaType, definition: &str) -> StreamResult<()> {
    match schema_type {
        SchemaType::Avro | SchemaType::JsonSchema => {
            serde_json::from_str::<JsonValue>(definition).map_err(|e| {
                StreamError::SchemaValidation(format!("Invalid JSON in schema: {e}"))
            })?;
            Ok(())
        }
        SchemaType::Protobuf => {
            if definition.trim().is_empty() {
                return Err(StreamError::SchemaValidation(
                    "Protobuf schema must not be empty".into(),
                ));
            }
            Ok(())
        }
    }
}

// ─── FNV-1a 64-bit ───────────────────────────────────────────────────────────

fn fnv1a_64(data: &[u8]) -> u64 {
    let mut hash: u64 = 14_695_981_039_346_656_037;
    for byte in data {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    hash
}

// ─── Result types ─────────────────────────────────────────────────────────────

#[derive(Debug, serde::Serialize)]
pub struct CompatibilityCheckResult {
    pub compatible: bool,
    pub messages: Vec<String>,
}
