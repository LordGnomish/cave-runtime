//! CAVE Auth — full enterprise authentication & authorization layer.
//!
//! ## What's included
//!
//! | Module | Responsibility |
//! |--------|---------------|
//! | [`jwks`] | JWKS fetching, caching, background rotation |
//! | [`claims`] | JWT claims → `CaveIdentity` mapping (Okta + Keycloak) |
//! | [`middleware`] | Original `CaveAuthLayer` (simple AuthN only, kept for compat) |
//! | [`auth_middleware`] | Full `AuthLayer` (Tower Layer): JWT + PAT + service tokens, `AuthContext`, `AuthCtx` extractor, `require_permission!` macro |
//! | [`rbac`] | Role-Based Access Control: `Role`, `RoleBinding`, `ResourcePolicy`, `RbacEngine` |
//! | [`abac`] | Attribute-Based Access Control: policy engine, OPA-compatible policy format |
//! | [`tokens`] | PAT + service-to-service token management |
//! | [`audit`] | Structured audit logging of every auth decision |
//! | [`okta`] | Okta Authorization Server client, group sync, SCIM 2.0 handlers |

pub mod abac;
pub mod audit;
pub mod auth_middleware;
pub mod auth_routes;
pub mod claims;
pub mod jwks;
pub mod jwt_middleware;
pub mod keycloak;
pub mod middleware;
pub mod okta;
pub mod rbac;
pub mod saml;
pub mod tokens;

use cave_core::config::AuthProvider;

// ── Convenient re-exports for common usage ────────────────────────────────────

/// The full Tower `AuthLayer` — use this in production routers.
pub use auth_middleware::{AuthContext, AuthCtx, AuthLayer, AuthLayerConfig};

/// Original lightweight AuthLayer (AuthN only, no RBAC/ABAC).
pub use middleware::CaveAuthLayer;

// Additional auth modules
pub mod oidc;
pub mod pat;
pub mod scim;
pub mod session;
pub mod tenant;
pub mod token;

/// RBAC engine + models.
pub use rbac::{BindingScope, RbacEngine, ResourcePolicy, Role, RoleBinding};

/// ABAC policy engine + types.
pub use abac::{AbacPolicyEngine, AbacPolicy, PolicyDecision};

/// Token management.
pub use tokens::TokenStore;

/// Audit logger.
pub use audit::{AuditEvent, AuditLogger};

/// Okta client + SCIM router.
pub use okta::{OktaAuthServer, OktaAuthServerConfig};

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Determine the human-readable IdP name from config.
pub fn provider_name(provider: &AuthProvider) -> &'static str {
    match provider {
        AuthProvider::Okta => "Okta",
        AuthProvider::Keycloak => "Keycloak",
    }
}

/// Build an `AuthLayer` from environment variables — useful in tests and
/// when the full YAML config is not available.
///
/// Environment variables read:
/// - `OKTA_DOMAIN` — e.g. `https://company.okta.com`
/// - `OKTA_AUTH_SERVER_ID` — defaults to `"default"`
/// - `OKTA_AUDIENCE` — OAuth 2.0 audience
pub fn auth_layer_from_env() -> AuthLayer {
    let disabled = std::env::var("CAVE_AUTH_DISABLED")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    if disabled {
        tracing::warn!("CAVE_AUTH_DISABLED=true — all requests run as platform-admin. NEVER use in production.");
        return AuthLayer::dev_bypass();
    }

    let domain = std::env::var("OKTA_DOMAIN").unwrap_or_default();
    let server_id = std::env::var("OKTA_AUTH_SERVER_ID").unwrap_or_else(|_| "default".to_string());
    let audience = std::env::var("OKTA_AUDIENCE").unwrap_or_default();

    AuthLayer::new(AuthLayerConfig {
        jwks_uri: format!("{domain}/oauth2/{server_id}/v1/keys"),
        audience,
        issuer: format!("{domain}/oauth2/{server_id}"),
    })
}
