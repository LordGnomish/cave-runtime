//! DEPRECATED: This module is being phased out in favor of standalone Twenty CRM
//! (`crates/cave-crm/`). See `docs/adr/ADR-145_CRM_Upstream_Selection_Twenty.md`.
//! Will be removed in v0.2 OSS launch hijyen.

use crate::models::*;
use crate::store::ErpStore;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
pub struct CreateLeadRequest {
    pub name: String,
    pub contact_name: String,
    pub email: String,
    pub phone: Option<String>,
    pub company: String,
    pub source: String,
    pub assigned_to: Option<Uuid>,
}

#[derive(Serialize, Deserialize)]
pub struct ConvertLeadRequest {
    pub salesperson_id: Option<Uuid>,
}

#[derive(Serialize, Deserialize)]
pub struct CreateOpportunityRequest {
    pub name: String,
    pub stage_id: Uuid,
    pub amount: f64,
    pub currency: String,
    pub close_date: chrono::DateTime<chrono::Utc>,
    pub partner_id: Uuid,
    pub owner_id: Uuid,
}

#[derive(Serialize, Deserialize)]
pub struct CreateActivityRequest {
    pub entity_type: ActivityEntityType,
    pub entity_id: Uuid,
    pub activity_type: ActivityType,
    pub due_at: chrono::DateTime<chrono::Utc>,
    pub note: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct CreatePartnerRequest {
    pub name: String,
    pub is_customer: bool,
    pub is_supplier: bool,
    pub email: String,
    pub phone: Option<String>,
    pub tags: Option<Vec<String>>,
}

// Handlers
async fn create_lead(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateLeadRequest>,
) -> impl IntoResponse {
    let lead = Lead {
        id: Uuid::new_v4(),
        name: req.name,
        contact_name: req.contact_name,
        email: req.email,
        phone: req.phone,
        company: req.company,
        source: req.source,
        status: LeadStatus::New,
        created_at: Utc::now(),
        assigned_to: req.assigned_to,
    };
    let id = lead.id;
    store.leads.write().await.insert(id, lead.clone());
    (StatusCode::CREATED, Json(lead))
}

async fn list_leads(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let leads: Vec<_> = store.leads.read().await.values().cloned().collect();
    Json(leads)
}

async fn convert_lead(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
    Json(_req): Json<ConvertLeadRequest>,
) -> impl IntoResponse {
    let mut leads = store.leads.write().await;
    let Some(lead_snapshot) = leads.get(&id).cloned() else {
        return (StatusCode::NOT_FOUND, Json(Lead {
            id: Uuid::nil(),
            name: String::new(),
            contact_name: String::new(),
            email: String::new(),
            phone: None,
            company: String::new(),
            source: String::new(),
            status: LeadStatus::New,
            created_at: Utc::now(),
            assigned_to: None,
        }));
    };
    // Create partner from lead snapshot — no live borrow kept.
    let partner = Partner {
        id: Uuid::new_v4(),
        name: lead_snapshot.company.clone(),
        is_customer: true,
        is_supplier: false,
        email: lead_snapshot.email.clone(),
        phone: lead_snapshot.phone.clone(),
        billing_address: None,
        shipping_address: None,
        tags: vec![],
        created_at: Utc::now(),
    };
    drop(leads);
    {
        // Insert partner
        let partner_id = partner.id;
        store.partners.write().await.insert(partner_id, partner);

        // Create opportunity
        let stages = store.stages_crm.read().await;
        let first_stage = stages.values().min_by_key(|s| s.order).map(|s| s.id);
        drop(stages);

        let stage_id = first_stage.unwrap_or_else(Uuid::new_v4);

        let opportunity = Opportunity {
            id: Uuid::new_v4(),
            name: format!("Opp from {}", lead_snapshot.name),
            stage_id,
            amount: 0.0,
            currency: "EUR".to_string(),
            close_date: Utc::now() + chrono::Duration::days(30),
            probability: 30,
            partner_id,
            owner_id: Uuid::new_v4(),
            status: OpportunityStatus::Open,
            state_reason: None,
            created_at: Utc::now(),
        };

        let opp_id = opportunity.id;
        store.opportunities.write().await.insert(opp_id, opportunity);

        // Mark lead as converted
        let mut leads = store.leads.write().await;
        if let Some(lead) = leads.get_mut(&id) {
            lead.status = LeadStatus::Converted;
            (StatusCode::OK, Json(lead.clone()))
        } else {
            (StatusCode::NOT_FOUND, Json(Lead {
                id: Uuid::nil(),
                name: String::new(),
                contact_name: String::new(),
                email: String::new(),
                phone: None,
                company: String::new(),
                source: String::new(),
                status: LeadStatus::New,
                created_at: Utc::now(),
                assigned_to: None,
            }))
        }
    }
}

async fn create_opportunity(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateOpportunityRequest>,
) -> impl IntoResponse {
    let opp = Opportunity {
        id: Uuid::new_v4(),
        name: req.name,
        stage_id: req.stage_id,
        amount: req.amount,
        currency: req.currency,
        close_date: req.close_date,
        probability: 50,
        partner_id: req.partner_id,
        owner_id: req.owner_id,
        status: OpportunityStatus::Open,
        state_reason: None,
        created_at: Utc::now(),
    };
    let id = opp.id;
    store.opportunities.write().await.insert(id, opp.clone());
    (StatusCode::CREATED, Json(opp))
}

async fn list_opportunities(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let opps: Vec<_> = store.opportunities.read().await.values().cloned().collect();
    Json(opps)
}

async fn win_opportunity(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut opps = store.opportunities.write().await;
    if let Some(opp) = opps.get_mut(&id) {
        opp.status = OpportunityStatus::Won;
        (StatusCode::OK, Json(opp.clone()))
    } else {
        (StatusCode::NOT_FOUND, Json(Opportunity {
            id: Uuid::nil(),
            name: String::new(),
            stage_id: Uuid::nil(),
            amount: 0.0,
            currency: String::new(),
            close_date: Utc::now(),
            probability: 0,
            partner_id: Uuid::nil(),
            owner_id: Uuid::nil(),
            status: OpportunityStatus::Open,
            state_reason: None,
            created_at: Utc::now(),
        }))
    }
}

async fn lose_opportunity(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut opps = store.opportunities.write().await;
    if let Some(opp) = opps.get_mut(&id) {
        opp.status = OpportunityStatus::Lost;
        (StatusCode::OK, Json(opp.clone()))
    } else {
        (StatusCode::NOT_FOUND, Json(Opportunity {
            id: Uuid::nil(),
            name: String::new(),
            stage_id: Uuid::nil(),
            amount: 0.0,
            currency: String::new(),
            close_date: Utc::now(),
            probability: 0,
            partner_id: Uuid::nil(),
            owner_id: Uuid::nil(),
            status: OpportunityStatus::Open,
            state_reason: None,
            created_at: Utc::now(),
        }))
    }
}

async fn create_activity(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateActivityRequest>,
) -> impl IntoResponse {
    let activity = Activity {
        id: Uuid::new_v4(),
        entity_type: req.entity_type,
        entity_id: req.entity_id,
        activity_type: req.activity_type,
        due_at: req.due_at,
        note: req.note,
        status: ActivityStatus::Planned,
        created_at: Utc::now(),
    };
    let id = activity.id;
    store.activities.write().await.insert(id, activity.clone());
    (StatusCode::CREATED, Json(activity))
}

async fn list_activities(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let activities: Vec<_> = store.activities.read().await.values().cloned().collect();
    Json(activities)
}

async fn complete_activity(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut activities = store.activities.write().await;
    if let Some(activity) = activities.get_mut(&id) {
        activity.status = ActivityStatus::Done;
        (StatusCode::OK, Json(activity.clone()))
    } else {
        (StatusCode::NOT_FOUND, Json(Activity {
            id: Uuid::nil(),
            entity_type: ActivityEntityType::Lead,
            entity_id: Uuid::nil(),
            activity_type: ActivityType::Call,
            due_at: Utc::now(),
            note: None,
            status: ActivityStatus::Planned,
            created_at: Utc::now(),
        }))
    }
}

async fn create_partner(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreatePartnerRequest>,
) -> impl IntoResponse {
    let partner = Partner {
        id: Uuid::new_v4(),
        name: req.name,
        is_customer: req.is_customer,
        is_supplier: req.is_supplier,
        email: req.email,
        phone: req.phone,
        billing_address: None,
        shipping_address: None,
        tags: req.tags.unwrap_or_default(),
        created_at: Utc::now(),
    };
    let id = partner.id;
    store.partners.write().await.insert(id, partner.clone());
    (StatusCode::CREATED, Json(partner))
}

async fn list_partners(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let partners: Vec<_> = store.partners.read().await.values().cloned().collect();
    Json(partners)
}

pub fn create_router(state: Arc<ErpStore>) -> Router {
    Router::new()
        .route("/api/erp/crm/leads", post(create_lead).get(list_leads))
        .route("/api/erp/crm/leads/{id}/convert", post(convert_lead))
        .route(
            "/api/erp/crm/opportunities",
            post(create_opportunity).get(list_opportunities),
        )
        .route("/api/erp/crm/opportunities/{id}/win", post(win_opportunity))
        .route("/api/erp/crm/opportunities/{id}/lose", post(lose_opportunity))
        .route(
            "/api/erp/crm/activities",
            post(create_activity).get(list_activities),
        )
        .route(
            "/api/erp/crm/activities/{id}/complete",
            post(complete_activity),
        )
        .route(
            "/api/erp/crm/partners",
            post(create_partner).get(list_partners),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_lead_sets_status() {
        let mut lead = Lead {
            id: Uuid::new_v4(),
            name: "TechCorp".to_string(),
            contact_name: "Bob".to_string(),
            email: "bob@techcorp.com".to_string(),
            phone: None,
            company: "TechCorp Inc".to_string(),
            source: "Website".to_string(),
            status: LeadStatus::New,
            created_at: Utc::now(),
            assigned_to: None,
        };

        assert_eq!(lead.status, LeadStatus::New);
        lead.status = LeadStatus::Converted;
        assert_eq!(lead.status, LeadStatus::Converted);
    }

    #[test]
    fn test_win_opportunity_sets_status() {
        let mut opp = Opportunity {
            id: Uuid::new_v4(),
            name: "Big Deal".to_string(),
            stage_id: Uuid::new_v4(),
            amount: 100000.0,
            currency: "EUR".to_string(),
            close_date: Utc::now(),
            probability: 80,
            partner_id: Uuid::new_v4(),
            owner_id: Uuid::new_v4(),
            status: OpportunityStatus::Open,
            state_reason: None,
            created_at: Utc::now(),
        };

        assert_eq!(opp.status, OpportunityStatus::Open);
        opp.status = OpportunityStatus::Won;
        assert_eq!(opp.status, OpportunityStatus::Won);
    }

    #[test]
    fn test_complete_activity_sets_done() {
        let mut activity = Activity {
            id: Uuid::new_v4(),
            entity_type: ActivityEntityType::Opportunity,
            entity_id: Uuid::new_v4(),
            activity_type: ActivityType::Meeting,
            due_at: Utc::now(),
            note: Some("Discussed pricing".to_string()),
            status: ActivityStatus::Planned,
            created_at: Utc::now(),
        };

        assert_eq!(activity.status, ActivityStatus::Planned);
        activity.status = ActivityStatus::Done;
        assert_eq!(activity.status, ActivityStatus::Done);
    }
}
