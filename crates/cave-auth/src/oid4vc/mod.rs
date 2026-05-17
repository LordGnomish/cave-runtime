// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/protocol/oid4vc/

//! OpenID for Verifiable Credentials — issuance (OID4VCI) + presentation (OID4VP).
//!
//! Port of Keycloak's `protocol/oid4vc/` (Java 24+ extension). Two
//! halves:
//!
//! * **Issuance (OID4VCI)** — `issuance::credential_offer` + `issuance::credential_endpoint`.
//!   Issuer-side: hand a wallet a credential offer, accept a pre-authorized-code
//!   grant, return a W3C VC Data Model 2.0 credential signed with Ed25519.
//! * **Presentation (OID4VP)** — `presentation::authz_request` + `presentation::authz_response`.
//!   Verifier-side: ask a wallet for credentials matching a
//!   `presentation_definition`, validate the returned `vp_token`.
//!
//! All credentials use the [W3C VC Data Model 2.0](https://www.w3.org/TR/vc-data-model-2.0/)
//! JSON-LD encoding and the [Ed25519 DataIntegrityProof](https://www.w3.org/TR/vc-di-eddsa/)
//! `eddsa-rdfc-2022` suite. JCS (RFC 8785) is used in place of full
//! RDF-c14n2 — see [`vc::proof`] for the honest limitation note.

pub mod issuance;
pub mod presentation;
pub mod vc;

use thiserror::Error;

/// Errors from the OID4VC surface.
#[derive(Debug, Error)]
pub enum Oid4vcError {
    /// JSON parse / shape error.
    #[error("oid4vc parse: {0}")]
    Parse(String),
    /// Required field missing.
    #[error("oid4vc missing field: {0}")]
    MissingField(String),
    /// Invalid grant (bad pre-authorized code, expired authorization code, etc.).
    #[error("oid4vc invalid grant: {0}")]
    InvalidGrant(String),
    /// Signature verification failed.
    #[error("oid4vc signature: {0}")]
    Signature(String),
    /// Presentation definition didn't match available credentials.
    #[error("oid4vc no_match")]
    NoMatchingCredentials,
}

/// Build the OID4VC router. Mount at `/oid4vc`.
pub fn router(state: issuance::credential_endpoint::IssuerState) -> axum::Router {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/credential_offer", get(issuance::credential_offer::handle_offer))
        .route("/credential", post(issuance::credential_endpoint::handle_issue))
        .route("/oid4vp/authz", get(presentation::authz_request::handle_authz_request))
        .route("/oid4vp/authz_response", post(presentation::authz_response::handle_authz_response))
        .with_state(state)
}
