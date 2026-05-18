// SPDX-License-Identifier: AGPL-3.0-or-later
//! Twenty CRM data model placeholders.
//!
//! Mirrors Twenty's core objects: Person, Company, Opportunity, Activity.
//! Field set is intentionally minimal at scaffold time; expanded by qwen-amele
//! drafts against the parity manifest.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Person {
    pub id: Uuid,
    pub first_name: String,
    pub last_name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub job_title: Option<String>,
    pub company_id: Option<Uuid>,
    pub city: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Company {
    pub id: Uuid,
    pub name: String,
    pub domain_name: Option<String>,
    pub address: Option<String>,
    pub employees: Option<u32>,
    pub annual_recurring_revenue: Option<f64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OpportunityStage {
    New,
    Screening,
    Meeting,
    Proposal,
    Customer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Opportunity {
    pub id: Uuid,
    pub name: String,
    pub stage: OpportunityStage,
    pub amount: Option<f64>,
    pub close_date: Option<DateTime<Utc>>,
    pub probability: Option<u8>,
    pub company_id: Option<Uuid>,
    pub point_of_contact_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActivityType {
    Note,
    Task,
    Email,
    Call,
    Meeting,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Activity {
    pub id: Uuid,
    pub activity_type: ActivityType,
    pub title: String,
    pub body: Option<String>,
    pub due_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub assignee_id: Option<Uuid>,
    pub target_person_id: Option<Uuid>,
    pub target_company_id: Option<Uuid>,
    pub target_opportunity_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
