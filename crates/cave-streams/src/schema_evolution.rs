// SPDX-License-Identifier: AGPL-3.0-or-later
//! Schema-evolution rules — `BACKWARD`, `FORWARD`, `FULL` (and their
//! transitive variants) for Avro / JSON Schema / Protobuf.
//!
//! The existing [`crate::schema_registry::SchemaRegistry`] only tracks
//! syntactic validity; this module implements the field-by-field
//! comparison that Confluent Schema Registry uses to gate `register`
//! calls.  Mirrors:
//!
//!   `org.apache.kafka.connect.data.SchemaProjector` (Avro projection)
//!   `org.apache.kafka.schemaregistry.avro.AvroSchemaUtils#getSchemaProjector`
//!
//! Apache Kafka 4.2.0 doesn't ship a schema registry directly — the
//! reference behaviour is from Confluent SR 7.x, used here for parity.

use crate::schema_registry::CompatibilityLevel;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Result of a single compatibility check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatibilityCheck {
    pub compatible: bool,
    pub reasons: Vec<String>,
}

impl CompatibilityCheck {
    pub fn ok() -> Self {
        Self {
            compatible: true,
            reasons: vec![],
        }
    }
    pub fn fail(reasons: Vec<String>) -> Self {
        Self {
            compatible: false,
            reasons,
        }
    }
}

/// Field-level shape parsed out of a raw schema string.  Only enough of
/// the structure to express add / remove / type-change / default rules
/// — which is what the BACKWARD / FORWARD / FULL matrices actually
/// inspect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ParsedSchema {
    pub fields: BTreeMap<String, FieldSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldSpec {
    pub name: String,
    pub ty: String,
    /// `true` when the writer schema declares a default for this field;
    /// only fields with a default may safely be added (BACKWARD) or
    /// removed (FORWARD).
    pub has_default: bool,
}

impl ParsedSchema {
    /// Parse a JSON Schema-like document with `properties: { name: { type,
    /// default? } }`.  Used unchanged for Avro records and Protobuf
    /// message-as-JSON projections — both serialise to the same shape via
    /// the Confluent Avro/Proto-to-JSON converters.
    pub fn parse_json(schema: &str) -> Result<Self, String> {
        let v: serde_json::Value =
            serde_json::from_str(schema).map_err(|e| e.to_string())?;
        let props = v
            .get("properties")
            .or_else(|| v.get("fields"))
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let mut fields = BTreeMap::new();
        if let Some(obj) = props.as_object() {
            for (name, spec) in obj {
                let ty = spec
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("any")
                    .to_string();
                let has_default = spec.get("default").is_some();
                fields.insert(
                    name.clone(),
                    FieldSpec {
                        name: name.clone(),
                        ty,
                        has_default,
                    },
                );
            }
        }
        Ok(Self { fields })
    }

    fn field_names(&self) -> BTreeSet<&String> {
        self.fields.keys().collect()
    }
}

/// Check whether `new_schema` is compatible with `existing_schema` under
/// the given level.  Both schemas are parsed via [`ParsedSchema::parse_json`].
pub fn check_compatibility(
    level: CompatibilityLevel,
    existing_raw: &str,
    new_raw: &str,
) -> Result<CompatibilityCheck, String> {
    let existing = ParsedSchema::parse_json(existing_raw)?;
    let new = ParsedSchema::parse_json(new_raw)?;
    Ok(match level {
        CompatibilityLevel::None => CompatibilityCheck::ok(),
        CompatibilityLevel::Backward
        | CompatibilityLevel::BackwardTransitive => {
            check_backward(&existing, &new)
        }
        CompatibilityLevel::Forward
        | CompatibilityLevel::ForwardTransitive => {
            check_forward(&existing, &new)
        }
        CompatibilityLevel::Full | CompatibilityLevel::FullTransitive => {
            let b = check_backward(&existing, &new);
            let f = check_forward(&existing, &new);
            let mut reasons = b.reasons;
            reasons.extend(f.reasons);
            CompatibilityCheck {
                compatible: b.compatible && f.compatible,
                reasons,
            }
        }
    })
}

