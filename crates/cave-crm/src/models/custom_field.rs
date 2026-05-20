// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Twenty CRM FieldMetadata — `packages/twenty-server/src/engine/metadata-modules/field-metadata/`
//!
//! Twenty's killer feature is custom-field-per-workspace: any user can
//! add a custom field to any standard object (Person, Company, …) and
//! the GraphQL/REST API exposes it as a first-class column. We mirror
//! the metadata table; the actual storage is encoded as JSON inside
//! `CrmStore::custom_field_values`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// All Twenty `FieldMetadataType` enum values (v2.6.0) — order matches
/// `packages/twenty-shared/src/types/FieldMetadataType.ts`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FieldKind {
    Uuid,
    Text,
    Phones,
    Emails,
    Datetime,
    Date,
    Boolean,
    Number,
    Numeric,
    Probability,
    Currency,
    FullName,
    Links,
    Address,
    Rating,
    Select,
    MultiSelect,
    Position,
    Relation,
    RichText,
    Actor,
    Array,
    TsVector,
    RawJson,
}

impl FieldKind {
    /// Whether the value occupies a single JSON scalar (true) vs a JSON
    /// object/array (false). Lets the storage layer skip prelude work.
    pub fn is_scalar(self) -> bool {
        matches!(
            self,
            Self::Uuid
                | Self::Text
                | Self::Datetime
                | Self::Date
                | Self::Boolean
                | Self::Number
                | Self::Numeric
                | Self::Probability
                | Self::Position
                | Self::Rating
                | Self::Select
                | Self::TsVector
        )
    }
}

/// Workspace-scoped metadata for a single custom field on a standard
/// object. The `object_metadata_id` resolves to either a built-in
/// (`Person` / `Company` / `Opportunity` / …) or a user-defined object
/// in `ObjectMetadata`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FieldMetadata {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub object_metadata_id: Uuid,
    /// Snake-case column name. Twenty validates `^[a-z][a-z0-9_]*$`.
    pub name: String,
    /// Free-form label shown in UI.
    pub label: String,
    pub description: Option<String>,
    pub kind: FieldKind,
    pub is_nullable: bool,
    pub is_unique: bool,
    pub is_indexed: bool,
    pub is_system: bool,
    /// JSON-encoded default value (string per cave-crm convention to
    /// avoid pulling `serde_json::Value` into core data shapes).
    pub default_value_json: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl FieldMetadata {
    pub fn new(
        workspace_id: Uuid,
        object_metadata_id: Uuid,
        name: impl Into<String>,
        kind: FieldKind,
    ) -> Self {
        let now = Utc::now();
        let name = name.into();
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            object_metadata_id,
            label: name.clone(),
            name,
            description: None,
            kind,
            is_nullable: true,
            is_unique: false,
            is_indexed: false,
            is_system: false,
            default_value_json: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Validate the snake-case identifier rule.
    pub fn is_valid_name(name: &str) -> bool {
        let mut chars = name.chars();
        let Some(first) = chars.next() else { return false };
        if !first.is_ascii_lowercase() {
            return false;
        }
        chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_metadata_new_defaults_nullable() {
        let f = FieldMetadata::new(Uuid::nil(), Uuid::nil(), "annual_revenue", FieldKind::Currency);
        assert!(f.is_nullable);
        assert!(!f.is_unique);
        assert_eq!(f.label, "annual_revenue");
    }

    #[test]
    fn field_kind_scalar_classification() {
        assert!(FieldKind::Text.is_scalar());
        assert!(FieldKind::Number.is_scalar());
        assert!(!FieldKind::Phones.is_scalar());
        assert!(!FieldKind::Links.is_scalar());
        assert!(!FieldKind::FullName.is_scalar());
    }

    #[test]
    fn name_validation_rules() {
        assert!(FieldMetadata::is_valid_name("annual_revenue"));
        assert!(FieldMetadata::is_valid_name("x"));
        assert!(!FieldMetadata::is_valid_name("AnnualRevenue"));
        assert!(!FieldMetadata::is_valid_name("9_lives"));
        assert!(!FieldMetadata::is_valid_name(""));
        assert!(!FieldMetadata::is_valid_name("a-b"));
    }
}
