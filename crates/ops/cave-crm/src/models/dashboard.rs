// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Twenty CRM Dashboard — `packages/twenty-server/src/modules/dashboard/standard-objects/dashboard.workspace-entity.ts`
//!
//! A user-built analytics page. Twenty's `DashboardWorkspaceEntity` carries
//! `title` (nullable TEXT, the lone search field per
//! `SEARCH_FIELDS_FOR_DASHBOARD`), `pageLayoutId` (nullable UUID linking the
//! widget layout), a float `position` (ordering), `createdBy`/`updatedBy`
//! `ActorMetadata` composites, relations to `timelineActivities` and
//! `attachments`, and a Postgres-generated `searchVector`.
//!
//! We model the scalar columns + the `ActorMetadata` composite (also shared
//! by every other Twenty standard-object's audit columns). The `searchVector`
//! tsvector itself is a Postgres GENERATED column; we hold its single source
//! field — the lowercased `title` text fed to `to_tsvector` — in
//! [`Dashboard::search_vector`], rebuilt whenever `title` changes.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::connected_account::ConnectedAccountProvider;

/// `FieldActorSource` — the origin that produced an audit action. Mirrors
/// Twenty's `FieldActorSource` enum (10 variants) one-for-one; wire values
/// are the SCREAMING_SNAKE variant names.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ActorSource {
    Email,
    Calendar,
    Workflow,
    Agent,
    Api,
    Import,
    Manual,
    System,
    Webhook,
    Application,
}

/// `ActorMetadata.context` — currently a single optional `provider` hint,
/// matching Twenty's `{ provider?: ConnectedAccountProvider }`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActorContext {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub provider: Option<ConnectedAccountProvider>,
}

/// `ActorMetadata` composite — stamped onto `createdBy`/`updatedBy` of every
/// Twenty workspace-entity. `name` is required; `workspaceMemberId` is null
/// for non-member actors (system / API / import).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActorMetadata {
    pub source: ActorSource,
    pub workspace_member_id: Option<Uuid>,
    pub name: String,
    #[serde(default)]
    pub context: ActorContext,
}

impl ActorMetadata {
    /// A `SYSTEM`-sourced actor (migrations, seeds) — no workspace member.
    pub fn system(name: impl Into<String>) -> Self {
        Self {
            source: ActorSource::System,
            workspace_member_id: None,
            name: name.into(),
            context: ActorContext::default(),
        }
    }

    /// A `MANUAL`-sourced actor — a workspace member acting through the UI.
    pub fn manual(workspace_member_id: Uuid, name: impl Into<String>) -> Self {
        Self {
            source: ActorSource::Manual,
            workspace_member_id: Some(workspace_member_id),
            name: name.into(),
            context: ActorContext::default(),
        }
    }
}

