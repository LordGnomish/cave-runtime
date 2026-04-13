//! Factory function for selecting the auth backend from config.
//!
//! At startup the CAVE Runtime calls [`create_auth_backend`] with the
//! resolved config. The returned `Arc<dyn AuthBackend>` is stored in shared
//! state and called on every authenticated request.

use std::sync::Arc;

use crate::adapters::{auth0::Auth0Adapter, entra::EntraIdAdapter, okta_adapter::OktaAdapter};
use crate::builtin::{BuiltinAuthBackend, BuiltinAuthConfig};
use crate::provider::{AuthBackend, AuthBackendProfile};

/// Instantiate the correct auth backend for the given profile.
///
/// # Defaults
///
/// When `profile` is `None` (or env `CAVE_AUTH_BACKEND` is unset), the
/// built-in OIDC/JWKS backend is used with values from env:
/// - `CAVE_OIDC_JWKS_URI`
/// - `CAVE_OIDC_ISSUER`
/// - `CAVE_OIDC_AUDIENCE`
pub fn create_auth_backend(
    profile: AuthBackendProfile,
    jwks_uri: Option<String>,
    issuer: Option<String>,
    audience: Option<String>,
) -> Arc<dyn AuthBackend> {
    match profile {
        AuthBackendProfile::Builtin => {
            let config = BuiltinAuthConfig {
                jwks_uri: jwks_uri
                    .or_else(|| std::env::var("CAVE_OIDC_JWKS_URI").ok())
                    .unwrap_or_default(),
                issuer: issuer
                    .or_else(|| std::env::var("CAVE_OIDC_ISSUER").ok())
                    .unwrap_or_default(),
                audience: audience
                    .or_else(|| std::env::var("CAVE_OIDC_AUDIENCE").ok())
                    .unwrap_or_default(),
            };
            tracing::info!(backend = "builtin-oidc", jwks_uri = %config.jwks_uri, "auth backend selected");
            Arc::new(BuiltinAuthBackend::new(config))
        }

        AuthBackendProfile::Okta => {
            let config = crate::adapters::okta_adapter::OktaAdapterConfig {
                domain: std::env::var("OKTA_DOMAIN").unwrap_or_default(),
                client_id: std::env::var("OKTA_CLIENT_ID").unwrap_or_default(),
                client_secret: std::env::var("OKTA_CLIENT_SECRET").unwrap_or_default(),
                auth_server_id: std::env::var("OKTA_AUTH_SERVER_ID")
                    .unwrap_or_else(|_| "default".to_string()),
            };
            tracing::info!(backend = "okta", domain = %config.domain, "auth backend selected");
            Arc::new(OktaAdapter::new(config))
        }

        AuthBackendProfile::EntraId => {
            let config = crate::adapters::entra::EntraIdConfig {
                tenant_id: std::env::var("ENTRA_TENANT_ID").unwrap_or_default(),
                client_id: std::env::var("ENTRA_CLIENT_ID").unwrap_or_default(),
                client_secret: std::env::var("ENTRA_CLIENT_SECRET").unwrap_or_default(),
            };
            tracing::info!(backend = "entra-id", tenant_id = %config.tenant_id, "auth backend selected");
            Arc::new(EntraIdAdapter::new(config))
        }

        AuthBackendProfile::Auth0 => {
            let config = crate::adapters::auth0::Auth0Config {
                domain: std::env::var("AUTH0_DOMAIN").unwrap_or_default(),
                audience: std::env::var("AUTH0_AUDIENCE").unwrap_or_default(),
                client_id: std::env::var("AUTH0_CLIENT_ID").unwrap_or_default(),
                client_secret: std::env::var("AUTH0_CLIENT_SECRET").unwrap_or_default(),
            };
            tracing::info!(backend = "auth0", domain = %config.domain, "auth backend selected");
            Arc::new(Auth0Adapter::new(config))
        }
    }
}

/// Convenience: build backend from environment variables alone.
///
/// `CAVE_AUTH_BACKEND` = `builtin` | `okta` | `entra_id` | `auth0`
pub fn create_auth_backend_from_env() -> Arc<dyn AuthBackend> {
    let profile = match std::env::var("CAVE_AUTH_BACKEND")
        .unwrap_or_else(|_| "builtin".to_string())
        .as_str()
    {
        "okta" => AuthBackendProfile::Okta,
        "entra_id" | "entra" => AuthBackendProfile::EntraId,
        "auth0" => AuthBackendProfile::Auth0,
        _ => AuthBackendProfile::Builtin,
    };
    create_auth_backend(profile, None, None, None)
}
