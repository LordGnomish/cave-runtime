// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Twenty CRM Activity umbrella: Note + Task + ActivityTarget.
//!
//! Twenty upstream (v2.6.0 split):
//! * `packages/twenty-server/src/modules/note/standard-objects/note.workspace-entity.ts`
//! * `packages/twenty-server/src/modules/note/standard-objects/note-target.workspace-entity.ts`
//! * `packages/twenty-server/src/modules/task/...` (task entity)
//!
//! Earlier Twenty versions exposed a single `Activity` entity with a
//! `type` discriminator (Note / Task / Email / Call / Meeting). v2.x
//! split it into two top-level objects (Note + Task) sharing the same
//! `ActivityTarget` polymorphic-link table. This module mirrors v2.6.0's
//! shape while keeping a unified `Activity` enum view for legacy callers.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Pre-split umbrella type — useful for `cavectl crm activity ls` style
/// affordances that don't care whether the row is a Note or a Task.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Activity {
    Note(Note),
    Task(crate::models::task::Task),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ActivityKind {
    Note,
    Task,
}

/// Twenty's Note workspace-entity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Note {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub title: String,
    /// Markdown body. Twenty stores this as a `RICH_TEXT` (BlockNote-encoded
    /// JSON) — at the cave-crm wire boundary we keep raw markdown and let
    /// the portal renderer parse it.
    pub body: String,
    pub author_user_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Note {
    pub fn new(workspace_id: Uuid, title: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            title: title.into(),
            body: String::new(),
            author_user_id: None,
            created_at: now,
            updated_at: now,
        }
    }
}

/// Polymorphic link from a Note/Task to one of: Person / Company /
/// Opportunity. Mirrors Twenty's `note-target` + `task-target` tables.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActivityTarget {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub activity_id: Uuid,
    pub activity_kind: ActivityKind,
    pub target_kind: ActivityTargetKind,
    pub target_id: Uuid,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ActivityTargetKind {
    Person,
    Company,
    Opportunity,
    Lead,
}

impl ActivityTarget {
    pub fn new(
        workspace_id: Uuid,
        activity_id: Uuid,
        activity_kind: ActivityKind,
        target_kind: ActivityTargetKind,
        target_id: Uuid,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            activity_id,
            activity_kind,
            target_kind,
            target_id,
            created_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_new_has_empty_body() {
        let n = Note::new(Uuid::nil(), "Kickoff");
        assert_eq!(n.title, "Kickoff");
        assert!(n.body.is_empty());
    }

    #[test]
    fn activity_target_carries_polymorphic_kind() {
        let t = ActivityTarget::new(
            Uuid::nil(),
            Uuid::new_v4(),
            ActivityKind::Note,
            ActivityTargetKind::Company,
            Uuid::new_v4(),
        );
        assert_eq!(t.target_kind, ActivityTargetKind::Company);
        assert_eq!(t.activity_kind, ActivityKind::Note);
    }

    #[test]
    fn activity_enum_serializes_with_kind_tag() {
        let n = Note::new(Uuid::nil(), "k");
        let a = Activity::Note(n);
        let j = serde_json::to_value(&a).unwrap();
        assert_eq!(j["kind"], "NOTE");
    }
}
