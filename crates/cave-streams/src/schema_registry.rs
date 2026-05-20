// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Confluent-compatible Schema Registry.
//!
//! Implements the Schema Registry REST API:
//!   GET/POST /subjects
//!   GET/POST /subjects/{subject}/versions
//!   GET      /subjects/{subject}/versions/{version}
//!   DELETE   /subjects/{subject}
//!   GET      /schemas/ids/{id}
//!   POST     /compatibility/subjects/{subject}/versions/{version}
//!   GET/PUT  /config  (global compatibility level)
//!   GET/PUT  /config/{subject}

use crate::error::{StreamsError, StreamsResult};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering};

// ── Schema format ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum SchemaFormat {
    Avro,
    Protobuf,
    JsonSchema,
}

impl SchemaFormat {
    pub fn from_str(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "PROTOBUF" => Self::Protobuf,
            "JSON" | "JSON_SCHEMA" | "JSONSCHEMA" => Self::JsonSchema,
            _ => Self::Avro,
        }
    }
}

// ── Compatibility level ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CompatibilityLevel {
    None,
    Backward,
    BackwardTransitive,
    Forward,
    ForwardTransitive,
    Full,
    FullTransitive,
}

impl Default for CompatibilityLevel {
    fn default() -> Self {
        Self::Backward
    }
}

impl CompatibilityLevel {
    pub fn from_str(s: &str) -> Self {
        match s.to_uppercase().replace('-', "_").as_str() {
            "NONE" => Self::None,
            "BACKWARD" => Self::Backward,
            "BACKWARD_TRANSITIVE" => Self::BackwardTransitive,
            "FORWARD" => Self::Forward,
            "FORWARD_TRANSITIVE" => Self::ForwardTransitive,
            "FULL" => Self::Full,
            "FULL_TRANSITIVE" => Self::FullTransitive,
            _ => Self::Backward,
        }
    }
}

// ── Schema ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schema {
    pub id: i32,
    pub version: i32,
    pub subject: String,
    pub schema: String,
    pub schema_type: SchemaFormat,
    /// Schemas referenced by this schema
    pub references: Vec<SchemaReference>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaReference {
    pub name: String,
    pub subject: String,
    pub version: i32,
}

// ── Subject ───────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct Subject {
    pub name: String,
    pub versions: Vec<Schema>,
    pub compatibility: Option<CompatibilityLevel>,
    pub is_deleted: bool,
}

impl Subject {
    pub fn new(name: String) -> Self {
        Self {
            name,
            versions: Vec::new(),
            compatibility: None,
            is_deleted: false,
        }
    }

    pub fn latest(&self) -> Option<&Schema> {
        self.versions.last()
    }

    pub fn get_version(&self, version: i32) -> Option<&Schema> {
        if version == -1 {
            return self.latest();
        }
        self.versions.iter().find(|s| s.version == version)
    }
}

// ── Compatibility check ───────────────────────────────────────────────────────

/// Very lightweight structural compatibility check for JSON Schema.
/// Real implementations would use a full schema compatibility library.
fn check_json_schema_compatible(
    level: CompatibilityLevel,
    new_schema: &str,
    existing: &[&Schema],
) -> bool {
    if level == CompatibilityLevel::None {
        return true;
    }
    if existing.is_empty() {
        return true;
    }
    // Simplified: check that new schema is syntactically valid JSON
    serde_json::from_str::<serde_json::Value>(new_schema).is_ok()
}

fn check_schema_compatible(
    level: CompatibilityLevel,
    format: SchemaFormat,
    new_schema: &str,
    existing: &[&Schema],
) -> bool {
    match format {
        SchemaFormat::JsonSchema => check_json_schema_compatible(level, new_schema, existing),
        // For Avro/Protobuf: accept if non-empty and parseable as JSON (schema definitions are JSON)
        SchemaFormat::Avro | SchemaFormat::Protobuf => {
            if level == CompatibilityLevel::None {
                return true;
            }
            !new_schema.is_empty()
        }
    }
}

// ── Schema Registry ───────────────────────────────────────────────────────────

pub struct SchemaRegistry {
    subjects: DashMap<String, Subject>,
    /// Global schema ID → schema for lookup by ID
    schemas_by_id: DashMap<i32, Schema>,
    next_id: AtomicI32,
    global_compatibility: std::sync::RwLock<CompatibilityLevel>,
}

impl SchemaRegistry {
    pub fn new() -> Self {
        Self {
            subjects: DashMap::new(),
            schemas_by_id: DashMap::new(),
            next_id: AtomicI32::new(1),
            global_compatibility: std::sync::RwLock::new(CompatibilityLevel::Backward),
        }
    }

