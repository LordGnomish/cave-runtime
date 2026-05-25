// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Twenty CRM ObjectMetadata — `packages/twenty-server/src/engine/metadata-modules/object-metadata/`
//!
//! Workspace-scoped metadata about an object — either a built-in standard
//! object (`person`, `company`, `opportunity`, …) or a user-defined custom
//! object. Built-in rows are seeded by `WorkspaceInitService` and marked
//! `is_system = true` so the UI hides them from the "delete object" affordance.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ObjectMetadata {
    pub id: Uuid,
    pub workspace_id: Uuid,
    /// Singular snake-case name (e.g. `person`, `meeting_room`).
    pub name_singular: String,
    /// Plural snake-case name (e.g. `people`, `meeting_rooms`).
    pub name_plural: String,
    pub label_singular: String,
    pub label_plural: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub is_system: bool,
    pub is_active: bool,
    pub is_remote: bool,
    pub is_searchable: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ObjectMetadata {
    pub fn new(
        workspace_id: Uuid,
        name_singular: impl Into<String>,
        name_plural: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        let ns = name_singular.into();
        let np = name_plural.into();
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            label_singular: ns.clone(),
            label_plural: np.clone(),
            name_singular: ns,
            name_plural: np,
            description: None,
            icon: None,
            is_system: false,
            is_active: true,
            is_remote: false,
            is_searchable: true,
            created_at: now,
            updated_at: now,
        }
    }

    /// The standard objects Twenty seeds on workspace init (sans `View`
    /// and a couple of internal-only entities) — matches the `Cave CRM`
    /// minimal MVP entity set.
    pub fn standards(workspace_id: Uuid) -> Vec<Self> {
        [
            ("person", "people"),
            ("company", "companies"),
            ("opportunity", "opportunities"),
            ("pipeline_step", "pipeline_steps"),
            ("note", "notes"),
            ("task", "tasks"),
            ("calendar_event", "calendar_events"),
            ("lead", "leads"),
        ]
        .into_iter()
        .map(|(s, p)| {
            let mut o = Self::new(workspace_id, s, p);
            o.is_system = true;
            o
        })
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standards_seeds_eight_system_objects() {
        let s = ObjectMetadata::standards(Uuid::nil());
        assert_eq!(s.len(), 8);
        assert!(s.iter().all(|o| o.is_system));
        assert!(s.iter().any(|o| o.name_singular == "person"));
        assert!(s.iter().any(|o| o.name_singular == "opportunity"));
    }
}
