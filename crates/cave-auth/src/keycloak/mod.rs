// SPDX-License-Identifier: AGPL-3.0-or-later
//! Keycloak-compatible admin and OIDC endpoints.
//!
//! Modules:
//! - [`realm`] — Realm CRUD (/admin/realms)
//! - [`client`] — Client CRUD (/admin/realms/{realm}/clients)
//! - [`user`] — User CRUD (/admin/realms/{realm}/users)
//! - [`token_endpoint`] — OIDC token/userinfo/logout/introspect
//! - [`discovery`] — OpenID discovery (.well-known/openid-configuration)
//! - [`pqc`] — PQC-hybrid ML-DSA-65+Ed25519 JWT signing

pub mod client;
pub mod discovery;
pub mod pqc;
pub mod realm;
pub mod token_endpoint;
pub mod user;

use crate::keycloak::{
    client::ClientStore,
    discovery::router as discovery_router,
    realm::RealmStore,
    token_endpoint::{router as token_router, KeycloakTokenService},
    user::UserStore,
};

/// Build the combined Keycloak router with shared stores.
pub fn router(
    realm_store: RealmStore,
    client_store: ClientStore,
    user_store: UserStore,
    token_service: KeycloakTokenService,
) -> axum::Router {
    axum::Router::new()
        .merge(realm::router(realm_store.clone()))
        .merge(client::router(client_store.clone(), realm_store.clone()))
        .merge(user::router(user_store.clone(), realm_store.clone()))
        .merge(token_router(token_service.clone()))
        .merge(discovery_router(token_service))
}