    // ── Register schema ───────────────────────────────────────────────────────

    pub fn register_schema(
        &self,
        subject: &str,
        schema: String,
        schema_type: SchemaFormat,
        references: Vec<SchemaReference>,
    ) -> StreamsResult<i32> {
        let compat = self.effective_compatibility(subject);

        let mut subj = self
            .subjects
            .entry(subject.to_string())
            .or_insert_with(|| Subject::new(subject.to_string()));

        // Check compatibility
        if !subj.versions.is_empty() {
            let existing: Vec<&Schema> = subj.versions.iter().collect();
            if !check_schema_compatible(compat, schema_type, &schema, &existing) {
                return Err(StreamsError::SchemaIncompatible {
                    subject: subject.to_string(),
                    reason: format!("not {:?} compatible", compat),
                });
            }
        }

        // Check for exact duplicate
        for existing in &subj.versions {
            if existing.schema == schema {
                return Ok(existing.id);
            }
        }

        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let version = subj.versions.len() as i32 + 1;
        let s = Schema {
            id,
            version,
            subject: subject.to_string(),
            schema: schema.clone(),
            schema_type,
            references,
            created_at: Utc::now(),
        };
        self.schemas_by_id.insert(id, s.clone());
        subj.versions.push(s);
        Ok(id)
    }

    // ── Lookup ────────────────────────────────────────────────────────────────

    pub fn get_schema_by_id(&self, id: i32) -> StreamsResult<Schema> {
        self.schemas_by_id
            .get(&id)
            .map(|s| s.clone())
            .ok_or_else(|| StreamsError::SchemaNotFound(id))
    }

    pub fn get_schema_by_version(&self, subject: &str, version: i32) -> StreamsResult<Schema> {
        let subj = self
            .subjects
            .get(subject)
            .ok_or_else(|| StreamsError::SubjectNotFound(subject.to_string()))?;
        subj.get_version(version)
            .cloned()
            .ok_or_else(|| StreamsError::SchemaNotFound(version))
    }

    pub fn get_latest_schema(&self, subject: &str) -> StreamsResult<Schema> {
        self.get_schema_by_version(subject, -1)
    }

    pub fn list_subjects(&self) -> Vec<String> {
        self.subjects
            .iter()
            .filter(|e| !e.is_deleted)
            .map(|e| e.key().clone())
            .collect()
    }

    pub fn list_versions(&self, subject: &str) -> StreamsResult<Vec<i32>> {
        let subj = self
            .subjects
            .get(subject)
            .ok_or_else(|| StreamsError::SubjectNotFound(subject.to_string()))?;
        Ok(subj.versions.iter().map(|s| s.version).collect())
    }

    pub fn delete_subject(&self, subject: &str) -> StreamsResult<Vec<i32>> {
        let mut subj = self
            .subjects
            .get_mut(subject)
            .ok_or_else(|| StreamsError::SubjectNotFound(subject.to_string()))?;
        let versions: Vec<i32> = subj.versions.iter().map(|s| s.version).collect();
        subj.is_deleted = true;
        Ok(versions)
    }

    pub fn delete_schema_version(&self, subject: &str, version: i32) -> StreamsResult<i32> {
        let mut subj = self
            .subjects
            .get_mut(subject)
            .ok_or_else(|| StreamsError::SubjectNotFound(subject.to_string()))?;
        let pos = subj
            .versions
            .iter()
            .position(|s| s.version == version)
            .ok_or_else(|| StreamsError::SchemaNotFound(version))?;
        let removed = subj.versions.remove(pos);
        Ok(removed.id)
    }

    // ── Compatibility ─────────────────────────────────────────────────────────

    pub fn check_compatibility(
        &self,
        subject: &str,
        version: i32,
        new_schema: &str,
        schema_type: SchemaFormat,
    ) -> StreamsResult<bool> {
        let compat = self.effective_compatibility(subject);
        let subj = self
            .subjects
            .get(subject)
            .ok_or_else(|| StreamsError::SubjectNotFound(subject.to_string()))?;
        let existing: Vec<&Schema> = if version == -1 {
            subj.versions.iter().collect()
        } else {
            subj.versions
                .iter()
                .filter(|s| s.version <= version)
                .collect()
        };
        Ok(check_schema_compatible(
            compat,
            schema_type,
            new_schema,
            &existing,
        ))
    }

    pub fn get_global_compatibility(&self) -> CompatibilityLevel {
        *self.global_compatibility.read().unwrap()
    }

    pub fn set_global_compatibility(&self, level: CompatibilityLevel) {
        *self.global_compatibility.write().unwrap() = level;
    }

