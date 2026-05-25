// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Twenty CRM View — `packages/twenty-server/src/modules/view/standard-objects/view.workspace-entity.ts`
//!
//! A `View` is a saved table/kanban configuration. Each View binds to
//! exactly one ObjectMetadata (e.g. "Opportunities by Stage") and carries
//! `viewKind` (table | kanban | calendar) plus serialized filter/sort
//! configuration. The MVP stores the configuration as opaque JSON strings;
//! per-field filter/sort sub-tables are deferred (see scope cuts).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ViewKind {
    Table,
    Kanban,
    Calendar,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct View {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub object_metadata_id: Uuid,
    pub name: String,
    pub icon: Option<String>,
    pub kind: ViewKind,
    /// Soft-default view per object (only one row per object_metadata_id
    /// should carry `is_default = true` — caller enforces).
    pub is_default: bool,
    /// JSON-encoded list of column / lane configuration (cave-crm holds
    /// it opaque; the portal renderer parses it).
    pub config_json: String,
    pub position: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl View {
    pub fn new(
        workspace_id: Uuid,
        object_metadata_id: Uuid,
        name: impl Into<String>,
        kind: ViewKind,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            object_metadata_id,
            name: name.into(),
            icon: None,
            kind,
            is_default: false,
            config_json: "{}".to_string(),
            position: 0,
            created_at: now,
            updated_at: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_new_defaults_empty_config() {
        let v = View::new(Uuid::nil(), Uuid::nil(), "All Opps", ViewKind::Kanban);
        assert_eq!(v.config_json, "{}");
        assert!(!v.is_default);
    }

    #[test]
    fn view_kind_serializes_screaming() {
        let s = serde_json::to_string(&ViewKind::Kanban).unwrap();
        assert_eq!(s, "\"KANBAN\"");
    }
}
