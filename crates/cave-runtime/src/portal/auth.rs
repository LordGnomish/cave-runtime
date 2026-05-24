// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Persona authentication flow for the portal.
//!
//! Browser flow:
//!  - GET  /login                    → HTML form (always public)
//!  - POST /api/auth/login           → form-encoded {username, password}
//!                                     issues JWT, sets HttpOnly cookie,
//!                                     303 redirects to /
//!  - GET  /api/auth/logout          → clears cookie, 303 redirects to /login
//!
//! Two personas in dev mode (CAVE_DEV_MODE=true):
//!   * `admin@platform`   → role `platform_admin` (sees everything, all tenants)
//!   * `admin@tenant1`    → role `tenant_admin`   (scoped to tenant `tenant1`)
//!
//! Production flow (Keycloak): `CAVE_KEYCLOAK_ISSUER` set → `/login` shows a
//! "Sign in with Keycloak" button that hits `/api/auth/oidc/start` (delegates
//! to `cave_auth::oidc`). For now the dev path is the primary surface — the
//! Keycloak realm wiring is handled by `cave-auth/keycloak/*` and is opt-in.

use axum::{
    Json, Router,
    extract::Form,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use cave_auth::jwt_middleware::JwtClaims;
use chrono::Utc;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use serde_json::json;

const SESSION_COOKIE: &str = "cave_session";
const SESSION_TTL_SECS: i64 = 8 * 60 * 60;

static LOGIN_HTML: &str = include_str!("templates/login.html");

#[derive(Debug, Deserialize)]
pub struct LoginForm {
    pub username: String,
    pub password: String,
}

/// Dev-mode user table. Production deployments must set
/// `CAVE_DEV_MODE=false` and rely on Keycloak / Okta for auth.
#[derive(Debug, Clone, Serialize)]
pub struct DevUser {
    pub username: &'static str,
    pub email: &'static str,
    pub password: &'static str,
    pub roles: &'static [&'static str],
    pub tenant: &'static str,
}

pub const DEV_USERS: &[DevUser] = &[
    DevUser {
        username: "admin@platform",
        email: "admin@platform.cave",
        password: "admin",
        roles: &["platform_admin"],
        tenant: "*",
    },
    DevUser {
        username: "admin@tenant1",
        email: "admin@tenant1.cave",
        password: "admin",
        roles: &["tenant_admin"],
        tenant: "tenant1",
    },
];

fn jwt_secret() -> String {
    std::env::var("CAVE_JWT_SECRET").expect("CAVE_JWT_SECRET must be set (use any string for dev)")
}

fn dev_mode_enabled() -> bool {
    std::env::var("CAVE_DEV_MODE")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false)
}

/// Login form handler — always public.
pub async fn login_page() -> Html<&'static str> {
    Html(LOGIN_HTML)
}

/// POST /api/auth/login — form-encoded {username, password}.
/// Looks up dev user, builds a JWT, sets cookie, redirects to `/`.
pub async fn do_login(Form(body): Form<LoginForm>) -> Response {
    if !dev_mode_enabled() {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "Dev login disabled — set CAVE_DEV_MODE=true or sign in via /api/auth/oidc/start"
            })),
        )
            .into_response();
    }

    let user = match DEV_USERS
        .iter()
        .find(|u| u.username == body.username && u.password == body.password)
    {
        Some(u) => u,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "Invalid credentials" })),
            )
                .into_response();
        }
    };

    let token = match issue_jwt_for(user) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "JWT encode failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Failed to issue session" })),
            )
                .into_response();
        }
    };

    let cookie = format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={SESSION_TTL_SECS}"
    );
    let mut resp = Redirect::to("/").into_response();
    resp.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).expect("cookie ASCII"),
    );
    resp
}

/// GET /api/auth/logout — clear cookie + redirect /login.
pub async fn do_logout() -> Response {
    let expire = format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0");
    let mut resp = Redirect::to("/login").into_response();
    resp.headers_mut()
        .insert(header::SET_COOKIE, HeaderValue::from_str(&expire).unwrap());
    resp
}

/// GET /api/auth/whoami — friendly current-user JSON.
///
/// `/api/auth/*` is bypassed by the JWT middleware (so login/logout work
/// before a session exists), which means the middleware doesn't pre-decode
/// the cookie for us. We read the cookie ourselves and only consult
/// `JwtClaims` extension as a fallback.
pub async fn whoami(
    headers: HeaderMap,
    claims: Option<axum::Extension<JwtClaims>>,
) -> Json<serde_json::Value> {
    if let Some(axum::Extension(c)) = claims {
        return Json(json!({
            "authenticated": true,
            "sub": c.sub,
            "email": c.email,
            "roles": c.roles,
            "exp": c.exp,
        }));
    }
    if let Some(token) = read_session_token(&headers) {
        if let Some(c) = decode_session(&token) {
            return Json(json!({
                "authenticated": true,
                "sub": c.sub,
                "email": c.email,
                "roles": c.roles,
                "exp": c.exp,
            }));
        }
    }
    Json(json!({ "authenticated": false }))
}

