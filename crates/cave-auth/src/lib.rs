//! CAVE Auth — Native Okta/Keycloak OIDC authentication.
//!
//! Provides JWT validation, JWKS key rotation, cave_uid extraction,
//! and axum middleware for all runtime modules.

pub mod jwks;
pub mod middleware;
pub mod claims;

use cave_core::config::AuthProvider;

/// Re-export the middleware layer for easy use in axum routers.
pub use middleware::CaveAuthLayer;

/// Determine if we're running with Okta (Azure) or Keycloak (Hetzner).
pub fn provider_name(provider: &AuthProvider) -> &'static str {
    match provider {
        AuthProvider::Okta => "Okta",
        AuthProvider::Keycloak => "Keycloak",
    }
}
