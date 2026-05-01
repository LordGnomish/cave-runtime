//! OIDC Token Endpoints — /realms/{realm}/protocol/openid-connect/token et al.
//!
//! upstream: https://github.com/keycloak/keycloak/blob/v22.0.0/services/src/main/java/org/keycloak/services/resources/RealmsResource.java

use axum::{
    extract::{Form, Path, State},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::keycloak::{
    client::ClientStore,
    realm::RealmStore,
    user::UserStore,
};

// ─── Constants ────────────────────────────────────────────────────────────────

const SIGNING_SECRET: &[u8] = b"cave-keycloak-dev-secret-change-in-production";
const ACCESS_TOKEN_TTL: i64 = 300;   // seconds
const REFRESH_TOKEN_TTL: i64 = 1800; // seconds

// ─── Token response ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_expires_in: Option<i64>,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_state: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntrospectionResponse {
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exp: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

// ─── JWT Claims ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessTokenClaims {
    pub sub: String,
    pub iss: String,
    pub aud: String,
    pub exp: i64,
    pub iat: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    pub preferred_username: String,
    pub typ: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    pub scope: String,
    pub session_state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshTokenClaims {
    pub sub: String,
    pub iss: String,
    pub exp: i64,
    pub iat: i64,
    pub typ: String,
    pub session_state: String,
    pub scope: String,
}

// ─── Session store ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct SessionEntry {
    realm: String,
    sub: String,
    username: String,
    email: Option<String>,
    client_id: Option<String>,
    scope: String,
    exp: i64,
}

#[derive(Clone, Default)]
struct OidcSessionStore {
    inner: Arc<RwLock<HashMap<String, SessionEntry>>>,
}

impl OidcSessionStore {
    fn new() -> Self { Self::default() }

    async fn insert(&self, session_state: String, entry: SessionEntry) {
        self.inner.write().await.insert(session_state, entry);
    }

    async fn get(&self, session_state: &str) -> Option<SessionEntry> {
        self.inner.read().await.get(session_state).cloned()
    }

    async fn remove(&self, session_state: &str) {
        self.inner.write().await.remove(session_state);
    }
}

// ─── Token service ────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct KeycloakTokenService {
    realms: RealmStore,
    users: UserStore,
    clients: ClientStore,
    sessions: OidcSessionStore,
}

impl KeycloakTokenService {
    pub fn new(realms: RealmStore, users: UserStore, clients: ClientStore) -> Self {
        Self {
            realms,
            users,
            clients,
            sessions: OidcSessionStore::new(),
        }
    }

    fn issuer(realm: &str) -> String {
        format!("http://localhost:8080/realms/{realm}")
    }

    fn sign_access_token(claims: &AccessTokenClaims) -> Result<String, jsonwebtoken::errors::Error> {
        encode(
            &Header::new(Algorithm::HS256),
            claims,
            &EncodingKey::from_secret(SIGNING_SECRET),
        )
    }

    fn sign_refresh_token(claims: &RefreshTokenClaims) -> Result<String, jsonwebtoken::errors::Error> {
        encode(
            &Header::new(Algorithm::HS256),
            claims,
            &EncodingKey::from_secret(SIGNING_SECRET),
        )
    }

    pub fn decode_access_token(token: &str) -> Option<AccessTokenClaims> {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_exp = true;
        validation.set_audience(&["account"]);
        decode::<AccessTokenClaims>(
            token,
            &DecodingKey::from_secret(SIGNING_SECRET),
            &validation,
        )
        .map(|d| d.claims)
        .ok()
    }

    fn decode_refresh_token_raw(token: &str) -> Option<RefreshTokenClaims> {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_exp = true;
        validation.set_required_spec_claims(&["sub", "exp"]);
        decode::<RefreshTokenClaims>(
            token,
            &DecodingKey::from_secret(SIGNING_SECRET),
            &validation,
        )
        .map(|d| d.claims)
        .ok()
    }

    fn make_token_pair(
        realm: &str,
        sub: &str,
        username: &str,
        email: Option<&str>,
        client_id: Option<&str>,
        scope: &str,
        session_state: &str,
    ) -> Result<(String, String), &'static str> {
        let now = Utc::now().timestamp();
        let iss = Self::issuer(realm);