/// Dashboard workspace-entity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Dashboard {
    pub id: Uuid,
    pub workspace_id: Uuid,
    /// Twenty `title` (nullable TEXT) — the lone search field.
    pub title: Option<String>,
    /// Twenty `pageLayoutId` — the widget layout this dashboard renders.
    pub page_layout_id: Option<Uuid>,
    /// Twenty `position` (float ordering key).
    pub position: f64,
    pub created_by: ActorMetadata,
    pub updated_by: ActorMetadata,
    /// Source text fed to Postgres `to_tsvector` for `searchVector`
    /// (lowercased `title`). The tsvector itself is DB-generated.
    pub search_vector: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Dashboard {
    /// Create a dashboard. `position` defaults to `0.0`; `updatedBy` mirrors
    /// `createdBy` at creation time (Twenty stamps both on insert).
    pub fn new(workspace_id: Uuid, title: Option<String>, created_by: ActorMetadata) -> Self {
        let now = Utc::now();
        let search_vector = Self::search_source(&title);
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            title,
            page_layout_id: None,
            position: 0.0,
            updated_by: created_by.clone(),
            created_by,
            search_vector,
            created_at: now,
            updated_at: now,
        }
    }

    /// Lowercased `title` text — the input column for `to_tsvector`.
    fn search_source(title: &Option<String>) -> String {
        title.as_deref().unwrap_or_default().to_lowercase()
    }

    /// Set the ordering position.
    pub fn reorder(&mut self, position: f64) {
        self.position = position;
        self.updated_at = Utc::now();
    }

    /// Rename, rebuilding `search_vector` and stamping `updatedBy`/`updatedAt`.
    pub fn rename(&mut self, title: Option<String>, updated_by: ActorMetadata) {
        self.search_vector = Self::search_source(&title);
        self.title = title;
        self.updated_by = updated_by;
        self.updated_at = Utc::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actor_source_serializes_screaming() {
        assert_eq!(serde_json::to_string(&ActorSource::Agent).unwrap(), "\"AGENT\"");
        assert_eq!(
            serde_json::to_string(&ActorSource::Application).unwrap(),
            "\"APPLICATION\""
        );
        assert_eq!(serde_json::to_string(&ActorSource::System).unwrap(), "\"SYSTEM\"");
    }

    #[test]
    fn actor_metadata_system_has_null_member() {
        let a = ActorMetadata::system("Migration");
        assert_eq!(a.source, ActorSource::System);
        assert_eq!(a.workspace_member_id, None);
        assert_eq!(a.name, "Migration");
        assert!(a.context.provider.is_none());
    }

    #[test]
    fn actor_context_omits_provider_when_absent() {
        let a = ActorMetadata::system("Sys");
        let json = serde_json::to_string(&a).unwrap();
        assert!(!json.contains("provider"), "provider must be omitted when None: {json}");
    }

    #[test]
    fn actor_context_round_trips_provider() {
        let mut a = ActorMetadata::manual(Uuid::nil(), "Ada");
        a.context.provider = Some(ConnectedAccountProvider::Google);
        let json = serde_json::to_string(&a).unwrap();
        let back: ActorMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(back.context.provider, Some(ConnectedAccountProvider::Google));
        assert_eq!(back.source, ActorSource::Manual);
    }

    #[test]
    fn dashboard_new_defaults() {
        let actor = ActorMetadata::manual(Uuid::new_v4(), "Ada");
        let d = Dashboard::new(Uuid::nil(), Some("Q3 Revenue".into()), actor.clone());
        assert_eq!(d.title.as_deref(), Some("Q3 Revenue"));
        assert_eq!(d.position, 0.0);
        assert_eq!(d.page_layout_id, None);
        assert_eq!(d.created_by, actor);
        assert_eq!(d.updated_by, actor);
        // searchVector source = lowercased title.
        assert_eq!(d.search_vector, "q3 revenue");
    }

    #[test]
    fn dashboard_null_title_has_empty_search_vector() {
        let d = Dashboard::new(Uuid::nil(), None, ActorMetadata::system("Sys"));
        assert_eq!(d.title, None);
        assert_eq!(d.search_vector, "");
    }

    #[test]
    fn dashboard_reorder_sets_position() {
        let mut d = Dashboard::new(Uuid::nil(), Some("A".into()), ActorMetadata::system("Sys"));
        d.reorder(3.5);
        assert_eq!(d.position, 3.5);
    }

    #[test]
    fn dashboard_rename_updates_title_search_vector_and_actor() {
        let mut d = Dashboard::new(Uuid::nil(), Some("Old".into()), ActorMetadata::system("Sys"));
        let editor = ActorMetadata::manual(Uuid::new_v4(), "Grace");
        d.rename(Some("New Title".into()), editor.clone());
        assert_eq!(d.title.as_deref(), Some("New Title"));
        assert_eq!(d.search_vector, "new title");
        assert_eq!(d.updated_by, editor);
        // createdBy is immutable.
        assert_eq!(d.created_by.name, "Sys");
    }
}
