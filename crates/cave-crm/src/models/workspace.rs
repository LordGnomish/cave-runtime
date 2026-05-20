// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Multi-tenant workspace + workspace member.
//!
//! Twenty upstream:
//! * `packages/twenty-server/src/engine/core-modules/workspace/workspace.entity.ts`
//! * `packages/twenty-server/src/engine/core-modules/workspace-member/workspace-member.workspace-entity.ts`
//!
//! In Twenty each workspace is a logical tenant — every domain object
//! (`person`, `company`, `opportunity`, …) carries an implicit `workspaceId`
//! that scopes reads/writes. We mirror that semantics: every model carries
//! `workspace_id: Uuid` and the in-memory store filters by it.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A logical multi-tenant boundary.
///
/// Per ADR-MULTI-TENANT-001 the strong-isolation cut is the Kamaji vCluster
/// boundary; `Workspace` is the *application-level* scoping primitive
/// inside a single cave-crm runtime — useful when a single vCluster hosts
/// several small tenants in shared schema.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Workspace {
    pub id: Uuid,
    pub name: String,
    /// Twenty's `displayName` — typically the company brand shown in UI.
    pub display_name: String,
    /// Optional logo URL (any HTTPS scheme).
    pub logo_url: Option<String>,
    /// Optional inviteHash — invitation links use this to bind to the
    /// workspace without leaking the internal UUID.
    pub invite_hash: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Workspace {
    pub fn new(name: impl Into<String>) -> Self {
        let now = Utc::now();
        let name = name.into();
        Self {
            id: Uuid::new_v4(),
            display_name: name.clone(),
            name,
            logo_url: None,
            invite_hash: None,
            created_at: now,
            updated_at: now,
        }
    }
}

/// A user's membership inside a workspace — the join row in Twenty.
///
/// Twenty has `WorkspaceMember` as a workspace-entity (i.e. it lives in
/// the tenant DB schema, not the platform DB). We collapse that to a
/// single record with both the tenant `workspace_id` and a foreign-key
/// `user_id` into the platform `User` table.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceMember {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub user_id: Uuid,
    pub role: WorkspaceMemberRole,
    pub name_first: String,
    pub name_last: String,
    pub locale: String,
    pub time_zone: String,
    /// Date format preference — Twenty exposes `SYSTEM` / `MONTH_FIRST` /
    /// `DAY_FIRST` / `YEAR_FIRST`. We store the literal Twenty enum value
    /// so the JSON wire is round-trippable.
    pub date_format: String,
    pub time_format: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkspaceMemberRole {
    Admin,
    Member,
    Guest,
}

impl WorkspaceMember {
    pub fn new(workspace_id: Uuid, user_id: Uuid, role: WorkspaceMemberRole) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            user_id,
            role,
            name_first: String::new(),
            name_last: String::new(),
            locale: "en".to_string(),
            time_zone: "UTC".to_string(),
            date_format: "SYSTEM".to_string(),
            time_format: "SYSTEM".to_string(),
            created_at: now,
            updated_at: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_new_copies_display_name() {
        let ws = Workspace::new("Acme");
        assert_eq!(ws.name, "Acme");
        assert_eq!(ws.display_name, "Acme");
        assert!(ws.logo_url.is_none());
    }

    #[test]
    fn workspace_member_defaults_locale_and_tz() {
        let m = WorkspaceMember::new(Uuid::new_v4(), Uuid::new_v4(), WorkspaceMemberRole::Admin);
        assert_eq!(m.locale, "en");
        assert_eq!(m.time_zone, "UTC");
        assert_eq!(m.role, WorkspaceMemberRole::Admin);
    }

    #[test]
    fn role_serializes_screaming_snake_case() {
        let s = serde_json::to_string(&WorkspaceMemberRole::Admin).unwrap();
        assert_eq!(s, "\"ADMIN\"");
    }
}