fn read_session_token(headers: &HeaderMap) -> Option<String> {
    if let Some(auth) = headers.get(header::AUTHORIZATION) {
        if let Ok(s) = auth.to_str() {
            if let Some(t) = s.strip_prefix("Bearer ") {
                return Some(t.to_string());
            }
        }
    }
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
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

fn decode_session(token: &str) -> Option<JwtClaims> {
    let secret = std::env::var("CAVE_JWT_SECRET").ok()?;
    let key = DecodingKey::from_secret(secret.as_bytes());
    let mut v = Validation::new(Algorithm::HS256);
    v.required_spec_claims.clear();
    decode::<JwtClaims>(token, &key, &v).ok().map(|t| t.claims)
}

fn issue_jwt_for(user: &DevUser) -> Result<String, jsonwebtoken::errors::Error> {
    let secret = jwt_secret();
    let exp = (Utc::now().timestamp() + SESSION_TTL_SECS) as usize;
    let claims = JwtClaims {
        sub: user.email.to_string(),
        email: user.email.to_string(),
        roles: user.roles.iter().map(|r| r.to_string()).collect(),
        exp,
    };
    let key = EncodingKey::from_secret(secret.as_bytes());
    encode(&Header::new(Algorithm::HS256), &claims, &key)
}

pub fn router() -> Router {
    Router::new()
        .route("/login", get(login_page))
        .route("/api/auth/login", post(do_login))
        .route("/api/auth/logout", get(do_logout))
        .route("/api/auth/whoami", get(whoami))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::{Method, Request};
    use http_body_util::BodyExt;
    use jsonwebtoken::{DecodingKey, Validation, decode};
    use std::sync::{Mutex, MutexGuard, OnceLock};
    use tower::ServiceExt;

    /// All tests in this module mutate process-wide CAVE_DEV_MODE and need to
    /// observe their own value across an async oneshot. cargo test's
    /// default multi-thread runner races those writes, so each test takes
    /// this lock for its whole body. Held across `.await` because the env
    /// var is read inside the handler, not just at setup time.
    fn env_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    fn set_env() {
        // SAFETY: callers hold env_lock() for the rest of the test, so no
        // other test in this module reads/writes CAVE_DEV_MODE concurrently.
        unsafe {
            std::env::set_var("CAVE_JWT_SECRET", "test-secret");
            std::env::set_var("CAVE_DEV_MODE", "true");
        }
    }

    #[tokio::test]
    async fn login_page_is_public_html() {
        let _g = env_lock();
        set_env();
        let app = router();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/login")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(body.contains("CAVE"));
        assert!(body.contains("Sign in"));
        assert!(body.contains("admin@platform"));
    }

    #[tokio::test]
    async fn login_sets_cookie_and_jwt_decodes() {
        let _g = env_lock();
        set_env();
        let app = router();
        let body = "username=admin%40platform&password=admin";
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/auth/login")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::SEE_OTHER,
            "expected 303 redirect"
        );
        let cookie = resp
            .headers()
            .get(header::SET_COOKIE)
            .expect("Set-Cookie header missing")
            .to_str()
            .unwrap();
        assert!(cookie.starts_with("cave_session="), "cookie: {cookie}");
        assert!(cookie.contains("HttpOnly"));

        let token = cookie
            .trim_start_matches("cave_session=")
            .split(';')
            .next()
            .unwrap();
        let key = DecodingKey::from_secret(b"test-secret");
        let mut v = Validation::new(Algorithm::HS256);
        v.required_spec_claims.clear();
        let data = decode::<JwtClaims>(token, &key, &v).expect("token decodes");
        assert_eq!(data.claims.email, "admin@platform.cave");
        assert!(data.claims.roles.contains(&"platform_admin".to_string()));
    }

    #[tokio::test]
    async fn login_rejects_bad_password() {
        let _g = env_lock();
        set_env();
        let app = router();
        let body = "username=admin%40platform&password=wrong";
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/auth/login")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn logout_clears_cookie() {
        let _g = env_lock();
        set_env();
        let app = router();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/auth/logout")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        let cookie = resp
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(cookie.contains("Max-Age=0"));
    }

    #[tokio::test]
    async fn login_disabled_when_not_dev_mode() {
        let _g = env_lock();
        // SAFETY: env_lock() above prevents sibling tests in this module
        // from observing or mutating CAVE_DEV_MODE while this test runs.
        unsafe {
            std::env::set_var("CAVE_JWT_SECRET", "test-secret");
            std::env::set_var("CAVE_DEV_MODE", "false");
        }
        let app = router();
        let body = "username=admin%40platform&password=admin";
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/auth/login")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let txt = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(txt.contains("Dev login disabled"));
    }

    #[tokio::test]
    async fn whoami_unauthenticated() {
        let app = router();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/auth/whoami")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["authenticated"], serde_json::Value::Bool(false));
    }

    #[tokio::test]
    async fn whoami_decodes_session_cookie_directly() {
        let _g = env_lock();
        set_env();
        let app = router();
        // Issue a real JWT for admin@platform via the dev path.
        let token = issue_jwt_for(&DEV_USERS[0]).expect("issue token");
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/auth/whoami")
                    .header(header::COOKIE, format!("cave_session={token}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["authenticated"], serde_json::Value::Bool(true));
        assert_eq!(v["email"], "admin@platform.cave");
    }
}