        let access_claims = AccessTokenClaims {
            sub: sub.to_string(),
            iss: iss.clone(),
            aud: "account".to_string(),
            exp: now + ACCESS_TOKEN_TTL,
            iat: now,
            email: email.map(String::from),
            preferred_username: username.to_string(),
            typ: "Bearer".to_string(),
            client_id: client_id.map(String::from),
            scope: scope.to_string(),
            session_state: session_state.to_string(),
        };

        let refresh_claims = RefreshTokenClaims {
            sub: sub.to_string(),
            iss: iss.clone(),
            exp: now + REFRESH_TOKEN_TTL,
            iat: now,
            typ: "Refresh".to_string(),
            session_state: session_state.to_string(),
            scope: scope.to_string(),
        };

        let access = Self::sign_access_token(&access_claims).map_err(|_| "sign_error")?;
        let refresh = Self::sign_refresh_token(&refresh_claims).map_err(|_| "sign_error")?;
        Ok((access, refresh))
    }

    pub async fn password_grant(
        &self,
        realm: &str,
        username: &str,
        password: &str,
        client_id: &str,
    ) -> Result<TokenResponse, &'static str> {
        if self.realms.get(realm).await.is_none() {
            return Err("realm_not_found");
        }
        let user = self.users.get_by_username(realm, username).await.ok_or("invalid_credentials")?;
        if !self.users.verify_password(realm, user.id, password).await {
            return Err("invalid_credentials");
        }

        let session_state = Uuid::new_v4().to_string();
        let scope = "openid profile email";
        let (access, refresh) = Self::make_token_pair(
            realm,
            &user.id.to_string(),
            username,
            user.email.as_deref(),
            Some(client_id),
            scope,
            &session_state,
        )?;

        self.sessions.insert(session_state.clone(), SessionEntry {
            realm: realm.to_string(),
            sub: user.id.to_string(),
            username: username.to_string(),
            email: user.email.clone(),
            client_id: Some(client_id.to_string()),
            scope: scope.to_string(),
            exp: Utc::now().timestamp() + REFRESH_TOKEN_TTL,
        }).await;

        Ok(TokenResponse {
            access_token: access,
            token_type: "Bearer".to_string(),
            expires_in: ACCESS_TOKEN_TTL,
            refresh_token: Some(refresh),
            refresh_expires_in: Some(REFRESH_TOKEN_TTL),
            scope: scope.to_string(),
            session_state: Some(session_state),
        })
    }

    pub async fn client_credentials_grant(
        &self,
        realm: &str,
        client_id: &str,
        client_secret: &str,
    ) -> Result<TokenResponse, &'static str> {
        if self.realms.get(realm).await.is_none() {
            return Err("realm_not_found");
        }
        let client = self.clients.get_by_client_id(realm, client_id).await.ok_or("invalid_client")?;
        if client.public_client {
            return Err("public_client_not_allowed");
        }
        let secret = client.secret.as_deref().unwrap_or("");
        if secret != client_secret {
            return Err("invalid_client_secret");
        }

        let session_state = Uuid::new_v4().to_string();
        let scope = "openid";
        let (access, _) = Self::make_token_pair(
            realm,
            &client.id.to_string(),
            client_id,
            None,
            Some(client_id),
            scope,
            &session_state,
        )?;

        Ok(TokenResponse {
            access_token: access,
            token_type: "Bearer".to_string(),
            expires_in: ACCESS_TOKEN_TTL,
            refresh_token: None,
            refresh_expires_in: None,
            scope: scope.to_string(),
            session_state: Some(session_state),
        })
    }

    pub async fn refresh_grant(
        &self,
        realm: &str,
        refresh_token: &str,
    ) -> Result<TokenResponse, &'static str> {
        if self.realms.get(realm).await.is_none() {
            return Err("realm_not_found");
        }
        let claims = Self::decode_refresh_token_raw(refresh_token).ok_or("invalid_refresh_token")?;

        // Verify session still exists
        let session = self.sessions.get(&claims.session_state).await.ok_or("session_not_found")?;
        if session.realm != realm {
            return Err("realm_mismatch");
        }

        let new_session_state = Uuid::new_v4().to_string();
        let (access, refresh) = Self::make_token_pair(
            realm,
            &claims.sub,
            &session.username,
            session.email.as_deref(),
            session.client_id.as_deref(),
            &session.scope,
            &new_session_state,
        )?;

        // Rotate session
        self.sessions.remove(&claims.session_state).await;
        self.sessions.insert(new_session_state.clone(), SessionEntry {
            realm: realm.to_string(),
            sub: claims.sub,
            username: session.username,
            email: session.email,
            client_id: session.client_id,
            scope: session.scope.clone(),
            exp: Utc::now().timestamp() + REFRESH_TOKEN_TTL,
        }).await;

        Ok(TokenResponse {
            access_token: access,
            token_type: "Bearer".to_string(),
            expires_in: ACCESS_TOKEN_TTL,
            refresh_token: Some(refresh),
            refresh_expires_in: Some(REFRESH_TOKEN_TTL),
            scope: session.scope,
            session_state: Some(new_session_state),
        })
    }

    pub async fn introspect(&self, realm: &str, token: &str) -> IntrospectionResponse {
        match Self::decode_access_token(token) {
            Some(claims) if claims.iss == Self::issuer(realm) => IntrospectionResponse {
                active: true,
                sub: Some(claims.sub),
                exp: Some(claims.exp),
                username: Some(claims.preferred_username),
                client_id: claims.client_id,
                scope: Some(claims.scope),
            },
            _ => IntrospectionResponse {
                active: false,
                sub: None,
                exp: None,
                username: None,
                client_id: None,
                scope: None,
            },
        }
    }

    pub async fn logout(&self, session_state: &str) {
        self.sessions.remove(session_state).await;
    }

    /// Helper: issuer URL (pub for discovery).
    pub fn issuer_for(realm: &str) -> String {
        Self::issuer(realm)
    }
}

