// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: upstream is spiffe/spire (Apache-2.0). REST endpoint shape inspired
// by pkg/server/endpoints/handler.go (gRPC) and adapted to axum.
//
//! HTTP API routes for cave-identity (`/api/identity/...`).
//!
//! Surface:
//! - `POST /entry`                — create registration entry
//! - `GET  /entries`              — list registration entries
//! - `GET  /entries/{id}`         — fetch a single entry
//! - `DELETE /entries/{id}`       — delete a registration entry
//! - `POST /agent/attest`         — attest an agent (node-attestor)
//! - `GET  /agents`               — list attested agents
//! - `GET  /bundle`               — fetch own trust bundle (JWKS doc)
//! - `POST /federation/bundle`    — push a federated bundle from a peer
//! - `GET  /federation`           — list federation relationships
//! - `GET  /oidc/.well-known/openid-configuration` — OIDC discovery
//! - `GET  /oidc/keys`            — JWKS for JWT-SVID verifiers

use crate::agent_manager::AgentManager;
use crate::bundle::{self, BundleDoc};
use crate::error::IdentityError;
use crate::federation::FederationManager;
use crate::models::{AttestedNode, FederationRelationship, RegistrationEntry, TrustDomain};
use crate::oidc;
use crate::server_ca::ServerCa;
use crate::store::MemStore;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Shared HTTP state — wired by the orchestrator into a Tower service.
pub struct IdentityState {
    pub ca: Arc<ServerCa>,
    pub store: Arc<MemStore>,
    pub federation: Arc<FederationManager>,
    pub agents: Arc<AgentManager>,
    pub issuer_url: String,
}

