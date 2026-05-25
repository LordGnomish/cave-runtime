// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Twenty CRM Opportunity — `packages/twenty-server/src/modules/opportunity/standard-objects/opportunity.workspace-entity.ts`

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OpportunityStatus {
    Open,
    Won,
    Lost,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Opportunity {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    /// Foreign key into `PipelineStep`. Twenty uses this for kanban grouping.
    pub pipeline_step_id: Uuid,
    pub status: OpportunityStatus,
    pub amount: Option<f64>,
    pub currency: String,
    pub close_date: Option<DateTime<Utc>>,
    /// `[0..100]` probability — Twenty validates the bound server-side.
    pub probability: u8,
    pub company_id: Option<Uuid>,
    pub point_of_contact_id: Option<Uuid>,
    pub owner_user_id: Option<Uuid>,
    pub position: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Opportunity {
    pub fn new(workspace_id: Uuid, name: impl Into<String>, pipeline_step_id: Uuid) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            name: name.into(),
            pipeline_step_id,
            status: OpportunityStatus::Open,
            amount: None,
            currency: "USD".to_string(),
            close_date: None,
            probability: 30,
            company_id: None,
            point_of_contact_id: None,
            owner_user_id: None,
            position: 0,
            created_at: now,
            updated_at: now,
        }
    }

    /// Mark as won. The win-rate goes to 100; the close date is stamped
    /// "now" so reports see a stable timestamp.
    pub fn mark_won(&mut self) {
        self.status = OpportunityStatus::Won;
        self.probability = 100;
        let now = Utc::now();
        self.close_date = Some(now);
        self.updated_at = now;
    }

    pub fn mark_lost(&mut self) {
        self.status = OpportunityStatus::Lost;
        self.probability = 0;
        let now = Utc::now();
        self.close_date = Some(now);
        self.updated_at = now;
    }

    /// Move into a new pipeline step (kanban drag-drop).
    pub fn move_to(&mut self, pipeline_step_id: Uuid) {
        self.pipeline_step_id = pipeline_step_id;
        self.updated_at = Utc::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_opp_starts_open_with_30_probability() {
        let opp = Opportunity::new(Uuid::nil(), "Big Deal", Uuid::nil());
        assert_eq!(opp.status, OpportunityStatus::Open);
        assert_eq!(opp.probability, 30);
        assert!(opp.close_date.is_none());
    }

    #[test]
    fn mark_won_sets_status_probability_and_close_date() {
        let mut opp = Opportunity::new(Uuid::nil(), "Big Deal", Uuid::nil());
        opp.mark_won();
        assert_eq!(opp.status, OpportunityStatus::Won);
        assert_eq!(opp.probability, 100);
        assert!(opp.close_date.is_some());
    }

    #[test]
    fn mark_lost_zeros_probability() {
        let mut opp = Opportunity::new(Uuid::nil(), "Big Deal", Uuid::nil());
        opp.mark_lost();
        assert_eq!(opp.status, OpportunityStatus::Lost);
        assert_eq!(opp.probability, 0);
    }

    #[test]
    fn move_to_updates_step_and_timestamp() {
        let mut opp = Opportunity::new(Uuid::nil(), "Big Deal", Uuid::nil());
        let original = opp.updated_at;
        std::thread::sleep(std::time::Duration::from_millis(2));
        let new_step = Uuid::new_v4();
        opp.move_to(new_step);
        assert_eq!(opp.pipeline_step_id, new_step);
        assert!(opp.updated_at > original);
    }
}
