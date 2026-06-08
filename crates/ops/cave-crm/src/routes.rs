// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Cave CRM REST API v1.
//!
//! Twenty exposes both GraphQL (primary) and REST (auto-generated) surfaces.
//! The MVP REST surface mirrors Twenty's `/rest/{object_plural}` shape:
//!
//! ```text
//! GET    /api/crm/health
//! GET    /api/crm/workspaces
//! POST   /api/crm/workspaces                 # body: { "name": "Acme" } → bootstraps
//! GET    /api/crm/{workspace_id}/people
//! POST   /api/crm/{workspace_id}/people
//! GET    /api/crm/{workspace_id}/companies
//! POST   /api/crm/{workspace_id}/companies
//! GET    /api/crm/{workspace_id}/opportunities
//! POST   /api/crm/{workspace_id}/opportunities
//! POST   /api/crm/{workspace_id}/opportunities/{id}/win
//! POST   /api/crm/{workspace_id}/opportunities/{id}/lose
//! GET    /api/crm/{workspace_id}/leads
//! POST   /api/crm/{workspace_id}/leads
//! POST   /api/crm/{workspace_id}/leads/{id}/convert
//! GET    /api/crm/{workspace_id}/pipeline-steps
//! GET    /api/crm/{workspace_id}/notes
//! POST   /api/crm/{workspace_id}/notes
//! GET    /api/crm/{workspace_id}/tasks
//! POST   /api/crm/{workspace_id}/tasks
//! POST   /api/crm/{workspace_id}/tasks/{id}/complete
//! GET    /api/crm/{workspace_id}/calendar-events
//! POST   /api/crm/{workspace_id}/calendar-events
//! GET    /api/crm/{workspace_id}/views
//! POST   /api/crm/{workspace_id}/views
//! GET    /api/crm/{workspace_id}/object-metadata
//! GET    /api/crm/{workspace_id}/field-metadata
//! POST   /api/crm/{workspace_id}/field-metadata
//! GET    /api/crm/{workspace_id}/indexes
//! GET    /api/crm/{workspace_id}/graphql-schema    # text/plain SDL
//! ```

use crate::models::*;
use crate::store::CrmStore;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct CreateWorkspaceRequest {
    pub name: String,
}

#[derive(Deserialize)]
pub struct CreatePersonRequest {
    pub first_name: String,
    pub last_name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub job_title: Option<String>,
    pub company_id: Option<Uuid>,
}

