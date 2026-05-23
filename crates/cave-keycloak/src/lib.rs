// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cave-keycloak` — Keycloak IAM/SSO control plane reimplementation.
//!
//! Upstream: keycloak/keycloak v26.6.2 (Apache-2.0).
//! Source commit pin: see `parity.manifest.toml`.
//!
//! Surface:
//!   * realm + user + group + role + client CRUD with multi-tenancy
//!   * authentication: password (PBKDF2 / Argon2-style), TOTP (RFC 6238),
//!     WebAuthn assertion verify, magic-link
//!   * OAuth2 / OIDC provider: auth code + PKCE, client_credentials,
//!     device code (RFC 8628), refresh token rotation, ROPC (opt-in)
//!   * SAML2 IDP + SP (AuthnRequest in / Response out, Response verify)
//!   * JWT signing: RS256 / ES256 / EdDSA + PQC ML-DSA-65 placeholder
//!   * JWKS, discovery, introspection (RFC 7662), revocation (RFC 7009)
//!   * SSO + offline sessions, refresh token rotation
//!   * LDAP / AD federation (bind + search; sync deferred)
//!   * Identity brokering: Google / GitHub / Microsoft OIDC
//!   * Password policy, brute-force detection, audit event listener
//!   * Admin REST API + Account REST API
//!   * `cavectl iam {realm,user,role,client,session,event}`
//!
//! Non-goals (see `parity.manifest.toml::[[scope_cuts]]`):
//!   * admin console + account console UI runtime → `cave-portal-ui`
//!   * email theme rendering → `cave-templates`
//!   * LDAP write-back + federation sync daemon → Phase 2
//!   * Kerberos GSSAPI runtime → Phase 2
//!   * Vault-backed secrets → `cave-vault` adapter (model present)
//!   * Token Exchange (RFC 8693) runtime → Phase 2

use std::sync::Arc;

pub mod account_api;
pub mod admin_api;
pub mod auth_flow;
pub mod brokering;
pub mod client_registry;
pub mod credentials;
pub mod discovery;
pub mod error;
pub mod events;
pub mod jwks;
pub mod ldap;
pub mod metrics;
pub mod models;
pub mod oauth2;
pub mod policies;
pub mod realm;
pub mod role;
pub mod routes;
pub mod saml;
pub mod session;
pub mod signer;
pub mod store;
pub mod user;

use axum::Router;
use store::KeycloakStore;

pub const MODULE_NAME: &str = "keycloak";

/// Module state bundling the in-memory store + signer keychain.
pub struct State {
    pub store: KeycloakStore,
    pub signer: signer::SignerRegistry,
    pub brute_force: policies::BruteForceTracker,
    pub event_sink: events::EventSink,
}

impl Default for State {
    fn default() -> Self {
        Self {
            store: KeycloakStore::new(),
            signer: signer::SignerRegistry::default(),
            brute_force: policies::BruteForceTracker::default(),
            event_sink: events::EventSink::default(),
        }
    }
}

/// Axum router exposed under `/api/iam/*`.
pub fn router(state: Arc<State>) -> Router {
    routes::create_router(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_name_is_keycloak() {
        assert_eq!(MODULE_NAME, "keycloak");
    }

    #[test]
    fn default_state_is_empty() {
        let s = State::default();
        assert_eq!(s.store.realm_count(), 0);
        assert_eq!(s.store.user_count(), 0);
    }
}