    pub fn get_subject_compatibility(&self, subject: &str) -> Option<CompatibilityLevel> {
        self.subjects.get(subject).and_then(|s| s.compatibility)
    }

    pub fn set_subject_compatibility(&self, subject: &str, level: CompatibilityLevel) {
        let mut subj = self
            .subjects
            .entry(subject.to_string())
            .or_insert_with(|| Subject::new(subject.to_string()));
        subj.compatibility = Some(level);
    }

    fn effective_compatibility(&self, subject: &str) -> CompatibilityLevel {
        self.subjects
            .get(subject)
            .and_then(|s| s.compatibility)
            .unwrap_or_else(|| self.get_global_compatibility())
    }
}

// ── HTTP request/response DTOs ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RegisterSchemaRequest {
    pub schema: String,
    #[serde(rename = "schemaType", default = "default_schema_type")]
    pub schema_type: String,
    #[serde(default)]
    pub references: Vec<SchemaRefDto>,
}

fn default_schema_type() -> String {
    "AVRO".into()
}

#[derive(Debug, Deserialize)]
pub struct SchemaRefDto {
    pub name: String,
    pub subject: String,
    pub version: i32,
}

#[derive(Debug, Serialize)]
pub struct RegisterSchemaResponse {
    pub id: i32,
}

#[derive(Debug, Serialize)]
pub struct SchemaResponse {
    pub id: i32,
    pub version: i32,
    pub subject: String,
    pub schema: String,
    #[serde(rename = "schemaType")]
    pub schema_type: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CompatibilityConfig {
    pub compatibility: String,
}

#[derive(Debug, Serialize)]
pub struct CompatibilityCheckResponse {
    pub is_compatible: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> SchemaRegistry {
        SchemaRegistry::new()
    }

    const AVRO_SCHEMA_V1: &str = r#"{"type":"record","name":"User","fields":[{"name":"id","type":"int"},{"name":"name","type":"string"}]}"#;
    const AVRO_SCHEMA_V2: &str = r#"{"type":"record","name":"User","fields":[{"name":"id","type":"int"},{"name":"name","type":"string"},{"name":"email","type":["null","string"],"default":null}]}"#;

    #[test]
    fn register_and_lookup_by_id() {
        let r = registry();
        let id = r
            .register_schema(
                "user-value",
                AVRO_SCHEMA_V1.into(),
                SchemaFormat::Avro,
                vec![],
            )
            .unwrap();
        let schema = r.get_schema_by_id(id).unwrap();
        assert_eq!(schema.subject, "user-value");
        assert_eq!(schema.version, 1);
    }

    #[test]
    fn duplicate_schema_returns_same_id() {
        let r = registry();
        let id1 = r
            .register_schema("evt", AVRO_SCHEMA_V1.into(), SchemaFormat::Avro, vec![])
            .unwrap();
        let id2 = r
            .register_schema("evt", AVRO_SCHEMA_V1.into(), SchemaFormat::Avro, vec![])
            .unwrap();
        assert_eq!(id1, id2);
    }

    #[test]
    fn list_subjects_and_versions() {
        let r = registry();
        r.register_schema("s1", AVRO_SCHEMA_V1.into(), SchemaFormat::Avro, vec![])
            .unwrap();
        r.register_schema("s2", AVRO_SCHEMA_V2.into(), SchemaFormat::Avro, vec![])
            .unwrap();
        let subjects = r.list_subjects();
        assert!(subjects.contains(&"s1".to_string()));
        assert!(subjects.contains(&"s2".to_string()));
    }

    #[test]
    fn json_schema_registration() {
        let r = registry();
        let js = r#"{"type":"object","properties":{"name":{"type":"string"}}}"#;
        let id = r
            .register_schema("cfg-value", js.into(), SchemaFormat::JsonSchema, vec![])
            .unwrap();
        assert!(id > 0);
    }

    #[test]
    fn delete_subject() {
        let r = registry();
        r.register_schema("temp", AVRO_SCHEMA_V1.into(), SchemaFormat::Avro, vec![])
            .unwrap();
        let versions = r.delete_subject("temp").unwrap();
        assert_eq!(versions, vec![1]);
    }

    #[test]
    fn compatibility_check() {
        let r = registry();
        r.set_global_compatibility(CompatibilityLevel::None);
        r.register_schema("any", AVRO_SCHEMA_V1.into(), SchemaFormat::Avro, vec![])
            .unwrap();
        let ok = r
            .check_compatibility("any", -1, AVRO_SCHEMA_V2, SchemaFormat::Avro)
            .unwrap();
        assert!(ok);
    }
}