#[derive(Deserialize)]
pub struct CreateCompanyRequest {
    pub name: String,
    pub domain_name: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateOpportunityRequest {
    pub name: String,
    pub pipeline_step_id: Uuid,
    pub amount: Option<f64>,
    pub currency: Option<String>,
    pub company_id: Option<Uuid>,
}

#[derive(Deserialize)]
pub struct CreateLeadRequest {
    pub name: String,
    pub contact_name: String,
    pub email: String,
    pub phone: Option<String>,
    pub company: String,
    pub source: String,
}

#[derive(Deserialize)]
pub struct CreateNoteRequest {
    pub title: String,
    pub body: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateTaskRequest {
    pub title: String,
    pub body: Option<String>,
    pub due_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Deserialize)]
pub struct CreateCalendarEventRequest {
    pub title: String,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub ends_at: chrono::DateTime<chrono::Utc>,
    pub description: Option<String>,
    pub location: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateViewRequest {
    pub object_metadata_id: Uuid,
    pub name: String,
    pub kind: ViewKind,
}

#[derive(Deserialize)]
pub struct CreateFieldMetadataRequest {
    pub object_metadata_id: Uuid,
    pub name: String,
    pub kind: FieldKind,
}

async fn health() -> impl IntoResponse {
    Json(json!({
        "module": "cave-crm",
        "status": "ok",
        "upstream": "twentyhq/twenty",
        "upstream_version": "v2.6.0",
    }))
}

async fn list_workspaces(State(s): State<Arc<CrmStore>>) -> impl IntoResponse {
    let v: Vec<Workspace> = s.workspaces.read().await.values().cloned().collect();
    Json(v)
}

async fn create_workspace(
    State(s): State<Arc<CrmStore>>,
    Json(req): Json<CreateWorkspaceRequest>,
) -> impl IntoResponse {
    let ws = s.bootstrap_workspace(req.name).await;
    (StatusCode::CREATED, Json(ws))
}

async fn list_people(State(s): State<Arc<CrmStore>>, Path(ws_id): Path<Uuid>) -> impl IntoResponse {
    let v: Vec<Person> = s
        .people
        .read()
        .await
        .values()
        .filter(|p| p.workspace_id == ws_id)
        .cloned()
        .collect();
    Json(v)
}

async fn create_person(
    State(s): State<Arc<CrmStore>>,
    Path(ws_id): Path<Uuid>,
    Json(req): Json<CreatePersonRequest>,
) -> impl IntoResponse {
    let mut p = Person::new(ws_id, req.first_name, req.last_name);
    p.email = req.email;
    p.phone = req.phone;
    p.job_title = req.job_title;
    p.company_id = req.company_id;
    let id = p.id;
    s.people.write().await.insert(id, p.clone());
    (StatusCode::CREATED, Json(p))
}

async fn list_companies(
    State(s): State<Arc<CrmStore>>,
    Path(ws_id): Path<Uuid>,
) -> impl IntoResponse {
    let v: Vec<Company> = s
        .companies
        .read()
        .await
        .values()
        .filter(|c| c.workspace_id == ws_id)
        .cloned()
        .collect();
    Json(v)
}

async fn create_company(
    State(s): State<Arc<CrmStore>>,
    Path(ws_id): Path<Uuid>,
    Json(req): Json<CreateCompanyRequest>,
) -> impl IntoResponse {
    let mut c = Company::new(ws_id, req.name);
    c.domain_name = req.domain_name;
    let id = c.id;
    s.companies.write().await.insert(id, c.clone());
    (StatusCode::CREATED, Json(c))
}

async fn list_opportunities(
    State(s): State<Arc<CrmStore>>,
    Path(ws_id): Path<Uuid>,
) -> impl IntoResponse {
    Json(s.opportunities_in_workspace(ws_id).await)
}

async fn create_opportunity(
    State(s): State<Arc<CrmStore>>,
    Path(ws_id): Path<Uuid>,
    Json(req): Json<CreateOpportunityRequest>,
) -> impl IntoResponse {
    let mut o = Opportunity::new(ws_id, req.name, req.pipeline_step_id);
    o.amount = req.amount;
    if let Some(cur) = req.currency {
        o.currency = cur;
    }
    o.company_id = req.company_id;
    let id = o.id;
    s.opportunities.write().await.insert(id, o.clone());
    (StatusCode::CREATED, Json(o))
}

async fn win_opportunity(
    State(s): State<Arc<CrmStore>>,
    Path((_ws_id, id)): Path<(Uuid, Uuid)>,
) -> impl IntoResponse {
    let mut opps = s.opportunities.write().await;
    let Some(o) = opps.get_mut(&id) else {
        return (StatusCode::NOT_FOUND, Json(json!({"error": "not found"})));
    };
    o.mark_won();
    (StatusCode::OK, Json(serde_json::to_value(o).unwrap()))
}

async fn lose_opportunity(
    State(s): State<Arc<CrmStore>>,
    Path((_ws_id, id)): Path<(Uuid, Uuid)>,
) -> impl IntoResponse {
    let mut opps = s.opportunities.write().await;
    let Some(o) = opps.get_mut(&id) else {
        return (StatusCode::NOT_FOUND, Json(json!({"error": "not found"})));
    };
    o.mark_lost();
    (StatusCode::OK, Json(serde_json::to_value(o).unwrap()))
}

async fn list_leads(State(s): State<Arc<CrmStore>>, Path(ws_id): Path<Uuid>) -> impl IntoResponse {
    let v: Vec<Lead> = s
        .leads
        .read()
        .await
        .values()
        .filter(|l| l.workspace_id == ws_id)
        .cloned()
        .collect();
    Json(v)
}

async fn create_lead(
    State(s): State<Arc<CrmStore>>,
    Path(ws_id): Path<Uuid>,
    Json(req): Json<CreateLeadRequest>,
) -> impl IntoResponse {
    let mut l = Lead::new(
        ws_id,
        req.name,
        req.contact_name,
        req.email,
        req.company,
        req.source,
    );
    l.phone = req.phone;
    let id = l.id;
    s.leads.write().await.insert(id, l.clone());
    (StatusCode::CREATED, Json(l))
}

async fn convert_lead(
    State(s): State<Arc<CrmStore>>,
    Path((_ws_id, id)): Path<(Uuid, Uuid)>,
) -> impl IntoResponse {
    match s.convert_lead(id).await {
        Some(c) => (
            StatusCode::CREATED,
            Json(json!({
                "company": c.company,
                "person": c.person,
                "opportunity": c.opportunity,
            })),
        ),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "lead not found or already converted"})),
        ),
    }
}

async fn list_pipeline_steps(
    State(s): State<Arc<CrmStore>>,
    Path(ws_id): Path<Uuid>,
) -> impl IntoResponse {
    let mut v: Vec<PipelineStep> = s
        .pipeline_steps
        .read()
        .await
        .values()
        .filter(|p| p.workspace_id == ws_id)
        .cloned()
        .collect();
    v.sort_by_key(|p| p.position);
    Json(v)
}