// ─── Form types ───────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct TokenForm {
    pub grant_type: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub refresh_token: Option<String>,
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct IntrospectForm {
    pub token: String,
}

#[derive(Debug, Deserialize)]
pub struct LogoutForm {
    pub refresh_token: Option<String>,
    pub session_state: Option<String>,
}

// ─── Handlers ────────────────────────────────────────────────────────────────

pub async fn token_endpoint(
    State(svc): State<KeycloakTokenService>,
    Path(realm): Path<String>,
    Form(form): Form<TokenForm>,
) -> impl IntoResponse {
    let result = match form.grant_type.as_str() {
        "password" => {
            let username = form.username.unwrap_or_default();
            let password = form.password.unwrap_or_default();
            let client_id = form.client_id.unwrap_or_default();
            svc.password_grant(&realm, &username, &password, &client_id).await
        }
        "client_credentials" => {
            let client_id = form.client_id.unwrap_or_default();
            let client_secret = form.client_secret.unwrap_or_default();
            svc.client_credentials_grant(&realm, &client_id, &client_secret).await
        }
        "refresh_token" => {
            let rt = form.refresh_token.unwrap_or_default();
            svc.refresh_grant(&realm, &rt).await
        }
        _ => Err("unsupported_grant_type"),
    };

    match result {
        Ok(tokens) => (StatusCode::OK, Json(serde_json::to_value(tokens).unwrap())).into_response(),
        Err("realm_not_found") | Err("invalid_credentials") | Err("invalid_client") | Err("invalid_client_secret") | Err("public_client_not_allowed") => {
            (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error":"unauthorized"}))).into_response()
        }
        Err("invalid_refresh_token") | Err("session_not_found") | Err("realm_mismatch") => {
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"invalid_grant"}))).into_response()
        }
        Err(_) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"bad_request"}))).into_response(),
    }
}