pub fn create_router(state: Arc<IdentityState>) -> Router {
    Router::new()
        .route("/api/identity/entry", post(create_entry))
        .route("/api/identity/entries", get(list_entries))
        .route(
            "/api/identity/entries/{id}",
            get(get_entry).delete(delete_entry_route),
        )
        .route("/api/identity/agent/attest", post(attest_agent))
        .route("/api/identity/agents", get(list_agents))
        .route("/api/identity/bundle", get(get_bundle))
        .route("/api/identity/federation/bundle", post(push_federation_bundle))
        .route("/api/identity/federation", get(list_federations))
        .route(
            "/api/identity/oidc/.well-known/openid-configuration",
            get(oidc_discovery),
        )
        .route("/api/identity/oidc/keys", get(oidc_keys))
        .with_state(state)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateEntryReq {
    pub entry: RegistrationEntry,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateEntryResp {
    pub entry: RegistrationEntry,
}

async fn create_entry(
    State(state): State<Arc<IdentityState>>,
    Json(req): Json<CreateEntryReq>,
) -> Result<(StatusCode, Json<CreateEntryResp>), (StatusCode, String)> {
    let e = state
        .store
        .entries
        .create(req.entry)
        .map_err(to_http)?;
    Ok((StatusCode::CREATED, Json(CreateEntryResp { entry: e })))
}

async fn list_entries(
    State(state): State<Arc<IdentityState>>,
) -> Json<Vec<RegistrationEntry>> {
    Json(state.store.entries.list())
}

async fn get_entry(
    State(state): State<Arc<IdentityState>>,
    Path(id): Path<String>,
) -> Result<Json<RegistrationEntry>, (StatusCode, String)> {
    let e = state.store.entries.get(&id).map_err(to_http)?;
    Ok(Json(e))
}

async fn delete_entry_route(
    State(state): State<Arc<IdentityState>>,
    Path(id): Path<String>,
) -> Result<Json<RegistrationEntry>, (StatusCode, String)> {
    let e = state.store.entries.delete(&id).map_err(to_http)?;
    Ok(Json(e))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AttestAgentReq {
    pub node: AttestedNode,
}

async fn attest_agent(
    State(state): State<Arc<IdentityState>>,
    Json(req): Json<AttestAgentReq>,
) -> Result<(StatusCode, Json<AttestedNode>), (StatusCode, String)> {
    state.store.put_agent(req.node.clone()).map_err(to_http)?;
    Ok((StatusCode::CREATED, Json(req.node)))
}

async fn list_agents(State(state): State<Arc<IdentityState>>) -> Json<Vec<AttestedNode>> {
    Json(state.store.list_agents())
}

async fn get_bundle(State(state): State<Arc<IdentityState>>) -> Json<BundleDoc> {
    Json(bundle::marshal(&state.ca.trust_bundle()))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PushFederationReq {
    pub trust_domain: String,
    pub doc: BundleDoc,
}

async fn push_federation_bundle(
    State(state): State<Arc<IdentityState>>,
    Json(req): Json<PushFederationReq>,
) -> Result<(StatusCode, Json<BundleDoc>), (StatusCode, String)> {
    let td = TrustDomain::new(req.trust_domain);
    let b = bundle::unmarshal(&td, &req.doc).map_err(to_http)?;
    state.store.put_bundle(b.clone()).map_err(to_http)?;
    Ok((StatusCode::CREATED, Json(bundle::marshal(&b))))
}

async fn list_federations(
    State(state): State<Arc<IdentityState>>,
) -> Json<Vec<FederationRelationship>> {
    Json(state.federation.list())
}

async fn oidc_discovery(State(state): State<Arc<IdentityState>>) -> Json<oidc::OidcDiscovery> {
    Json(oidc::OidcDiscovery::new(&state.issuer_url))
}

async fn oidc_keys(State(state): State<Arc<IdentityState>>) -> Json<oidc::JwkSet> {
    Json(oidc::jwks_for_bundle(&state.ca.trust_bundle()))
}

fn to_http(e: IdentityError) -> (StatusCode, String) {
    use IdentityError::*;
    let code = match &e {
        EntryNotFound(_) | BundleNotFound(_) | AttestorNotFound(_) => StatusCode::NOT_FOUND,
        EntryExists(_) => StatusCode::CONFLICT,
        InvalidSpiffeId(_)
        | InvalidTrustDomain(_)
        | JwtInvalid(_)
        | OidcInvalid(_)
        | FederationInvalid(_) => StatusCode::BAD_REQUEST,
        PolicyViolation(_) | AgentBanned(_) => StatusCode::FORBIDDEN,
        TtlOutOfBounds { .. } => StatusCode::BAD_REQUEST,
        AttestationFailed(_) => StatusCode::UNAUTHORIZED,
        SvidIssuanceFailed(_) | SvidVerificationFailed(_) | CaNotInitialised => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
        FederationUnreachable(_) => StatusCode::BAD_GATEWAY,
        Io(_) | Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (code, e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::federation::StubBundleFetcher;
    use crate::server_ca::RotationParams;
    use chrono::Utc;

    fn state() -> Arc<IdentityState> {
        let ca = ServerCa::new(TrustDomain::new("example.org"), RotationParams::default());
        ca.bootstrap(Utc::now()).unwrap();
        let ca = Arc::new(ca);
        let store = Arc::new(MemStore::new());
        let federation = Arc::new(FederationManager::new(
            store.clone(),
            Arc::new(StubBundleFetcher::default()),
        ));
        let agents = Arc::new(AgentManager::new(ca.clone()));
        Arc::new(IdentityState {
            ca,
            store,
            federation,
            agents,
            issuer_url: "https://spire.example.org".into(),
        })
    }

    #[test]
    fn http_error_mapping() {
        let (code, _) = to_http(IdentityError::EntryNotFound("x".into()));
        assert_eq!(code, StatusCode::NOT_FOUND);
        let (code, _) = to_http(IdentityError::EntryExists("y".into()));
        assert_eq!(code, StatusCode::CONFLICT);
        let (code, _) = to_http(IdentityError::PolicyViolation("p".into()));
        assert_eq!(code, StatusCode::FORBIDDEN);
        let (code, _) = to_http(IdentityError::InvalidSpiffeId("s".into()));
        assert_eq!(code, StatusCode::BAD_REQUEST);
        let (code, _) = to_http(IdentityError::AttestationFailed("a".into()));
        assert_eq!(code, StatusCode::UNAUTHORIZED);
        let (code, _) = to_http(IdentityError::FederationUnreachable("u".into()));
        assert_eq!(code, StatusCode::BAD_GATEWAY);
        let (code, _) = to_http(IdentityError::TtlOutOfBounds {
            requested: 1,
            min: 60,
            max: 600,
        });
        assert_eq!(code, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn router_builds() {
        let r = create_router(state());
        let _ = r; // smoke: type-checks Router<()>.
    }
}
