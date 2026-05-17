// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/protocol/oid4vc/

//! OID4VC — RED phase: tests defined, implementation lands in GREEN.

pub mod issuance;
pub mod presentation;
pub mod vc;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Oid4vcError {
    #[error("oid4vc parse: {0}")]
    Parse(String),
    #[error("oid4vc missing field: {0}")]
    MissingField(String),
    #[error("oid4vc invalid grant: {0}")]
    InvalidGrant(String),
    #[error("oid4vc signature: {0}")]
    Signature(String),
    #[error("oid4vc no_match")]
    NoMatchingCredentials,
}

pub fn router(state: issuance::credential_endpoint::IssuerState) -> axum::Router {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/credential_offer", get(issuance::credential_offer::handle_offer))
        .route("/credential", post(issuance::credential_endpoint::handle_issue))
        .route("/oid4vp/authz", get(presentation::authz_request::handle_authz_request))
        .route("/oid4vp/authz_response", post(presentation::authz_response::handle_authz_response))
        .with_state(state)
}
