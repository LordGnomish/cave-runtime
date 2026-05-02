//! JWT authentication middleware with RBAC support for CAVE runtime.

use axum::{
    extract::{FromRequestParts, Request},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tracing::{debug, warn};

/// JWT claims extracted from token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtClaims {
    pub sub: String,
    pub email: String,
    #[serde(default)]
    pub roles: Vec<String>,
    pub exp: usize,
}

/// Auth middleware configuration.
#[derive(Clone)]
pub struct AuthState {
    pub jwt_secret: String,
    pub bypass_paths: Vec<String>,
}

impl AuthState {
    pub fn should_bypass(&self, path: &str) -> bool {
        self.bypass_paths.iter().any(|bp| {
            if let Some(exact) = bp.strip_prefix("_exact:") {
                // Exact match only (for "/" which would otherwise prefix-match everything)
                path == exact
            } else if bp.ends_with('/') && bp.len() > 1 {
                // Prefix match (e.g. "/portal/" matches "/portal/tracker")
                path == bp.trim_end_matches('/') || path.starts_with(bp)
            } else {
                // Exact match
                path == bp
            }
        })
    }
}

fn extract_bearer_token(req: &Request) -> Option<String> {
    req.headers()
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(|s| s.to_string())
}

/// Extract a JWT from the `cave_session` cookie (portal browser flow).
/// Falls back when no `Authorization: Bearer …` header is present.
fn extract_session_cookie(req: &Request) -> Option<String> {
    let raw = req.headers().get(header::COOKIE)?.to_str().ok()?;
    for kv in raw.split(';') {
        let kv = kv.trim();
        if let Some(value) = kv.strip_prefix("cave_session=") {
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Create an auth middleware closure that captures the given AuthState.
/// Use with `axum::middleware::from_fn(make_auth_middleware(state))`.
pub fn make_auth_middleware(
    state: Arc<AuthState>,
) -> impl Fn(Request, Next) -> std::pin::Pin<Box<dyn std::future::Future<Output = Response> + Send>> + Clone + Send {
    move |req, next| {
        let state = state.clone();
        Box::pin(auth_middleware_inner(state, req, next))
    }
}

/// Inner auth middleware logic.
pub async fn auth_middleware_inner(
    state: Arc<AuthState>,
    mut req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path().to_string();

    if state.should_bypass(&path) {
        debug!(path = %path, "Bypassing auth");
        return next.run(req).await;
    }

    let token = match extract_bearer_token(&req).or_else(|| extract_session_cookie(&req)) {
        Some(t) => t,
        None => {
            // Browser navigations (Accept: text/html) get redirected to /login;
            // API clients get a JSON 401 so they can refresh tokens cleanly.
            let wants_html = req
                .headers()
                .get(header::ACCEPT)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.contains("text/html"))
                .unwrap_or(false);
            if wants_html {
                return axum::response::Redirect::to("/login").into_response();
            }
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "Missing or invalid Authorization header / session cookie" })),
            )
                .into_response();
        }
    };

    let key = DecodingKey::from_secret(state.jwt_secret.as_bytes());
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    // Don't require specific claims beyond what we decode
    validation.required_spec_claims.clear();

    match decode::<JwtClaims>(&token, &key, &validation) {
        Ok(token_data) => {
            let claims = token_data.claims;
            debug!(sub = %claims.sub, email = %claims.email, roles = ?claims.roles, "Token validated");
            req.extensions_mut().insert(claims);
            next.run(req).await
        }
        Err(e) => {
            let msg = format!("Authentication failed: {}", e);
            warn!(error = %e, "Token validation failed");
            (StatusCode::UNAUTHORIZED, Json(json!({ "error": msg }))).into_response()
        }
    }
}

/// Extractor: get current user's claims from request (injected by middleware).
impl<S: Send + Sync> FromRequestParts<S> for JwtClaims {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<JwtClaims>()
            .cloned()
            .ok_or_else(|| {
                (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({ "error": "Not authenticated" })),
                )
                    .into_response()
            })
    }
}

