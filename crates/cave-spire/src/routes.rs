//! HTTP routes for cave-spire.

use crate::models::*;
use crate::SpireState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;

pub fn create_router(state: Arc<SpireState>) -> Router {
    Router::new()
        // Trust Domains
        .route("/api/v1/spire/trustdomains", get(list_trust_domains).post(create_trust_domain))
        .route("/api/v1/spire/trustdomains/{name}", get(get_trust_domain).delete(delete_trust_domain))
        // Registration Entries
        .route("/api/v1/spire/entries", get(list_entries).post(create_entry))
        .route("/api/v1/spire/entries/{id}", get(get_entry).delete(delete_entry))
        // SVIDs
        .route("/api/v1/spire/svids/x509/mint", post(mint_x509))
        .route("/api/v1/spire/svids/jwt/mint", post(mint_jwt))
        .route("/api/v1/spire/svids/x509/{id}", get(get_x509_svid))
        .route("/api/v1/spire/svids/jwt/{id}", get(get_jwt_svid))
        // Agents
        .route("/api/v1/spire/agents", get(list_agents))
        .route("/api/v1/spire/agents/attest", post(attest_agent))
        .route("/api/v1/spire/agents/{id}", get(get_agent).delete(delete_agent))
        .route("/api/v1/spire/agents/{id}/ban", post(ban_agent))
        // Federation
        .route("/api/v1/spire/federation", get(list_federation).post(create_federation))
        .route("/api/v1/spire/federation/{trust_domain}", get(get_federation).delete(delete_federation))
        .with_state(state)
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<serde_json::Value>)>;

fn err(code: StatusCode, msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (code, Json(serde_json::json!({ "error": msg })))
}
fn map_err(e: crate::error::SpireError) -> (StatusCode, Json<serde_json::Value>) {
    err(StatusCode::from_u16(e.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), &e.to_string())
}

async fn list_trust_domains(State(state): State<Arc<SpireState>>) -> Json<serde_json::Value> {
    let domains = state.trust_domains.list();
    Json(serde_json::json!({ "trust_domains": domains, "count": domains.len() }))
}
async fn create_trust_domain(State(state): State<Arc<SpireState>>, Json(req): Json<CreateTrustDomainRequest>) -> ApiResult<serde_json::Value> {
    state.trust_domains.create(req).map(|d| Json(serde_json::json!({ "trust_domain": d }))).map_err(map_err)
}
async fn get_trust_domain(Path(name): Path<String>, State(state): State<Arc<SpireState>>) -> ApiResult<serde_json::Value> {
    state.trust_domains.get(&name).map(|d| Json(serde_json::json!({ "trust_domain": d }))).map_err(map_err)
}
async fn delete_trust_domain(Path(name): Path<String>, State(state): State<Arc<SpireState>>) -> ApiResult<serde_json::Value> {
    state.trust_domains.delete(&name).map(|_| Json(serde_json::json!({ "deleted": true }))).map_err(map_err)
}

#[derive(Deserialize)]
struct EntryQuery { trust_domain: Option<String> }

async fn list_entries(Query(q): Query<EntryQuery>, State(state): State<Arc<SpireState>>) -> Json<serde_json::Value> {
    let entries = state.registrations.list(q.trust_domain.as_deref());
    Json(serde_json::json!({ "entries": entries, "count": entries.len() }))
}
async fn create_entry(State(state): State<Arc<SpireState>>, Json(req): Json<CreateRegistrationEntryRequest>) -> ApiResult<serde_json::Value> {
    state.registrations.create(req).map(|e| Json(serde_json::json!({ "entry": e }))).map_err(map_err)
}
async fn get_entry(Path(id): Path<String>, State(state): State<Arc<SpireState>>) -> ApiResult<serde_json::Value> {
    state.registrations.get(&id).map(|e| Json(serde_json::json!({ "entry": e }))).map_err(map_err)
}
async fn delete_entry(Path(id): Path<String>, State(state): State<Arc<SpireState>>) -> ApiResult<serde_json::Value> {
    state.registrations.delete(&id).map(|_| Json(serde_json::json!({ "deleted": true }))).map_err(map_err)
}