async fn list_notes(State(s): State<Arc<CrmStore>>, Path(ws_id): Path<Uuid>) -> impl IntoResponse {
    let v: Vec<Note> = s
        .notes
        .read()
        .await
        .values()
        .filter(|n| n.workspace_id == ws_id)
        .cloned()
        .collect();
    Json(v)
}

async fn create_note(
    State(s): State<Arc<CrmStore>>,
    Path(ws_id): Path<Uuid>,
    Json(req): Json<CreateNoteRequest>,
) -> impl IntoResponse {
    let mut n = Note::new(ws_id, req.title);
    if let Some(b) = req.body {
        n.body = b;
    }
    let id = n.id;
    s.notes.write().await.insert(id, n.clone());
    (StatusCode::CREATED, Json(n))
}

async fn list_tasks(State(s): State<Arc<CrmStore>>, Path(ws_id): Path<Uuid>) -> impl IntoResponse {
    let v: Vec<Task> = s
        .tasks
        .read()
        .await
        .values()
        .filter(|t| t.workspace_id == ws_id)
        .cloned()
        .collect();
    Json(v)
}

async fn create_task(
    State(s): State<Arc<CrmStore>>,
    Path(ws_id): Path<Uuid>,
    Json(req): Json<CreateTaskRequest>,
) -> impl IntoResponse {
    let mut t = Task::new(ws_id, req.title);
    if let Some(b) = req.body {
        t.body = b;
    }
    t.due_at = req.due_at;
    let id = t.id;
    s.tasks.write().await.insert(id, t.clone());
    (StatusCode::CREATED, Json(t))
}

async fn complete_task(
    State(s): State<Arc<CrmStore>>,
    Path((_ws_id, id)): Path<(Uuid, Uuid)>,
) -> impl IntoResponse {
    let mut tasks = s.tasks.write().await;
    let Some(t) = tasks.get_mut(&id) else {
        return (StatusCode::NOT_FOUND, Json(json!({"error": "not found"})));
    };
    t.complete();
    (StatusCode::OK, Json(serde_json::to_value(t).unwrap()))
}

async fn list_calendar_events(
    State(s): State<Arc<CrmStore>>,
    Path(ws_id): Path<Uuid>,
) -> impl IntoResponse {
    let v: Vec<CalendarEvent> = s
        .calendar_events
        .read()
        .await
        .values()
        .filter(|c| c.workspace_id == ws_id)
        .cloned()
        .collect();
    Json(v)
}

async fn create_calendar_event(
    State(s): State<Arc<CrmStore>>,
    Path(ws_id): Path<Uuid>,
    Json(req): Json<CreateCalendarEventRequest>,
) -> impl IntoResponse {
    let mut e = CalendarEvent::new(ws_id, req.title, req.starts_at, req.ends_at);
    if let Some(d) = req.description {
        e.description = d;
    }
    if let Some(l) = req.location {
        e.location = l;
    }
    let id = e.id;
    s.calendar_events.write().await.insert(id, e.clone());
    (StatusCode::CREATED, Json(e))
}

async fn list_views(State(s): State<Arc<CrmStore>>, Path(ws_id): Path<Uuid>) -> impl IntoResponse {
    let v: Vec<View> = s
        .views
        .read()
        .await
        .values()
        .filter(|v| v.workspace_id == ws_id)
        .cloned()
        .collect();
    Json(v)
}

async fn create_view(
    State(s): State<Arc<CrmStore>>,
    Path(ws_id): Path<Uuid>,
    Json(req): Json<CreateViewRequest>,
) -> impl IntoResponse {
    let v = View::new(ws_id, req.object_metadata_id, req.name, req.kind);
    let id = v.id;
    s.views.write().await.insert(id, v.clone());
    (StatusCode::CREATED, Json(v))
}

async fn list_object_metadata(
    State(s): State<Arc<CrmStore>>,
    Path(ws_id): Path<Uuid>,
) -> impl IntoResponse {
    let v: Vec<ObjectMetadata> = s
        .object_metadata
        .read()
        .await
        .values()
        .filter(|o| o.workspace_id == ws_id)
        .cloned()
        .collect();
    Json(v)
}

async fn list_field_metadata(
    State(s): State<Arc<CrmStore>>,
    Path(ws_id): Path<Uuid>,
) -> impl IntoResponse {
    let v: Vec<FieldMetadata> = s
        .field_metadata
        .read()
        .await
        .values()
        .filter(|f| f.workspace_id == ws_id)
        .cloned()
        .collect();
    Json(v)
}