/// Check if current user has a specific role. Use in handlers:
/// ```ignore
/// async fn admin_only(claims: JwtClaims) -> impl IntoResponse {
///     claims.require_role("admin")?;
///     // ...
/// }
/// ```
impl JwtClaims {
    pub fn require_role(&self, role: &str) -> Result<(), Response> {
        if self.roles.iter().any(|r| r == role) {
            Ok(())
        } else {
            Err((
                StatusCode::FORBIDDEN,
                Json(json!({ "error": format!("Missing required role: {}", role) })),
            )
                .into_response())
        }
    }

    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bypass_exact_match() {
        let state = AuthState {
            jwt_secret: "s".into(),
            bypass_paths: vec!["/health".into(), "/portal/".into()],
        };
        assert!(state.should_bypass("/health"));
        assert!(!state.should_bypass("/health/extra"));
    }

    #[test]
    fn bypass_prefix_match() {
        let state = AuthState {
            jwt_secret: "s".into(),
            bypass_paths: vec!["/portal/".into()],
        };
        assert!(state.should_bypass("/portal/tracker"));
        assert!(state.should_bypass("/portal/"));
        assert!(state.should_bypass("/portal")); // trim trailing /
        assert!(!state.should_bypass("/api/tracker"));
    }

    #[test]
    fn valid_token_decodes() {
        use jsonwebtoken::{encode, EncodingKey, Header};
        let secret = "test-secret";
        let claims = JwtClaims {
            sub: "user1".into(),
            email: "user@test.com".into(),
            roles: vec!["admin".into(), "dev".into()],
            exp: (chrono::Utc::now().timestamp() + 3600) as usize,
        };
        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap();

        let key = DecodingKey::from_secret(secret.as_bytes());
        let mut v = Validation::new(Algorithm::HS256);
        v.required_spec_claims.clear();
        let decoded = decode::<JwtClaims>(&token, &key, &v).unwrap();
        assert_eq!(decoded.claims.sub, "user1");
        assert_eq!(decoded.claims.roles, vec!["admin", "dev"]);
    }

    #[test]
    fn expired_token_rejected() {
        use jsonwebtoken::{encode, EncodingKey, Header};
        let secret = "test-secret";
        let claims = JwtClaims {
            sub: "user1".into(),
            email: "u@t.com".into(),
            roles: vec![],
            exp: 1000, // way in the past
        };
        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap();

        let key = DecodingKey::from_secret(secret.as_bytes());
        let mut v = Validation::new(Algorithm::HS256);
        v.required_spec_claims.clear();
        let result = decode::<JwtClaims>(&token, &key, &v);
        assert!(result.is_err());
    }

    #[test]
    fn wrong_secret_rejected() {
        use jsonwebtoken::{encode, EncodingKey, Header};
        let claims = JwtClaims {
            sub: "u".into(),
            email: "e".into(),
            roles: vec![],
            exp: (chrono::Utc::now().timestamp() + 3600) as usize,
        };
        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(b"secret-a"),
        )
        .unwrap();

        let key = DecodingKey::from_secret(b"secret-b");
        let mut v = Validation::new(Algorithm::HS256);
        v.required_spec_claims.clear();
        assert!(decode::<JwtClaims>(&token, &key, &v).is_err());
    }

    #[test]
    fn cookie_extracted() {
        let req = Request::builder()
            .uri("/x")
            .header(header::COOKIE, "other=foo; cave_session=abc.def.ghi; tail=zz")
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(
            extract_session_cookie(&req).as_deref(),
            Some("abc.def.ghi")
        );
    }

    #[test]
    fn cookie_absent_returns_none() {
        let req = Request::builder()
            .uri("/x")
            .header(header::COOKIE, "other=foo")
            .body(axum::body::Body::empty())
            .unwrap();
        assert!(extract_session_cookie(&req).is_none());
    }

    #[test]
    fn role_check() {
        let claims = JwtClaims {
            sub: "u".into(),
            email: "e".into(),
            roles: vec!["admin".into(), "viewer".into()],
            exp: 0,
        };
        assert!(claims.has_role("admin"));
        assert!(!claims.has_role("superadmin"));
        assert!(claims.require_role("admin").is_ok());
        assert!(claims.require_role("superadmin").is_err());
    }
}
