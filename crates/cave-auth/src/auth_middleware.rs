//! AuthLayer — full enterprise auth middleware for axum.
//!
//! ## Tower Layer architecture
//!
//! ```text
//! Router
//!   └─ AuthLayer (Tower Layer)
//!        └─ AuthService<S> (Tower Service)
//!             ├─ process_auth()
//!             │    ├─ validate_jwt()   — JWKS-backed RS256 JWT validation
//!             │    └─ validate_pat()   — SHA-256 hashed PAT lookup
//!             ├─ injects AuthContext into request extensions
//!             └─ inner.call(req)       — downstream handler
//! ```
//!
//! ## Usage in route handlers
//!
//! ```rust,no_run
//! use cave_auth::auth_middleware::{AuthCtx, AuthContext};
//!
//! async fn my_handler(AuthCtx(ctx): AuthCtx) -> impl axum::response::IntoResponse {
//!     // require_permission!(ctx, "cave-flags:write");
//!     axum::Json(serde_json::json!({ "ok": true }))
//! }
//! ```
//!
//! ## Dev bypass
//!
//! Set `CAVE_AUTH_DISABLED=true` to inject a platform-admin `AuthContext`
//! without JWT validation.  Never use in production.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use cave_core::types::{CaveRole, TokenType};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde_json::json;
use tower::{Layer, Service};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::{
    audit::{AuditEvent, AuditLogger},
    claims::RawClaims,
    jwks::JwksCache,
    tokens::{PatClaims, TokenStore},
};

// ─── AuthContext ──────────────────────────────────────────────────────────────

/// Rich auth context injected into every authenticated request's extensions.
/// Prefer `AuthCtx` extractor in handlers over `Extension<AuthContext>`.
#[derive(Debug, Clone)]
pub struct AuthContext {
    /// Platform-stable user UUID
    pub cave_uid: Uuid,
    /// User email from IdP
    pub email: Option<String>,
    /// Coarse roles (from Okta groups / JWT claims)
    pub roles: Vec<CaveRole>,
    /// Resolved fine-grained permissions, e.g. "cave-flags:write"
    pub permissions: Vec<String>,
    /// Okta group memberships
    pub groups: Vec<String>,
    /// Raw Okta JWT claims serialised as JSON (for debugging / policy engines)
    pub okta_claims: serde_json::Value,
    /// How this request was authenticated
    pub token_type: TokenType,
}

impl AuthContext {
    /// Check whether this context grants a specific permission string.
    ///
    /// Permission format: `"<module>:<action>"`, e.g. `"cave-flags:write"`.
    /// Wildcards: `"*"` or `"<module>:*"`.
    pub fn has_permission(&self, perm: &str) -> bool {
        if self.roles.contains(&CaveRole::PlatformAdmin) {
            return true;
        }
        if self.permissions.contains(&"*".to_string()) {
            return true;
        }
        if self.permissions.contains(&perm.to_string()) {
            return true;
        }
        let module = perm.split(':').next().unwrap_or("");
        self.permissions.contains(&format!("{module}:*"))
    }

    /// Create a development bypass context (CAVE_AUTH_DISABLED=true).
    pub fn dev() -> Self {
        Self {
            cave_uid: Uuid::nil(),
            email: Some("dev@cave.local".to_string()),
            roles: vec![CaveRole::PlatformAdmin],
            permissions: vec!["*".to_string()],
            groups: vec!["platform-admin".to_string()],
            okta_claims: json!({}),
            token_type: TokenType::Jwt,
        }
    }
}

// ─── AuthLayer config ─────────────────────────────────────────────────────────

/// Configuration for building an `AuthLayer`.
pub struct AuthLayerConfig {
    pub jwks_uri: String,
    pub audience: String,
    pub issuer: String,
}

// ─── Internal shared state ────────────────────────────────────────────────────

#[derive(Clone)]
struct AuthState {
    jwks_cache: Arc<JwksCache>,
    audience: String,
    issuer: String,
    token_store: Arc<TokenStore>,
    audit: Arc<AuditLogger>,
    dev_mode: bool,
}

// ─── AuthLayer (Tower Layer) ──────────────────────────────────────────────────

/// Tower `Layer` that enforces authentication on every request.
///
/// Add to your axum router with:
/// ```rust,no_run
/// # let app: axum::Router = axum::Router::new();
/// # let auth_layer = cave_auth::auth_middleware::AuthLayer::dev_bypass();
/// app.layer(auth_layer);
/// ```
#[derive(Clone)]
pub struct AuthLayer {
    state: Arc<AuthState>,
}

