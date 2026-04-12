//! CAVE Auth — Full Okta AuthN+AuthZ.
//!
//! Provides OIDC authentication flow (authorization code + PKCE), token management,
//! RBAC, ABAC, SCIM 2.0 provisioning, Personal Access Tokens, session management,
//! multi-tenancy, Tower middleware, and audit logging.

pub mod abac;
pub mod audit;
pub mod claims;
pub mod jwks;
pub mod middleware;
pub mod oidc;
pub mod pat;
pub mod rbac;
pub mod scim;
pub mod session;
pub mod tenant;
pub mod token;

use cave_core::config::AuthProvider;

pub use middleware::CaveAuthLayer;

/// Determine if we're running with Okta (Azure) or Keycloak (Hetzner).
pub fn provider_name(provider: &AuthProvider) -> &'static str {
    match provider {
        AuthProvider::Okta => "Okta",
        AuthProvider::Keycloak => "Keycloak",
    }
}