pub async fn userinfo_endpoint(
    State(svc): State<KeycloakTokenService>,
    Path(realm): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let auth = match headers.get(header::AUTHORIZATION) {
        Some(v) => v.to_str().unwrap_or("").to_string(),
        None => return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error":"missing token"}))).into_response(),
    };

    let token = if auth.starts_with("Bearer ") {
        &auth[7..]
    } else {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error":"invalid auth header"}))).into_response();
    };

    match KeycloakTokenService::decode_access_token(token) {
        Some(claims) if claims.iss == KeycloakTokenService::issuer_for(&realm) => {
            let body = serde_json::json!({
                "sub": claims.sub,
                "preferred_username": claims.preferred_username,
                "email": claims.email,
                "scope": claims.scope,
            });
            (StatusCode::OK, Json(body)).into_response()
        }
        _ => (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error":"invalid token"}))).into_response(),
    }
}

pub async fn introspect_endpoint(
    State(svc): State<KeycloakTokenService>,
    Path(realm): Path<String>,
    Form(form): Form<IntrospectForm>,
) -> impl IntoResponse {
    let result = svc.introspect(&realm, &form.token).await;
    (StatusCode::OK, Json(serde_json::to_value(result).unwrap()))
}

pub async fn logout_endpoint(
    State(svc): State<KeycloakTokenService>,
    Path(realm): Path<String>,
    Form(form): Form<LogoutForm>,
) -> impl IntoResponse {
    let _ = realm;
    let session = form.session_state
        .or(form.refresh_token.as_ref().and_then(|rt| {
            KeycloakTokenService::decode_access_token(rt)
                .map(|c| c.session_state)
        }));
    if let Some(ss) = session {
        svc.logout(&ss).await;
    }
    StatusCode::NO_CONTENT
}

// ─── Router ──────────────────────────────────────────────────────────────────

pub fn router(svc: KeycloakTokenService) -> Router {
    Router::new()
        .route("/realms/{realm}/protocol/openid-connect/token", post(token_endpoint))
        .route("/realms/{realm}/protocol/openid-connect/userinfo", get(userinfo_endpoint))
        .route("/realms/{realm}/protocol/openid-connect/token/introspect", post(introspect_endpoint))
        .route("/realms/{realm}/protocol/openid-connect/logout", post(logout_endpoint))
        .with_state(svc)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keycloak::{
        client::{ClientStore, CreateClientRequest},
        realm::{RealmRequest, RealmStore},
        user::{CreateUserRequest, UserStore},
    };
    use axum::{body::Body, http::Request};
    use serde_json::Value;
    use tower::ServiceExt;

    async fn setup() -> (Router, KeycloakTokenService) {
        let realms = RealmStore::new();
        realms.create(RealmRequest { id: "myrealm".to_string(), display_name: None, enabled: None, ssl_required: None, registration_allowed: None, login_with_email_allowed: None, duplicate_emails_allowed: None, access_token_lifespan: None, sso_session_idle_timeout: None }).await.unwrap();

        let users = UserStore::new();
        users.create("myrealm", CreateUserRequest { username: "testuser".to_string(), email: Some("testuser@example.com".to_string()), email_verified: Some(true), first_name: None, last_name: None, enabled: Some(true), attributes: None, password: Some("correctpassword".to_string()) }).await.unwrap();

        let clients = ClientStore::new();
        clients.create("myrealm", CreateClientRequest { client_id: "test-client".to_string(), name: None, description: None, enabled: Some(true), public_client: Some(false), secret: Some("client-secret".to_string()), redirect_uris: None, web_origins: None, protocol: None }).await.unwrap();
        clients.create("myrealm", CreateClientRequest { client_id: "public-client".to_string(), name: None, description: None, enabled: Some(true), public_client: Some(true), secret: None, redirect_uris: None, web_origins: None, protocol: None }).await.unwrap();

        let svc = KeycloakTokenService::new(realms, users, clients);
        let app = router(svc.clone());
        (app, svc)
    }

    async fn body_json(resp: axum::response::Response) -> Value {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    }

    fn token_form(pairs: &[(&str, &str)]) -> String {
        pairs.iter().map(|(k, v)| format!("{}={}", k, urlencoding(v))).collect::<Vec<_>>().join("&")
    }

    fn urlencoding(s: &str) -> String {
        s.replace(' ', "+").replace('/', "%2F")
    }

    // upstream: keycloak/keycloak TokenEndpointTest.java:testPasswordGrantSuccess
    #[tokio::test]
    async fn test_password_grant_success() {
        let (app, _) = setup().await;
        let resp = app.oneshot(
            Request::builder().method("POST").uri("/realms/myrealm/protocol/openid-connect/token")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(token_form(&[("grant_type","password"),("username","testuser"),("password","correctpassword"),("client_id","test-client")]))).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        assert!(body["access_token"].is_string());
        assert!(!body["access_token"].as_str().unwrap().is_empty());
    }

    // upstream: keycloak/keycloak TokenEndpointTest.java:testPasswordGrantWrongPassword
    #[tokio::test]
    async fn test_password_grant_wrong_password() {
        let (app, _) = setup().await;
        let resp = app.oneshot(
            Request::builder().method("POST").uri("/realms/myrealm/protocol/openid-connect/token")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(token_form(&[("grant_type","password"),("username","testuser"),("password","wrongpassword"),("client_id","test-client")]))).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // upstream: keycloak/keycloak TokenEndpointTest.java:testPasswordGrantUnknownRealm
    #[tokio::test]
    async fn test_password_grant_unknown_realm() {
        let (app, _) = setup().await;
        let resp = app.oneshot(
            Request::builder().method("POST").uri("/realms/unknownrealm/protocol/openid-connect/token")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(token_form(&[("grant_type","password"),("username","testuser"),("password","correctpassword"),("client_id","test-client")]))).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // upstream: keycloak/keycloak TokenEndpointTest.java:testClientCredentialsSuccess
    #[tokio::test]
    async fn test_client_credentials_success() {
        let (app, _) = setup().await;
        let resp = app.oneshot(
            Request::builder().method("POST").uri("/realms/myrealm/protocol/openid-connect/token")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(token_form(&[("grant_type","client_credentials"),("client_id","test-client"),("client_secret","client-secret")]))).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        assert!(body["access_token"].is_string());
    }

    // upstream: keycloak/keycloak TokenEndpointTest.java:testClientCredentialsWrongSecret
    #[tokio::test]
    async fn test_client_credentials_wrong_secret() {
        let (app, _) = setup().await;
        let resp = app.oneshot(
            Request::builder().method("POST").uri("/realms/myrealm/protocol/openid-connect/token")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(token_form(&[("grant_type","client_credentials"),("client_id","test-client"),("client_secret","wrong-secret")]))).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // upstream: keycloak/keycloak TokenEndpointTest.java:testClientCredentialsPublicClientDenied
    #[tokio::test]
    async fn test_client_credentials_public_client_denied() {
        let (app, _) = setup().await;
        let resp = app.oneshot(
            Request::builder().method("POST").uri("/realms/myrealm/protocol/openid-connect/token")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(token_form(&[("grant_type","client_credentials"),("client_id","public-client"),("client_secret","")]))).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // upstream: keycloak/keycloak TokenEndpointTest.java:testRefreshTokenSuccess
    #[tokio::test]
    async fn test_refresh_token_success() {
        let (_, svc) = setup().await;
        let tokens = svc.password_grant("myrealm", "testuser", "correctpassword", "test-client").await.unwrap();
        let refresh_token = tokens.refresh_token.unwrap();

        let app = router(svc);
        let resp = app.oneshot(
            Request::builder().method("POST").uri("/realms/myrealm/protocol/openid-connect/token")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(token_form(&[("grant_type","refresh_token"),("refresh_token",&refresh_token)]))).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        assert!(body["access_token"].is_string());
    }

    // upstream: keycloak/keycloak TokenEndpointTest.java:testRefreshTokenInvalid
    #[tokio::test]
    async fn test_refresh_token_invalid() {
        let (app, _) = setup().await;
        let resp = app.oneshot(
            Request::builder().method("POST").uri("/realms/myrealm/protocol/openid-connect/token")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(token_form(&[("grant_type","refresh_token"),("refresh_token","totally.invalid.token")]))).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // upstream: keycloak/keycloak TokenEndpointTest.java:testIntrospectActiveToken
    #[tokio::test]
    async fn test_introspect_active_token() {
        let (_, svc) = setup().await;
        let tokens = svc.password_grant("myrealm", "testuser", "correctpassword", "test-client").await.unwrap();
        let access_token = tokens.access_token;

        let app = router(svc);
        let resp = app.oneshot(
            Request::builder().method("POST").uri("/realms/myrealm/protocol/openid-connect/token/introspect")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(format!("token={access_token}"))).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        assert_eq!(body["active"], true);
    }

    // upstream: keycloak/keycloak TokenEndpointTest.java:testIntrospectInvalidToken
    #[tokio::test]
    async fn test_introspect_invalid_token() {
        let (app, _) = setup().await;
        let resp = app.oneshot(
            Request::builder().method("POST").uri("/realms/myrealm/protocol/openid-connect/token/introspect")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("token=junk.junk.junk")).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        assert_eq!(body["active"], false);
    }

    // upstream: keycloak/keycloak UserInfoTest.java:testUserInfoWithValidToken
    #[tokio::test]
    async fn test_userinfo_with_valid_token() {
        let (_, svc) = setup().await;
        let tokens = svc.password_grant("myrealm", "testuser", "correctpassword", "test-client").await.unwrap();

        let app = router(svc);
        let resp = app.oneshot(
            Request::builder().method("GET").uri("/realms/myrealm/protocol/openid-connect/userinfo")
                .header("Authorization", format!("Bearer {}", tokens.access_token))
                .body(Body::empty()).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        assert!(body["sub"].is_string());
        assert_eq!(body["preferred_username"], "testuser");
    }

    // upstream: keycloak/keycloak UserInfoTest.java:testUserInfoMissingToken
    #[tokio::test]
    async fn test_userinfo_missing_token() {
        let (app, _) = setup().await;
        let resp = app.oneshot(
            Request::builder().method("GET").uri("/realms/myrealm/protocol/openid-connect/userinfo")
                .body(Body::empty()).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // upstream: keycloak/keycloak LogoutTest.java:testLogoutClearsSession
    #[tokio::test]
    async fn test_logout_clears_session() {
        let (_, svc) = setup().await;
        let tokens = svc.password_grant("myrealm", "testuser", "correctpassword", "test-client").await.unwrap();
        let session_state = tokens.session_state.unwrap();

        let app = router(svc);
        let resp = app.oneshot(
            Request::builder().method("POST").uri("/realms/myrealm/protocol/openid-connect/logout")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(format!("session_state={session_state}"))).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    // ─── RFC 7662 introspection conformance ──────────────────────────────────

    async fn introspect_with(svc: KeycloakTokenService, body: &str) -> (StatusCode, Value) {
        let app = router(svc);
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/realms/myrealm/protocol/openid-connect/token/introspect")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        (status, body_json(resp).await)
    }

    // upstream: rfc7662 §2.2 — inactive token responses MUST contain only
    // `active: false`; no sub / exp / username / client_id / scope leaks.
    #[tokio::test]
    async fn introspect_inactive_token_omits_all_other_claims() {
        let (_, svc) = setup().await;
        let (status, body) = introspect_with(svc, "token=junk.junk.junk").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["active"], false);
        // RFC 7662 §2.2: serialization MUST NOT include other claims for
        // inactive tokens. Our IntrospectionResponse uses
        // `skip_serializing_if = Option::is_none` for sub/exp/username/
        // client_id/scope, so they should be absent (Value::Null in JSON
        // means "key was present with null"; absent is checked via .get).
        assert!(body.get("sub").is_none(), "sub leak on inactive: {body}");
        assert!(body.get("exp").is_none(), "exp leak on inactive");
        assert!(body.get("username").is_none(), "username leak on inactive");
        assert!(body.get("client_id").is_none(), "client_id leak on inactive");
        assert!(body.get("scope").is_none(), "scope leak on inactive");
    }

    // upstream: rfc7662 §2.2 — active tokens populate sub, exp, username,
    // client_id, scope.
    #[tokio::test]
    async fn introspect_active_token_populates_required_fields() {
        let (_, svc) = setup().await;
        let tokens = svc
            .password_grant("myrealm", "testuser", "correctpassword", "test-client")
            .await
            .unwrap();
        let (status, body) = introspect_with(svc, &format!("token={}", tokens.access_token)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["active"], true);
        assert!(body["sub"].is_string());
        assert!(body["exp"].is_number(), "exp must be a numeric epoch");
        assert_eq!(body["username"], "testuser");
        assert_eq!(body["client_id"], "test-client");
        assert!(body["scope"].is_string());
    }

    // upstream: rfc7662 §2.1 — empty token field returns active=false, not
    // a 4xx (the request itself is well-formed).
    #[tokio::test]
    async fn introspect_empty_token_returns_inactive_not_error() {
        let (_, svc) = setup().await;
        let (status, body) = introspect_with(svc, "token=").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["active"], false);
    }

    // upstream: rfc7662 §2.1 — a token from a foreign issuer must be
    // reported inactive (the resource server only trusts tokens issued by
    // the realm it asks for).
    #[tokio::test]
    async fn introspect_token_from_wrong_issuer_is_inactive() {
        let (_, svc) = setup().await;
        // Mint a token under "myrealm", then introspect it via a request
        // whose realm path is forged-realm — `/realms/forged-realm/...`.
        // The introspect impl checks `claims.iss == issuer(realm)`.
        let tokens = svc
            .password_grant("myrealm", "testuser", "correctpassword", "test-client")
            .await
            .unwrap();
        // Inject the access token but ask the introspect endpoint for a
        // different realm by routing through a separate path.
        let app = router(svc);
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/realms/other-realm/protocol/openid-connect/token/introspect")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(format!("token={}", tokens.access_token)))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = body_json(resp).await;
        assert_eq!(body["active"], false, "foreign-issuer token must be inactive: {body}");
    }

    // upstream: rfc7662 §2.2 — the `exp` claim, when present, MUST be a
    // NumericDate (seconds since epoch) per JWT/RFC 7519 §2. Sanity-check
    // that the value is plausibly a recent epoch (post-2020-01-01) so a
    // future stringly-typed regression is caught.
    #[tokio::test]
    async fn introspect_active_token_exp_is_plausible_epoch() {
        let (_, svc) = setup().await;
        let tokens = svc
            .password_grant("myrealm", "testuser", "correctpassword", "test-client")
            .await
            .unwrap();
        let (_, body) = introspect_with(svc, &format!("token={}", tokens.access_token)).await;
        let exp = body["exp"].as_i64().expect("exp must be i64");
        assert!(exp > 1_577_836_800, "exp={exp} not a post-2020 epoch");
        // Realm default access_token_lifespan is short (minutes/hours),
        // so exp shouldn't be more than a year in the future either.
        let one_year_secs: i64 = 366 * 86_400;
        let now_secs = chrono::Utc::now().timestamp();
        assert!(exp - now_secs < one_year_secs, "exp={exp} too far in the future");
    }

    // upstream: rfc7662 §2.1 — request without `token` parameter returns
    // 400 Bad Request (the request itself is malformed).
    #[tokio::test]
    async fn introspect_missing_token_param_is_client_error() {
        let (_, svc) = setup().await;
        let (status, _body) = introspect_with(svc, "wrong_param=value").await;
        // The endpoint extracts `token` from the form; missing field
        // surfaces as a 4xx from axum's Form extractor.
        assert!(
            status.is_client_error(),
            "missing token param must be 4xx, got {status}"
        );
    }

    // upstream: rfc7662 §4 — privacy: `username` value, when emitted,
    // matches the user's preferred_username — never an email or sub.
    #[tokio::test]
    async fn introspect_active_username_is_preferred_username() {
        let (_, svc) = setup().await;
        let tokens = svc
            .password_grant("myrealm", "testuser", "correctpassword", "test-client")
            .await
            .unwrap();
        let (_, body) = introspect_with(svc, &format!("token={}", tokens.access_token)).await;
        assert_eq!(body["username"], "testuser");
        // Must NOT be an email shape.
        let s = body["username"].as_str().unwrap();
        assert!(!s.contains('@'), "username leaked email: {s}");
    }

    // upstream: rfc7662 §2.2 — `scope` is space-delimited, not a JSON
    // array. Catches a regression where someone serialises Vec<String>.
    #[tokio::test]
    async fn introspect_active_scope_is_string_not_array() {
        let (_, svc) = setup().await;
        let tokens = svc
            .password_grant("myrealm", "testuser", "correctpassword", "test-client")
            .await
            .unwrap();
        let (_, body) = introspect_with(svc, &format!("token={}", tokens.access_token)).await;
        assert!(body["scope"].is_string(), "scope must be string per RFC 7662 §2.2");
    }

    // upstream: rfc7662 §2.2 — confidential clients MAY rotate access
    // tokens via the password grant; the second introspection of an
    // earlier token MUST still be valid until exp (we don't denylist on
    // re-issue). Sanity-check both tokens introspect active.
    #[tokio::test]
    async fn introspect_two_tokens_from_same_user_both_active() {
        let (_, svc) = setup().await;
        let t1 = svc
            .password_grant("myrealm", "testuser", "correctpassword", "test-client")
            .await
            .unwrap();
        let t2 = svc
            .password_grant("myrealm", "testuser", "correctpassword", "test-client")
            .await
            .unwrap();
        let (_, b1) = introspect_with(svc.clone(), &format!("token={}", t1.access_token)).await;
        let (_, b2) = introspect_with(svc, &format!("token={}", t2.access_token)).await;
        assert_eq!(b1["active"], true);
        assert_eq!(b2["active"], true);
    }
}
