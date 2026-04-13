//! Enterprise-pluggable authentication backend trait.
//!
//! The CAVE Runtime ships with a built-in OIDC/JWKS implementation that is
//! fully sovereign (no external vendor dependency). Enterprises can replace it
//! with their own identity provider by implementing [`AuthBackend`] and
//! selecting it via config.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                       AuthBackend (trait)                       │
//! ├─────────────────────┬──────────────┬──────────┬────────────────┤
//! │  BuiltinAuthBackend │  OktaAdapter │  EntraId │  Auth0Adapter  │
//! │  (OIDC/JWKS built-in│  (external)  │ (external│  (external)    │
//! │   — Keycloak compat)│              │  MSFT)   │                │
//! └─────────────────────┴──────────────┴──────────┴────────────────┘
//!         ▲
//!   selected by BackendProfile::from_config(...)
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ─── Error type ────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum AuthBackendError {
    #[error("invalid token: {0}")]
    InvalidToken(String),
    #[error("token expired")]
    Expired,
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    #[error("provider unreachable: {0}")]
    ProviderError(String),
    #[error("configuration error: {0}")]
    ConfigError(String),
}

pub type AuthBackendResult<T> = Result<T, AuthBackendError>;

// ─── Verified identity ─────────────────────────────────────────────────────

/// Canonical identity returned by every auth backend after token validation.
///
/// All backends must populate at minimum `subject`. Other fields are
/// best-effort — the backend fills what the IdP provides.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifiedIdentity {
    /// IdP-specific stable user identifier (sub claim).
    pub subject: String,
    /// Platform-stable UUID (cave_uid claim, if present).
    pub cave_uid: Option<String>,
    /// Email address.
    pub email: Option<String>,
    /// Role list (e.g. Keycloak realm roles, Okta groups mapped to roles).
    pub roles: Vec<String>,
    /// Raw group list from the IdP.
    pub groups: Vec<String>,
    /// Tenant/organisation identifier, if multi-tenant.
    pub tenant_id: Option<String>,
    /// Token expiry as Unix timestamp (seconds).
    pub exp: i64,
}

// ─── AuthBackend trait ─────────────────────────────────────────────────────

/// Enterprise-pluggable authentication backend.
///
/// Implement this trait to integrate any identity provider with the CAVE
/// Runtime. The factory function [`crate::factory::create_auth_backend`]
/// selects the concrete implementation at startup based on config.
#[async_trait]
pub trait AuthBackend: Send + Sync + 'static {
    /// Validate a bearer token (JWT or opaque) and return the verified
    /// identity. Called on every authenticated request — must be fast.
    async fn validate_token(&self, token: &str) -> AuthBackendResult<VerifiedIdentity>;

    /// Return the current group memberships for `subject`. May return cached
    /// data. Called lazily for RBAC lookups that need fresh group state.
    async fn get_groups(&self, subject: &str) -> AuthBackendResult<Vec<String>>;

    /// Introspect an opaque token via the IdP's token introspection endpoint.
    /// Return `None` if the backend does not support introspection; in that
    /// case `validate_token` (JWKS) is the only path.
    async fn introspect(&self, token: &str) -> AuthBackendResult<Option<VerifiedIdentity>>;

    /// Human-readable backend name — used in logs and `/ready` output.
    fn name(&self) -> &'static str;
}

// ─── Profile config ────────────────────────────────────────────────────────

/// Selects which auth backend the factory should instantiate.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthBackendProfile {
    /// Built-in sovereign OIDC/JWKS implementation (default).
    /// Works with Keycloak, Dex, CAVE's own token server.
    #[default]
    Builtin,
    /// External Okta identity cloud via Okta API.
    Okta,
    /// Microsoft Entra ID (formerly Azure Active Directory).
    EntraId,
    /// Auth0 by Okta.
    Auth0,
}
