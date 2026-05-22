// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Lead — pre-qualification stage before an Opportunity is opened.
//!
//! Twenty doesn't expose a first-class `Lead` entity (the upstream
//! flattens lead/opportunity onto the same kanban). The pattern here
//! absorbs the Lead semantics from `crates/cave-erp/src/modules/crm.rs`
//! (deprecated by ADR-145, removed in this commit) so callers that
//! relied on `/api/erp/crm/leads/{id}/convert` keep a port forward
//! under cave-crm's surface.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LeadStatus {
    New,
    Qualified,
    Converted,
    Disqualified,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Lead {
    pub id: Uuid,
    pub workspace_id: Uuid,
    /// Free-form name (e.g. "Acme — Q4 expansion").
    pub name: String,
    pub contact_name: String,
    pub email: String,
    pub phone: Option<String>,
    pub company: String,
    pub source: String,
    pub status: LeadStatus,
    pub assigned_user_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Lead {
    pub fn new(
        workspace_id: Uuid,
        name: impl Into<String>,
        contact_name: impl Into<String>,
        email: impl Into<String>,
        company: impl Into<String>,
        source: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            name: name.into(),
            contact_name: contact_name.into(),
            email: email.into(),
            phone: None,
            company: company.into(),
            source: source.into(),
            status: LeadStatus::New,
            assigned_user_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn qualify(&mut self) {
        self.status = LeadStatus::Qualified;
        self.updated_at = Utc::now();
    }

    pub fn disqualify(&mut self) {
        self.status = LeadStatus::Disqualified;
        self.updated_at = Utc::now();
    }

    /// Mark as converted (an Opportunity has been opened for this lead).
    pub fn mark_converted(&mut self) {
        self.status = LeadStatus::Converted;
        self.updated_at = Utc::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_lead_status_is_new() {
        let l = Lead::new(Uuid::nil(), "n", "c", "e", "co", "s");
        assert_eq!(l.status, LeadStatus::New);
    }

    #[test]
    fn qualify_and_disqualify_transition() {
        let mut l = Lead::new(Uuid::nil(), "n", "c", "e", "co", "s");
        l.qualify();
        assert_eq!(l.status, LeadStatus::Qualified);
        l.disqualify();
        assert_eq!(l.status, LeadStatus::Disqualified);
    }

    #[test]
    fn mark_converted_transition() {
        let mut l = Lead::new(Uuid::nil(), "n", "c", "e", "co", "s");
        l.mark_converted();
        assert_eq!(l.status, LeadStatus::Converted);
    }
}
