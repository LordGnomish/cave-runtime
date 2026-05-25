// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Twenty CRM Company — `packages/twenty-server/src/modules/company/standard-objects/company.workspace-entity.ts`

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Company {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub domain_name: Option<String>,
    pub address: Option<String>,
    /// Headcount band. Twenty stores this as a free-form i32 so that
    /// "approximate" employee counts (10, 50, 200) can be filtered numerically.
    pub employees: Option<i32>,
    /// Annual Recurring Revenue — denominated in `currency`.
    pub annual_recurring_revenue: Option<f64>,
    pub currency: String,
    pub linkedin_url: Option<String>,
    pub x_url: Option<String>,
    pub idea_customer_profile: bool,
    pub position: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Company {
    pub fn new(workspace_id: Uuid, name: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            name: name.into(),
            domain_name: None,
            address: None,
            employees: None,
            annual_recurring_revenue: None,
            currency: "USD".to_string(),
            linkedin_url: None,
            x_url: None,
            idea_customer_profile: false,
            position: 0,
            created_at: now,
            updated_at: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn company_defaults_currency_to_usd() {
        let c = Company::new(Uuid::nil(), "Acme");
        assert_eq!(c.currency, "USD");
        assert!(!c.idea_customer_profile);
    }
}
