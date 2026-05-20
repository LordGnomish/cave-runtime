// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Twenty CRM CalendarEvent — `packages/twenty-server/src/modules/calendar/standard-objects/calendar-event.workspace-entity.ts`
//!
//! Calendar sync with Google/Microsoft is intentionally deferred for the
//! MVP (see `[[scope_cuts]]` in parity.manifest.toml). The model is
//! complete enough to ingest events from any source via the REST API.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CalendarEventVisibility {
    Visible,
    /// Private events still appear in the calendar grid but with the
    /// title masked to "Busy" — Twenty mirrors Google's `visibility=private`.
    HideDetails,
    Private,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CalendarEvent {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub title: String,
    pub description: String,
    pub location: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub is_full_day: bool,
    pub visibility: CalendarEventVisibility,
    /// Conference link (Meet/Zoom/Teams). Twenty stores the raw URL.
    pub conference_link: Option<String>,
    /// External provider id (Google `iCalUID`, Outlook `iCalUId`).
    pub external_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl CalendarEvent {
    pub fn new(
        workspace_id: Uuid,
        title: impl Into<String>,
        starts_at: DateTime<Utc>,
        ends_at: DateTime<Utc>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            title: title.into(),
            description: String::new(),
            location: String::new(),
            starts_at,
            ends_at,
            is_full_day: false,
            visibility: CalendarEventVisibility::Visible,
            conference_link: None,
            external_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn duration_seconds(&self) -> i64 {
        (self.ends_at - self.starts_at).num_seconds()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CalendarEventAttendee {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub calendar_event_id: Uuid,
    pub person_id: Option<Uuid>,
    pub email: String,
    pub display_name: String,
    /// `INVITED` / `ACCEPTED` / `TENTATIVE` / `DECLINED` — matches Twenty
    /// upstream enum casing.
    pub response_status: String,
    pub is_organizer: bool,
    pub created_at: DateTime<Utc>,
}

impl CalendarEventAttendee {
    pub fn new(workspace_id: Uuid, calendar_event_id: Uuid, email: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            calendar_event_id,
            person_id: None,
            email: email.into(),
            display_name: String::new(),
            response_status: "INVITED".to_string(),
            is_organizer: false,
            created_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn duration_seconds_matches_span() {
        let now = Utc::now();
        let e = CalendarEvent::new(Uuid::nil(), "Sync", now, now + Duration::minutes(30));
        assert_eq!(e.duration_seconds(), 1800);
    }

    #[test]
    fn attendee_defaults_invited() {
        let a = CalendarEventAttendee::new(Uuid::nil(), Uuid::nil(), "a@b.c");
        assert_eq!(a.response_status, "INVITED");
        assert!(!a.is_organizer);
    }
}
