// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Platform-level User identity (federation with cave-auth).
//!
//! Twenty upstream: `packages/twenty-server/src/engine/core-modules/user/user.entity.ts`
//!
//! In Twenty `User` lives in the platform DB and is workspace-agnostic;
//! membership of a user inside a workspace is recorded by `WorkspaceMember`.
//! Authentication itself is delegated — cave-crm trusts the `user_id`
//! claim minted by cave-auth and does not store passwords.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub first_name: String,
    pub last_name: String,
    pub disabled: bool,
    /// Last login timestamp — Twenty mirrors this to support session
    /// hygiene + "last seen" UI affordances.
    pub last_login_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl User {
    pub fn new(email: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            email: email.into(),
            first_name: String::new(),
            last_name: String::new(),
            disabled: false,
            last_login_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn full_name(&self) -> String {
        format!("{} {}", self.first_name, self.last_name).trim().to_string()
    }

    pub fn mark_logged_in(&mut self) {
        let now = Utc::now();
        self.last_login_at = Some(now);
        self.updated_at = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_user_has_email_and_not_disabled() {
        let u = User::new("alice@example.com");
        assert_eq!(u.email, "alice@example.com");
        assert!(!u.disabled);
        assert!(u.last_login_at.is_none());
    }

    #[test]
    fn full_name_concatenates() {
        let mut u = User::new("alice@example.com");
        u.first_name = "Alice".into();
        u.last_name = "Liddell".into();
        assert_eq!(u.full_name(), "Alice Liddell");
    }

    #[test]
    fn mark_logged_in_sets_timestamp() {
        let mut u = User::new("alice@example.com");
        assert!(u.last_login_at.is_none());
        u.mark_logged_in();
        assert!(u.last_login_at.is_some());
    }
}