impl AuthLayer {
    /// Build an `AuthLayer` from configuration.
    pub fn new(config: AuthLayerConfig) -> Self {
        let jwks_cache = Arc::new(JwksCache::new(config.jwks_uri));
        jwks_cache.clone().start_background_refresh();
        let token_store = Arc::new(TokenStore::new(b"change-me-in-production"));
        let audit = Arc::new(AuditLogger::new());

        Self {
            state: Arc::new(AuthState {
                jwks_cache,
                audience: config.audience,
                issuer: config.issuer,
                token_store,
                audit,
                dev_mode: false,
            }),
        }
    }

    /// Build a dev-bypass layer for local development (`CAVE_AUTH_DISABLED=true`).
    /// Injects a platform-admin `AuthContext` without any JWT validation.
    pub fn dev_bypass() -> Self {
        let jwks_cache = Arc::new(JwksCache::new("http://localhost/jwks".to_string()));
        let token_store = Arc::new(TokenStore::new(b"dev-secret"));
        let audit = Arc::new(AuditLogger::new());

        Self {
            state: Arc::new(AuthState {
                jwks_cache,
                audience: "dev".to_string(),
                issuer: "dev".to_string(),
                token_store,
                audit,
                dev_mode: true,
            }),
        }
    }

    /// Provide a custom `TokenStore` (e.g. pre-seeded with service tokens in tests).
    pub fn with_token_store(mut self, store: Arc<TokenStore>) -> Self {
        Arc::make_mut(&mut self.state).token_store = store;
        self
    }
}

impl<S> Layer<S> for AuthLayer {
    type Service = AuthService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthService {
            inner,
            state: self.state.clone(),
        }
    }
}

// ─── AuthService (Tower Service) ─────────────────────────────────────────────

/// The per-request Tower service produced by `AuthLayer`.
#[derive(Clone)]
pub struct AuthService<S> {
    inner: S,
    state: Arc<AuthState>,
}

impl<S> Service<axum::extract::Request> for AuthService<S>
where
    S: Service<axum::extract::Request, Response = Response>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
{
    type Response = Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Response, S::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), S::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: axum::extract::Request) -> Self::Future {
        // Tower clone-trick: replace inner with a fresh clone and take the
        // poll_ready'd original into the async block.
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        let state = self.state.clone();

        // Extract the Bearer token SYNCHRONOUSLY before entering the async block.
        // This avoids capturing `&Request<Body>` (which is !Sync) across an await.
        let token = extract_bearer_token(&req);

        Box::pin(async move {
            // ── Dev bypass ────────────────────────────────────────────────
            if state.dev_mode {
                let mut req = req;
                req.extensions_mut().insert(AuthContext::dev());
                return inner.call(req).await;
            }

            // ── Normal auth ───────────────────────────────────────────────
            match process_auth(&state, token).await {
                Ok(ctx) => {
                    let mut req = req;
                    req.extensions_mut().insert(ctx);
                    inner.call(req).await
                }
                Err(rejection) => Ok(rejection),
            }
        })
    }
}

// ─── Auth logic ───────────────────────────────────────────────────────────────

/// `token` is extracted before the async boundary so we never hold `&Request`
/// across an `.await` (Body is !Sync → &Request is !Send).
async fn process_auth(
    state: &AuthState,
    token: Option<String>,
) -> Result<AuthContext, Response> {
    let token = token.ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "Missing Authorization header", "hint": "Provide: Bearer <token>" })),
        )
            .into_response()
    })?;

    if token.starts_with("cave_pat_") {
        return validate_pat(state, &token).await;
    }

    if token.starts_with("cave_svc_") {
        return validate_service_token(state, &token).await;
    }

    validate_jwt(state, &token).await
}

async fn validate_jwt(state: &AuthState, token: &str) -> Result<AuthContext, Response> {
    let jwks = state
        .jwks_cache
        .get_keys()
        .await
        .map_err(|e| {
            warn!(error = %e, "JWKS unavailable");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "Authentication service unavailable" })),
            )
                .into_response()
        })?;

    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&[&state.audience]);
    validation.set_issuer(&[&state.issuer]);

    // Try all cached keys (handles in-flight key rotations)
    let ctx = try_decode_jwt(token, &jwks.keys, &validation);
    if let Some(ctx) = ctx {
        debug!(cave_uid = %ctx.cave_uid, "JWT authenticated");
        state.audit.log(AuditEvent::auth_success(ctx.cave_uid, "jwt_validate"));
        return Ok(ctx);
    }

    // Force-refresh JWKS and retry once
    if let Ok(fresh) = state.jwks_cache.refresh().await {
        if let Some(ctx) = try_decode_jwt(token, &fresh.keys, &validation) {
            debug!(cave_uid = %ctx.cave_uid, "JWT authenticated after JWKS refresh");
            state.audit.log(AuditEvent::auth_success(ctx.cave_uid, "jwt_validate"));
            return Ok(ctx);
        }
    }

    state.audit.log(AuditEvent::auth_failure("jwt_validate", "invalid_token"));
    Err((
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": "Invalid or expired token" })),
    )
        .into_response())
}