async fn mint_x509(State(state): State<Arc<SpireState>>, Json(req): Json<MintX509SvidRequest>) -> ApiResult<serde_json::Value> {
    state.svids.mint_x509(req).map(|s| Json(serde_json::json!({ "svid": s }))).map_err(map_err)
}
async fn mint_jwt(State(state): State<Arc<SpireState>>, Json(req): Json<MintJwtSvidRequest>) -> ApiResult<serde_json::Value> {
    state.svids.mint_jwt(req).map(|s| Json(serde_json::json!({ "svid": s }))).map_err(map_err)
}
async fn get_x509_svid(Path(id): Path<String>, State(state): State<Arc<SpireState>>) -> ApiResult<serde_json::Value> {
    state.svids.get_x509(&id).map(|s| Json(serde_json::json!({ "svid": s }))).map_err(map_err)
}
async fn get_jwt_svid(Path(id): Path<String>, State(state): State<Arc<SpireState>>) -> ApiResult<serde_json::Value> {
    state.svids.get_jwt(&id).map(|s| Json(serde_json::json!({ "svid": s }))).map_err(map_err)
}

#[derive(Deserialize)]
struct AgentQuery { namespace: Option<String> }

async fn list_agents(Query(q): Query<AgentQuery>, State(state): State<Arc<SpireState>>) -> Json<serde_json::Value> {
    let agents = state.agents.list(q.namespace.as_deref());
    Json(serde_json::json!({ "agents": agents, "count": agents.len() }))
}
async fn attest_agent(State(state): State<Arc<SpireState>>, Json(req): Json<AttestAgentRequest>) -> ApiResult<serde_json::Value> {
    state.agents.attest(req).map(|a| Json(serde_json::json!({ "agent": a }))).map_err(map_err)
}
async fn get_agent(Path(id): Path<String>, State(state): State<Arc<SpireState>>) -> ApiResult<serde_json::Value> {
    state.agents.get(&id).map(|a| Json(serde_json::json!({ "agent": a }))).map_err(map_err)
}
async fn delete_agent(Path(id): Path<String>, State(state): State<Arc<SpireState>>) -> ApiResult<serde_json::Value> {
    state.agents.delete(&id).map(|_| Json(serde_json::json!({ "deleted": true }))).map_err(map_err)
}
async fn ban_agent(Path(id): Path<String>, State(state): State<Arc<SpireState>>) -> ApiResult<serde_json::Value> {
    state.agents.ban(&id).map(|a| Json(serde_json::json!({ "agent": a }))).map_err(map_err)
}

async fn list_federation(State(state): State<Arc<SpireState>>) -> Json<serde_json::Value> {
    let rels = state.federation.list();
    Json(serde_json::json!({ "federation_relationships": rels, "count": rels.len() }))
}
async fn create_federation(State(state): State<Arc<SpireState>>, Json(req): Json<CreateFederationRequest>) -> ApiResult<serde_json::Value> {
    state.federation.create(req).map(|r| Json(serde_json::json!({ "relationship": r }))).map_err(map_err)
}
async fn get_federation(Path(trust_domain): Path<String>, State(state): State<Arc<SpireState>>) -> ApiResult<serde_json::Value> {
    state.federation.get(&trust_domain).map(|r| Json(serde_json::json!({ "relationship": r }))).map_err(map_err)
}
async fn delete_federation(Path(trust_domain): Path<String>, State(state): State<Arc<SpireState>>) -> ApiResult<serde_json::Value> {
    state.federation.delete(&trust_domain).map(|_| Json(serde_json::json!({ "deleted": true }))).map_err(map_err)
}