async fn create_field_metadata(
    State(s): State<Arc<CrmStore>>,
    Path(ws_id): Path<Uuid>,
    Json(req): Json<CreateFieldMetadataRequest>,
) -> impl IntoResponse {
    if !FieldMetadata::is_valid_name(&req.name) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "name must match ^[a-z][a-z0-9_]*$"})),
        );
    }
    let f = FieldMetadata::new(ws_id, req.object_metadata_id, req.name, req.kind);
    let id = f.id;
    s.field_metadata.write().await.insert(id, f.clone());
    (StatusCode::CREATED, Json(serde_json::to_value(&f).unwrap()))
}

#[derive(Serialize)]
struct IndexList {
    workspace_id: Uuid,
    indexes: Vec<crate::indexes::IndexSpec>,
}

async fn list_indexes(
    State(s): State<Arc<CrmStore>>,
    Path(ws_id): Path<Uuid>,
) -> impl IntoResponse {
    let list = s.indexes.read().await.list(ws_id).to_vec();
    Json(IndexList {
        workspace_id: ws_id,
        indexes: list,
    })
}

async fn graphql_schema(
    State(s): State<Arc<CrmStore>>,
    Path(ws_id): Path<Uuid>,
) -> impl IntoResponse {
    let objs: Vec<ObjectMetadata> = s
        .object_metadata
        .read()
        .await
        .values()
        .filter(|o| o.workspace_id == ws_id)
        .cloned()
        .collect();
    let fields: Vec<FieldMetadata> = s
        .field_metadata
        .read()
        .await
        .values()
        .filter(|f| f.workspace_id == ws_id)
        .cloned()
        .collect();
    let sdl = crate::graphql_schema::render_workspace_schema(&objs, &fields);
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; charset=utf-8",
        )],
        sdl,
    )
}

#[derive(Deserialize)]
pub struct GraphQlRequest {
    pub query: String,
}

/// `POST /api/crm/{workspace_id}/graphql` — execute a read query against the
/// workspace's records via the in-memory resolver runtime. Mirrors Twenty's
/// GraphQL endpoint (findOne / findMany Connection envelope).
async fn graphql_query(
    State(s): State<Arc<CrmStore>>,
    Path(ws_id): Path<Uuid>,
    Json(req): Json<GraphQlRequest>,
) -> impl IntoResponse {
    Json(s.graphql_query(ws_id, &req.query).await)
}

pub fn create_router(state: Arc<CrmStore>) -> Router {
    Router::new()
        .route("/api/crm/health", get(health))
        .route(
            "/api/crm/workspaces",
            get(list_workspaces).post(create_workspace),
        )
        .route(
            "/api/crm/{workspace_id}/people",
            get(list_people).post(create_person),
        )
        .route(
            "/api/crm/{workspace_id}/companies",
            get(list_companies).post(create_company),
        )
        .route(
            "/api/crm/{workspace_id}/opportunities",
            get(list_opportunities).post(create_opportunity),
        )
        .route(
            "/api/crm/{workspace_id}/opportunities/{id}/win",
            post(win_opportunity),
        )
        .route(
            "/api/crm/{workspace_id}/opportunities/{id}/lose",
            post(lose_opportunity),
        )
        .route(
            "/api/crm/{workspace_id}/leads",
            get(list_leads).post(create_lead),
        )
        .route(
            "/api/crm/{workspace_id}/leads/{id}/convert",
            post(convert_lead),
        )
        .route(
            "/api/crm/{workspace_id}/pipeline-steps",
            get(list_pipeline_steps),
        )
        .route(
            "/api/crm/{workspace_id}/notes",
            get(list_notes).post(create_note),
        )
        .route(
            "/api/crm/{workspace_id}/tasks",
            get(list_tasks).post(create_task),
        )
        .route(
            "/api/crm/{workspace_id}/tasks/{id}/complete",
            post(complete_task),
        )
        .route(
            "/api/crm/{workspace_id}/calendar-events",
            get(list_calendar_events).post(create_calendar_event),
        )
        .route(
            "/api/crm/{workspace_id}/views",
            get(list_views).post(create_view),
        )
        .route(
            "/api/crm/{workspace_id}/object-metadata",
            get(list_object_metadata),
        )
        .route(
            "/api/crm/{workspace_id}/field-metadata",
            get(list_field_metadata).post(create_field_metadata),
        )
        .route("/api/crm/{workspace_id}/indexes", get(list_indexes))
        .route(
            "/api/crm/{workspace_id}/graphql-schema",
            get(graphql_schema),
        )
        .route("/api/crm/{workspace_id}/graphql", post(graphql_query))
        .with_state(state)
}