fn try_decode_jwt(
    token: &str,
    keys: &[jsonwebtoken::jwk::Jwk],
    validation: &Validation,
) -> Option<AuthContext> {
    for key in keys {
        let Ok(decoding_key) = DecodingKey::from_jwk(key) else {
            continue;
        };
        let Ok(token_data) = decode::<RawClaims>(token, &decoding_key, validation) else {
            continue;
        };
        let Ok(identity) = token_data.claims.to_identity() else {
            continue;
        };
        let okta_claims = serde_json::to_value(&token_data.claims).unwrap_or(json!({}));
        return Some(AuthContext {
            cave_uid: identity.cave_uid,
            email: identity.email,
            roles: identity.roles,
            permissions: identity.permissions,
            groups: identity.groups,
            okta_claims,
            token_type: TokenType::Jwt,
        });
    }
    None
}

async fn validate_pat(state: &AuthState, token: &str) -> Result<AuthContext, Response> {
    match state.token_store.validate_pat(token).await {
        Some(PatClaims {
            cave_uid,
            roles,
            scopes,
            ..
        }) => {
            state
                .audit
                .log(AuditEvent::auth_success(cave_uid, "pat_validate"));
            Ok(AuthContext {
                cave_uid,
                email: None,
                roles,
                permissions: scopes,
                groups: vec![],
                okta_claims: json!({}),
                token_type: TokenType::PersonalAccessToken,
            })
        }
        None => {
            state.audit.log(AuditEvent::auth_failure(
                "pat_validate",
                "invalid_or_expired_pat",
            ));
            Err((
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "Invalid or expired personal access token" })),
            )
                .into_response())
        }
    }
}

async fn validate_service_token(state: &AuthState, token: &str) -> Result<AuthContext, Response> {
    match state.token_store.validate_service_token(token).await {
        Some(claims) => {
            // Service tokens run as a synthetic "service" identity with no cave_uid
            let synthetic_uid = Uuid::nil();
            state.audit.log(
                AuditEvent::auth_success(synthetic_uid, "svc_token_validate")
                    .with_details(json!({ "service": claims.service_name })),
            );
            Ok(AuthContext {
                cave_uid: synthetic_uid,
                email: None,
                roles: vec![CaveRole::Developer],
                permissions: claims.scopes,
                groups: vec![],
                okta_claims: json!({ "service_name": claims.service_name }),
                token_type: TokenType::ServiceToken,
            })
        }
        None => {
            state.audit.log(AuditEvent::auth_failure(
                "svc_token_validate",
                "invalid_or_expired",
            ));
            Err((
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "Invalid or expired service token" })),
            )
                .into_response())
        }
    }
}

fn extract_bearer_token(req: &axum::extract::Request) -> Option<String> {
    req.headers()
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::to_string)
}

// ─── Axum extractor ───────────────────────────────────────────────────────────

/// Axum extractor for `AuthContext`.
///
/// ```rust
/// async fn handler(AuthCtx(ctx): AuthCtx) -> impl IntoResponse {
///     require_permission!(ctx, "cave-flags:write");
///     // ...
/// }
/// ```
pub struct AuthCtx(pub AuthContext);

impl<S> FromRequestParts<S> for AuthCtx
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, Json<serde_json::Value>);

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthContext>()
            .cloned()
            .map(AuthCtx)
            .ok_or_else(|| {
                (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({ "error": "Not authenticated — AuthLayer not applied?" })),
                )
            })
    }
}

// ─── require_permission! macro ────────────────────────────────────────────────

/// Guard a handler body with a required permission.
///
/// Returns HTTP 403 immediately if the `AuthContext` does not grant the
/// permission.  The handler function must return `impl IntoResponse`.
///
/// ```rust
/// async fn create_flag(AuthCtx(ctx): AuthCtx) -> impl IntoResponse {
///     require_permission!(ctx, "cave-flags:write");
///     // only reached if ctx has the permission
///     StatusCode::CREATED
/// }
/// ```
#[macro_export]
macro_rules! require_permission {
    ($ctx:expr, $perm:literal) => {
        if !$ctx.has_permission($perm) {
            return (
                axum::http::StatusCode::FORBIDDEN,
                axum::Json(serde_json::json!({
                    "error": "Insufficient permissions",
                    "required": $perm,
                })),
            )
                .into_response();
        }
    };
}
