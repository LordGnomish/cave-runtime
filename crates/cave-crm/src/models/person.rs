// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Twenty CRM Person — `packages/twenty-server/src/modules/person/standard-objects/person.workspace-entity.ts`

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Person {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub first_name: String,
    pub last_name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub job_title: Option<String>,
    pub linkedin_url: Option<String>,
    pub x_url: Option<String>,
    pub avatar_url: Option<String>,
    /// Optional foreign-key to `Company` — Twenty's `companyId`.
    pub company_id: Option<Uuid>,
    pub city: Option<String>,
    /// Free-form ranked label — Twenty exposes a `position` int the UI
    /// uses to sort kanban lanes by intrinsic priority.
    pub position: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Person {
    pub fn new(workspace_id: Uuid, first_name: impl Into<String>, last_name: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            first_name: first_name.into(),
            last_name: last_name.into(),
            email: None,
            phone: None,
            job_title: None,
            linkedin_url: None,
            x_url: None,
            avatar_url: None,
            company_id: None,
            city: None,
            position: 0,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn display_name(&self) -> String {
        format!("{} {}", self.first_name, self.last_name).trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn person_new_carries_workspace_id() {
        let ws = Uuid::new_v4();
        let p = Person::new(ws, "Ada", "Lovelace");
        assert_eq!(p.workspace_id, ws);
        assert_eq!(p.display_name(), "Ada Lovelace");
    }

    #[test]
    fn person_serializes_optional_email_as_null() {
        let p = Person::new(Uuid::nil(), "Ada", "Lovelace");
        let j = serde_json::to_value(&p).unwrap();
        assert!(j["email"].is_null());
        assert_eq!(j["first_name"], "Ada");
    }
}