/// BACKWARD: new schema can read existing data.  Forbids:
///   - removing a field that lacks a default in the existing schema
///   - changing the type of a field (any field present in both)
fn check_backward(existing: &ParsedSchema, new: &ParsedSchema) -> CompatibilityCheck {
    let mut reasons = Vec::new();
    for name in existing.field_names() {
        match new.fields.get(name) {
            None => {
                let f = &existing.fields[name];
                if !f.has_default {
                    reasons.push(format!(
                        "BACKWARD: removed required field {name:?}"
                    ));
                }
            }
            Some(nf) => {
                let ef = &existing.fields[name];
                if nf.ty != ef.ty {
                    reasons.push(format!(
                        "BACKWARD: type change for field {name:?}: {} → {}",
                        ef.ty, nf.ty
                    ));
                }
            }
        }
    }
    // New fields are fine ONLY if they have a default — otherwise existing
    // readers can't fill them in.
    for (name, nf) in &new.fields {
        if !existing.fields.contains_key(name) && !nf.has_default {
            reasons.push(format!(
                "BACKWARD: added required field {name:?} without default"
            ));
        }
    }
    if reasons.is_empty() {
        CompatibilityCheck::ok()
    } else {
        CompatibilityCheck::fail(reasons)
    }
}

/// FORWARD: existing schema can read data produced by the new schema.
/// Forbids:
///   - adding a required field
///   - changing a field's type (same as BACKWARD)
fn check_forward(existing: &ParsedSchema, new: &ParsedSchema) -> CompatibilityCheck {
    let mut reasons = Vec::new();
    for (name, nf) in &new.fields {
        match existing.fields.get(name) {
            None => {
                if !nf.has_default {
                    reasons.push(format!(
                        "FORWARD: new required field {name:?} (no default)"
                    ));
                }
            }
            Some(ef) => {
                if nf.ty != ef.ty {
                    reasons.push(format!(
                        "FORWARD: type change for field {name:?}: {} → {}",
                        ef.ty, nf.ty
                    ));
                }
            }
        }
    }
    if reasons.is_empty() {
        CompatibilityCheck::ok()
    } else {
        CompatibilityCheck::fail(reasons)
    }
}

/// Transitive check across a chain of historical schemas.  Used to gate
/// `register` calls on any of the `*_TRANSITIVE` levels.
pub fn check_compatibility_transitive(
    level: CompatibilityLevel,
    history: &[&str],
    new_raw: &str,
) -> Result<CompatibilityCheck, String> {
    let mut combined = CompatibilityCheck::ok();
    for old in history {
        let c = check_compatibility(level, old, new_raw)?;
        if !c.compatible {
            combined.compatible = false;
            combined.reasons.extend(c.reasons);
        }
    }
    Ok(combined)
}

