// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Catalog entity models — port from @backstage/catalog-model.
//!
//! Upstream references:
//! - backstage/plugins/catalog-backend/src/database/tables.ts
//! - @backstage/catalog-model: Entity, EntityMeta, Location

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Entity ─────────────────────────────────────────────────────────────────────

/// A single catalog entity, analogous to Backstage's `Entity` type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    /// e.g. "backstage.io/v1alpha1"
    pub api_version: String,
    /// e.g. "Component", "API", "Resource"
    pub kind: String,
    pub metadata: EntityMetadata,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spec: Option<serde_json::Value>,
    #[serde(default)]
    pub relations: Vec<EntityRelation>,
}

/// Metadata block, analogous to Backstage's `EntityMeta`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityMetadata {
    /// UUID — assigned on creation.
    pub uid: String,
    pub name: String,
    /// Defaults to "default" when not specified.
    #[serde(default = "default_namespace")]
    pub namespace: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub labels: HashMap<String, String>,
    #[serde(default)]
    pub annotations: HashMap<String, String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// ISO 8601 creation timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// ISO 8601 last-updated timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

fn default_namespace() -> String {
    "default".to_string()
}

/// A directional relationship from one entity to another.
/// Analogous to Backstage's `EntityRelation`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityRelation {
    /// e.g. "ownedBy", "partOf", "hasPart"
    #[serde(rename = "type")]
    pub type_: String,
    /// Fully-qualified entity reference, e.g. "component:default/my-component"
    pub target_ref: String,
}

// ── Location ───────────────────────────────────────────────────────────────────

/// A catalog location — where entities are discovered.
/// Analogous to `LocationSpec` + `Location` in Backstage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    /// UUID.
    pub id: String,
    /// e.g. "url", "file"
    #[serde(rename = "type")]
    pub type_: String,
    /// URL or file path.
    pub target: String,
    /// "required" | "optional"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence: Option<String>,
}

// ── EntityFilter ───────────────────────────────────────────────────────────────

/// Search predicate for entities.
/// Upstream: `EntityFilter` in `@backstage/catalog-model`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EntityFilter {
    pub kind: Option<String>,
    pub namespace: Option<String>,
    pub name: Option<String>,
    #[serde(default)]
    pub labels: HashMap<String, String>,
}

// ── RefreshStateRecord ─────────────────────────────────────────────────────────

/// Tracks the processing state of an entity.
/// Upstream: `refresh_state` table in `DefaultCatalogDatabase`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshStateRecord {
    /// Fully-qualified entity reference, e.g. "component:default/my-service"
    pub entity_ref: String,
    /// The raw, unprocessed entity JSON that was discovered.
    pub unprocessed_entity: serde_json::Value,
    /// Serialised processing error, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<String>,
    /// When to next refresh this entity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_update_at: Option<chrono::DateTime<chrono::Utc>>,
    /// When this entity was last discovered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_discovery_at: Option<chrono::DateTime<chrono::Utc>>,
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Test 16: default EntityFilter has all None fields — matches everything.
    #[test]
    fn entity_filter_default_matches_all() {
        let f = EntityFilter::default();
        assert!(f.kind.is_none());
        assert!(f.namespace.is_none());
        assert!(f.name.is_none());
        assert!(f.labels.is_empty());
    }

    /// Test 17: Entity serialises to JSON and round-trips without loss.
    #[test]
    fn entity_serializes_roundtrip() {
        let entity = Entity {
            api_version: "backstage.io/v1alpha1".to_string(),
            kind: "Component".to_string(),
            metadata: EntityMetadata {
                uid: "uid-1".to_string(),
                name: "my-service".to_string(),
                namespace: "default".to_string(),
                title: Some("My Service".to_string()),
                description: Some("A test service".to_string()),
                labels: {
                    let mut m = HashMap::new();
                    m.insert("env".to_string(), "prod".to_string());
                    m
                },
                annotations: HashMap::new(),
                tags: vec!["rust".to_string()],
                created_at: None,
                updated_at: None,
            },
            spec: Some(serde_json::json!({ "type": "service", "lifecycle": "production" })),
            relations: vec![EntityRelation {
                type_: "ownedBy".to_string(),
                target_ref: "group:default/platform".to_string(),
            }],
        };

        let json = serde_json::to_string(&entity).expect("serialise");
        let back: Entity = serde_json::from_str(&json).expect("deserialise");

        assert_eq!(back.kind, "Component");
        assert_eq!(back.metadata.uid, "uid-1");
        assert_eq!(back.metadata.name, "my-service");
        assert_eq!(back.metadata.namespace, "default");
        assert_eq!(back.metadata.tags, vec!["rust"]);
        assert_eq!(back.relations.len(), 1);
        assert_eq!(back.relations[0].type_, "ownedBy");
        assert!(back.spec.is_some());
    }

    /// Test 18: Location serialises to JSON and round-trips without loss.
    #[test]
    fn location_serializes_roundtrip() {
        let loc = Location {
            id: "loc-1".to_string(),
            type_: "url".to_string(),
            target: "https://example.com/catalog-info.yaml".to_string(),
            presence: Some("required".to_string()),
        };

        let json = serde_json::to_string(&loc).expect("serialise");
        let back: Location = serde_json::from_str(&json).expect("deserialise");

        assert_eq!(back.id, "loc-1");
        assert_eq!(back.type_, "url");
        assert_eq!(back.target, "https://example.com/catalog-info.yaml");
        assert_eq!(back.presence, Some("required".to_string()));
    }
}
