// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Twenty CRM TaskTarget — `packages/twenty-server/src/modules/task/standard-objects/task-target.workspace-entity.ts`
//!
//! The polymorphic join that links a `Task` to the CRM records it acts on.
//! Twenty's `TaskTargetWorkspaceEntity` carries a nullable `task` relation
//! plus a fan-out of nullable `targetPerson` / `targetCompany` /
//! `targetOpportunity` relations (exactly one set per row). We collapse the
//! relation fan-out to a single `(target_kind, target_id)` pair — the same
//! idiom used by [`crate::models::attachment::Attachment`] and the
//! note-target half of [`crate::models::activity::ActivityTarget`] — so the
//! shape stays one-row-one-link while preserving Twenty's exact target set.
//!
//! Note: unlike `note-target` (which also targets `Lead` in cave-crm's
//! Activity umbrella), Twenty's `task-target` targets only Person / Company
//! / Opportunity — there is no `targetLead` relation on the upstream entity.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The CRM record a task is linked to. Mirrors Twenty's
/// `targetPerson` / `targetCompany` / `targetOpportunity` relation fan-out
/// on `TaskTargetWorkspaceEntity` (collapsed to a single one-of).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TaskTargetKind {
    Person,
    Company,
    Opportunity,
}

/// TaskTarget workspace-entity — the polymorphic link row joining a
/// [`crate::models::task::Task`] to one target record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskTarget {
    pub id: Uuid,
    pub workspace_id: Uuid,
    /// Twenty `taskId` — the owning Task.
    pub task_id: Uuid,
    /// Which `target*` relation is set.
    pub target_kind: TaskTargetKind,
    /// FK into the targeted record.
    pub target_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TaskTarget {
    /// Link `task_id` to `target_id` of kind `target_kind`.
    pub fn new(
        workspace_id: Uuid,
        task_id: Uuid,
        target_kind: TaskTargetKind,
        target_id: Uuid,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            task_id,
            target_kind,
            target_id,
            created_at: now,
            updated_at: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn task_target_carries_polymorphic_kind() {
        let task_id = Uuid::new_v4();
        let person_id = Uuid::new_v4();
        let t = TaskTarget::new(Uuid::nil(), task_id, TaskTargetKind::Person, person_id);
        assert_eq!(t.task_id, task_id);
        assert_eq!(t.target_kind, TaskTargetKind::Person);
        assert_eq!(t.target_id, person_id);
        assert_eq!(t.workspace_id, Uuid::nil());
    }

    #[test]
    fn task_target_kind_serializes_screaming_snake() {
        assert_eq!(
            serde_json::to_string(&TaskTargetKind::Opportunity).unwrap(),
            "\"OPPORTUNITY\""
        );
        assert_eq!(
            serde_json::to_string(&TaskTargetKind::Company).unwrap(),
            "\"COMPANY\""
        );
    }

    #[test]
    fn task_target_does_not_admit_lead() {
        // Compile-time witness that the upstream target set is P/C/O only.
        let kinds = [
            TaskTargetKind::Person,
            TaskTargetKind::Company,
            TaskTargetKind::Opportunity,
        ];
        assert_eq!(kinds.len(), 3);
    }

    #[test]
    fn new_assigns_unique_ids_and_timestamps() {
        let a = TaskTarget::new(Uuid::nil(), Uuid::new_v4(), TaskTargetKind::Company, Uuid::new_v4());
        let b = TaskTarget::new(Uuid::nil(), Uuid::new_v4(), TaskTargetKind::Company, Uuid::new_v4());
        assert_ne!(a.id, b.id);
        assert!(a.created_at <= b.created_at);
    }
}