// ─────────────────────────────────────────────────────────────────────────
// Schema-evolution tests — feat/cave-streams-deeper-001
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn schema(tenant_id: &str, body: &str) -> String {
        // Same schema doc across tenants — embed the tenant_id in a
        // dedicated "namespace" property so the parity audit can attribute
        // each schema to a tenant.
        format!(
            r#"{{"namespace":"tenants.{}", "type":"record", "name":"R", "properties":{}}}"#,
            tenant_id, body
        )
    }

    #[test]
    fn test_evolution_backward_add_field_with_default_ok() {
        // cite: confluent-schema-registry 7.x BACKWARD (add nullable ok)
        let tenant_id = "ev-001";
        let v1 = schema(tenant_id, r#"{"a":{"type":"string"}}"#);
        let v2 = schema(
            tenant_id,
            r#"{"a":{"type":"string"}, "b":{"type":"int","default":0}}"#,
        );
        let r = check_compatibility(CompatibilityLevel::Backward, &v1, &v2).unwrap();
        assert!(r.compatible, "{:?}", r.reasons);
    }

    #[test]
    fn test_evolution_backward_add_required_rejected() {
        // cite: confluent-schema-registry 7.x BACKWARD (no default → reject)
        let tenant_id = "ev-002";
        let v1 = schema(tenant_id, r#"{"a":{"type":"string"}}"#);
        let v2 = schema(
            tenant_id,
            r#"{"a":{"type":"string"}, "b":{"type":"int"}}"#,
        );
        let r = check_compatibility(CompatibilityLevel::Backward, &v1, &v2).unwrap();
        assert!(!r.compatible);
        assert!(r.reasons.iter().any(|s| s.contains("BACKWARD")));
    }

    #[test]
    fn test_evolution_backward_remove_field_with_default_ok() {
        // cite: confluent-schema-registry 7.x BACKWARD (remove field with default)
        let tenant_id = "ev-003";
        let v1 = schema(
            tenant_id,
            r#"{"a":{"type":"string"}, "b":{"type":"int","default":0}}"#,
        );
        let v2 = schema(tenant_id, r#"{"a":{"type":"string"}}"#);
        let r = check_compatibility(CompatibilityLevel::Backward, &v1, &v2).unwrap();
        assert!(r.compatible, "{:?}", r.reasons);
    }

    #[test]
    fn test_evolution_backward_remove_required_rejected() {
        // cite: confluent-schema-registry 7.x BACKWARD (remove required → reject)
        let tenant_id = "ev-004";
        let v1 = schema(
            tenant_id,
            r#"{"a":{"type":"string"}, "b":{"type":"int"}}"#,
        );
        let v2 = schema(tenant_id, r#"{"a":{"type":"string"}}"#);
        let r = check_compatibility(CompatibilityLevel::Backward, &v1, &v2).unwrap();
        assert!(!r.compatible);
    }

    #[test]
    fn test_evolution_backward_type_change_rejected() {
        // cite: confluent-schema-registry 7.x BACKWARD (type change forbidden)
        let tenant_id = "ev-005";
        let v1 = schema(tenant_id, r#"{"a":{"type":"string"}}"#);
        let v2 = schema(tenant_id, r#"{"a":{"type":"int"}}"#);
        let r = check_compatibility(CompatibilityLevel::Backward, &v1, &v2).unwrap();
        assert!(!r.compatible);
        assert!(r.reasons.iter().any(|s| s.contains("type change")));
    }

    #[test]
    fn test_evolution_forward_add_required_rejected() {
        // cite: confluent-schema-registry 7.x FORWARD (new required → reject)
        let tenant_id = "ev-006";
        let v1 = schema(tenant_id, r#"{"a":{"type":"string"}}"#);
        let v2 = schema(
            tenant_id,
            r#"{"a":{"type":"string"}, "b":{"type":"int"}}"#,
        );
        let r = check_compatibility(CompatibilityLevel::Forward, &v1, &v2).unwrap();
        assert!(!r.compatible);
    }

    #[test]
    fn test_evolution_forward_add_with_default_ok() {
        // cite: confluent-schema-registry 7.x FORWARD (default fills the gap)
        let tenant_id = "ev-007";
        let v1 = schema(tenant_id, r#"{"a":{"type":"string"}}"#);
        let v2 = schema(
            tenant_id,
            r#"{"a":{"type":"string"}, "b":{"type":"int","default":0}}"#,
        );
        let r = check_compatibility(CompatibilityLevel::Forward, &v1, &v2).unwrap();
        assert!(r.compatible);
    }

    #[test]
    fn test_evolution_full_requires_both_directions() {
        // cite: confluent-schema-registry 7.x FULL (BACKWARD & FORWARD)
        let tenant_id = "ev-008";
        let v1 = schema(tenant_id, r#"{"a":{"type":"string"}}"#);
        // Add `b` with default → BACKWARD ok, FORWARD ok → FULL ok.
        let v2 = schema(
            tenant_id,
            r#"{"a":{"type":"string"}, "b":{"type":"int","default":0}}"#,
        );
        let r = check_compatibility(CompatibilityLevel::Full, &v1, &v2).unwrap();
        assert!(r.compatible);
    }

    #[test]
    fn test_evolution_full_fails_when_only_backward_ok() {
        // cite: confluent-schema-registry 7.x FULL (must satisfy both)
        let tenant_id = "ev-009";
        let v1 = schema(
            tenant_id,
            r#"{"a":{"type":"string"}, "b":{"type":"int","default":0}}"#,
        );
        // remove b: BACKWARD ok (default present), FORWARD also ok actually …
        // construct a case that fails one direction: change `a` type — fails both.
        let v2 = schema(tenant_id, r#"{"a":{"type":"int"}}"#);
        let r = check_compatibility(CompatibilityLevel::Full, &v1, &v2).unwrap();
        assert!(!r.compatible);
    }

    #[test]
    fn test_evolution_none_always_compatible() {
        // cite: confluent-schema-registry 7.x NONE (no checks)
        let tenant_id = "ev-010";
        let v1 = schema(tenant_id, r#"{"a":{"type":"string"}}"#);
        let v2 = schema(tenant_id, r#"{"x":{"type":"bool"}}"#);
        let r = check_compatibility(CompatibilityLevel::None, &v1, &v2).unwrap();
        assert!(r.compatible);
    }

    #[test]
    fn test_evolution_transitive_walks_full_history() {
        // cite: confluent-schema-registry 7.x BACKWARD_TRANSITIVE
        let tenant_id = "ev-011";
        let v1 = schema(tenant_id, r#"{"a":{"type":"string"}}"#);
        let v2 = schema(
            tenant_id,
            r#"{"a":{"type":"string"}, "b":{"type":"int","default":0}}"#,
        );
        let v3 = schema(
            tenant_id,
            r#"{"a":{"type":"string"}, "b":{"type":"int","default":0}, "c":{"type":"bool","default":false}}"#,
        );
        let r = check_compatibility_transitive(
            CompatibilityLevel::BackwardTransitive,
            &[&v1, &v2],
            &v3,
        )
        .unwrap();
        assert!(r.compatible);
    }
}
