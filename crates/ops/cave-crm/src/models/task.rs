// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Twenty CRM Task — `packages/twenty-server/src/modules/task/standard-objects/task.workspace-entity.ts`

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TaskStatus {
    Todo,
    InProgress,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Task {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub title: String,
    pub body: String,
    pub status: TaskStatus,
    pub due_at: Option<DateTime<Utc>>,
    pub assignee_user_id: Option<Uuid>,
    pub author_user_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Task {
    pub fn new(workspace_id: Uuid, title: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            title: title.into(),
            body: String::new(),
            status: TaskStatus::Todo,
            due_at: None,
            assignee_user_id: None,
            author_user_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn complete(&mut self) {
        self.status = TaskStatus::Done;
        self.updated_at = Utc::now();
    }

    pub fn start(&mut self) {
        self.status = TaskStatus::InProgress;
        self.updated_at = Utc::now();
    }

    pub fn is_overdue(&self, now: DateTime<Utc>) -> bool {
        match self.due_at {
            Some(d) => d < now && self.status != TaskStatus::Done,
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn new_task_status_is_todo() {
        let t = Task::new(Uuid::nil(), "x");
        assert_eq!(t.status, TaskStatus::Todo);
        assert!(t.due_at.is_none());
    }

    #[test]
    fn complete_sets_done() {
        let mut t = Task::new(Uuid::nil(), "x");
        t.complete();
        assert_eq!(t.status, TaskStatus::Done);
    }

    #[test]
    fn is_overdue_only_when_past_and_not_done() {
        let now = Utc::now();
        let mut t = Task::new(Uuid::nil(), "x");
        t.due_at = Some(now - Duration::hours(1));
        assert!(t.is_overdue(now));
        t.complete();
        assert!(!t.is_overdue(now));
    }

    #[test]
    fn is_overdue_false_for_no_due_date() {
        let t = Task::new(Uuid::nil(), "x");
        assert!(!t.is_overdue(Utc::now()));
    }
}
