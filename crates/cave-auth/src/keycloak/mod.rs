// SPDX-License-Identifier: AGPL-3.0-or-later
//! Keycloak-compatible admin and OIDC endpoints.
//!
//! Modules:
//! - [`realm`] — Realm CRUD (/admin/realms)
//! - [`client`] — Client CRUD (/admin/realms/{realm}/clients)
//! - [`user`] — User CRUD (/admin/realms/{realm}/users)
//! - [`token_endpoint`] — OIDC token/userinfo/logout/introspect
//!                       (incl. authorization_code + device_code + CIBA grants)
//! - [`auth_endpoint`] — OAuth 2.0 / OIDC authorization endpoint
//! - [`device_endpoint`] — RFC 8628 device authorization
//! - [`revoke_endpoint`] — RFC 7009 token revocation
//! - [`par_endpoint`] — RFC 9126 Pushed Authorization Requests
//! - [`ciba_endpoint`] — OIDC Client-Initiated Backchannel Authentication
//! - [`admin`] — admin REST (IdentityProvider + AuthenticationFlow)
//! - [`discovery`] — OpenID discovery (.well-known/openid-configuration)
//! - [`pqc`] — PQC-hybrid ML-DSA-65+Ed25519 JWT signing
//! - [`metrics`] — Prometheus counters for every endpoint

pub mod admin;
pub mod auth_endpoint;
pub mod ciba_endpoint;
pub mod client;
pub mod device_endpoint;
pub mod discovery;
pub mod metrics;
pub mod par_endpoint;
pub mod pqc;
pub mod realm;
pub mod revoke_endpoint;
pub mod token_endpoint;
pub mod user;

use crate::keycloak::{
    auth_endpoint::AuthorizationService,
    ciba_endpoint::CibaService,
    client::ClientStore,
    device_endpoint::DeviceService,
    discovery::router as discovery_router,
    par_endpoint::ParService,
    realm::RealmStore,
    revoke_endpoint::RevokeService,
    token_endpoint::{router as token_router, KeycloakTokenService},
    user::UserStore,
};

/// Build the combined Keycloak router with shared stores.
///
/// The token service is rebuilt internally so the `auth_endpoint::AuthCodeStore`,
/// `device_endpoint::DeviceCodeStore`, and `ciba_endpoint::CibaRequestStore`
/// instances are shared between issuance and redemption sites.
pub fn router(
    realm_store: RealmStore,
    client_store: ClientStore,
    user_store: UserStore,
    token_service: KeycloakTokenService,
) -> axum::Router {
    let auth_svc = AuthorizationService::new(realm_store.clone(), user_store.clone(), client_store.clone());
    let device_svc = DeviceService::new(realm_store.clone(), client_store.clone());
    let ciba_svc = CibaService::new(realm_store.clone(), user_store.clone(), client_store.clone());
    let par_svc = ParService::new(auth_svc.par.clone(), client_store.clone());
    let revoke_svc = RevokeService::new(client_store.clone());

    // Re-wire the token service to share stores end-to-end.
    let token_service = token_service
        .with_auth_codes(auth_svc.codes.clone())
        .with_device_codes(device_svc.codes.clone())
        .with_ciba_requests(ciba_svc.requests.clone());

    axum::Router::new()
        .merge(realm::router(realm_store.clone()))
        .merge(client::router(client_store.clone(), realm_store.clone()))
        .merge(user::router(user_store.clone(), realm_store.clone()))
        .merge(token_router(token_service.clone()))
        .merge(discovery_router(token_service))
        .merge(auth_endpoint::router(auth_svc))
        .merge(device_endpoint::router(device_svc))
        .merge(par_endpoint::router(par_svc))
        .merge(ciba_endpoint::router(ciba_svc))
        .merge(revoke_endpoint::router(revoke_svc))
        .merge(admin::router())
}
