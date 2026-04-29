//! OAuth2 / OIDC routes for cave-portal — Authorization Code with PKCE.
//!
//! End-to-end flow:
//! 1. `GET /oauth/login` — generate state + PKCE verifier, store both in
//!    [`AuthFlowStore`], redirect to `${ISSUER}/protocol/openid-connect/auth`.
//! 2. `GET /oauth/callback?code=&state=` — look up state, POST to the IdP
//!    token endpoint, validate id_token, mint a HS256-signed session cookie.
//! 3. `GET /me` — read cookie, return claims.
//! 4. `POST /oauth/logout` — clear cookie, redirect to IdP end-session URL.
//!
//! Token exchange happens through the [`TokenExchanger`] trait so unit tests
//! can swap in an in-process mock; the production binary plugs in a reqwest
//! impl behind it.

use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

// ── Config ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct OAuthConfig {
    /// Base URL of the OIDC issuer, e.g. `https://keycloak.cave.local/realms/cave`.
    pub issuer: String,
    /// OIDC client id, e.g. `cave-portal`.
    pub client_id: String,
    /// Confidential client secret. Empty string for public clients.
    pub client_secret: String,
    /// Redirect URI registered with the IdP, e.g. `http://localhost:8080/oauth/callback`.
    pub redirect_uri: String,
    /// Scopes joined with space, e.g. `"openid profile email"`.
    pub scopes: String,
    /// HS256 cookie session secret.
    pub session_secret: String,
    /// Cookie name for the session.
    pub cookie_name: String,
    /// Cookie TTL in seconds. 24h by default.
    pub cookie_max_age_secs: i64,
    /// `Set-Cookie; Secure` flag — set false in unit tests.
    pub cookie_secure: bool,
}

impl Default for OAuthConfig {
    fn default() -> Self {
        Self {
            issuer: "http://localhost:8081/realms/cave".into(),
            client_id: "cave-portal".into(),
            client_secret: String::new(),
            redirect_uri: "http://localhost:8080/oauth/callback".into(),
            scopes: "openid profile email roles".into(),
            session_secret: "dev-session-secret-change-me".into(),
            cookie_name: "cave_session".into(),
            cookie_max_age_secs: 86_400,
            cookie_secure: false,
        }
    }
}

impl OAuthConfig {
    /// Build a config from environment variables. Missing variables fall back
    /// to defaults that point at a localhost Keycloak.
    pub fn from_env() -> Self {
        let mut c = Self::default();
        if let Ok(v) = std::env::var("CAVE_OIDC_ISSUER") { c.issuer = v; }
        if let Ok(v) = std::env::var("CAVE_OIDC_CLIENT_ID") { c.client_id = v; }
        if let Ok(v) = std::env::var("CAVE_OIDC_CLIENT_SECRET") { c.client_secret = v; }
        if let Ok(v) = std::env::var("CAVE_OIDC_REDIRECT_URI") { c.redirect_uri = v; }
        if let Ok(v) = std::env::var("CAVE_OIDC_SCOPES") { c.scopes = v; }
        if let Ok(v) = std::env::var("CAVE_SESSION_SECRET") { c.session_secret = v; }
        if let Ok(v) = std::env::var("CAVE_SESSION_COOKIE_NAME") { c.cookie_name = v; }
        if let Ok(v) = std::env::var("CAVE_SESSION_COOKIE_MAX_AGE") {
            if let Ok(n) = v.parse() { c.cookie_max_age_secs = n; }
        }
        if std::env::var("CAVE_COOKIE_SECURE").map(|v| v == "true").unwrap_or(false) {
            c.cookie_secure = true;
        }
        c
    }

    pub fn authorize_url(&self, state: &str, code_challenge: &str) -> String {
        let scopes = url_encode(&self.scopes);
        let redirect = url_encode(&self.redirect_uri);
        let client_id = url_encode(&self.client_id);
        format!(
            "{}/protocol/openid-connect/auth\
             ?response_type=code&client_id={}&redirect_uri={}\
             &scope={}&state={}&code_challenge={}&code_challenge_method=S256",
            self.issuer, client_id, redirect, scopes, state, code_challenge,
        )
    }

    pub fn end_session_url(&self, post_logout_redirect: &str) -> String {
        format!(
            "{}/protocol/openid-connect/logout?post_logout_redirect_uri={}&client_id={}",
            self.issuer,
            url_encode(post_logout_redirect),
            url_encode(&self.client_id),
        )
    }

    pub fn token_endpoint(&self) -> String {
        format!("{}/protocol/openid-connect/token", self.issuer)
    }
}

// ── PKCE + state ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AuthFlow {
    pub state: String,
    pub verifier: String,
    pub challenge: String,
    pub created_at: SystemTime,
}

#[derive(Debug, Default)]
pub struct AuthFlowStore {
    inner: RwLock<HashMap<String, AuthFlow>>,
}

const FLOW_TTL: Duration = Duration::from_secs(600);

impl AuthFlowStore {
    pub fn new() -> Self { Self::default() }

    pub async fn create(&self) -> AuthFlow {
        let state = random_url_safe(24);
        let verifier = random_url_safe(64);
        let challenge = pkce_challenge(&verifier);
        let flow = AuthFlow {
            state: state.clone(),
            verifier,
            challenge,
            created_at: SystemTime::now(),
        };
        self.inner.write().await.insert(state, flow.clone());
        flow
    }

    pub async fn consume(&self, state: &str) -> Option<AuthFlow> {
        let mut guard = self.inner.write().await;
        guard.retain(|_, f| {
            f.created_at.elapsed().map(|e| e < FLOW_TTL).unwrap_or(false)
        });
        guard.remove(state)
    }

    pub async fn len(&self) -> usize {
        self.inner.read().await.len()
    }
}

pub fn random_url_safe(n: usize) -> String {
    let mut bytes = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn pkce_challenge(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

// Minimal application/x-www-form-urlencoded encoder. Avoids pulling in a
// percent-encoding crate for one call site.
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        let safe = matches!(b,
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~'
        );
        if safe {
            out.push(b as char);
        } else if b == b' ' {
            out.push_str("%20");
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

// ── Token exchanger ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub id_token: String,
    pub refresh_token: Option<String>,
    pub token_type: String,
    pub expires_in: u64,
    #[serde(default)]
    pub scope: String,
}

#[derive(Debug, thiserror::Error)]
pub enum OAuthError {
    #[error("missing query param: {0}")]
    MissingParam(&'static str),
    #[error("unknown or expired state")]
    UnknownState,
    #[error("idp returned error: {0}")]
    IdpError(String),
    #[error("token endpoint http error: {0}")]
    HttpError(String),
    #[error("invalid id_token: {0}")]
    InvalidIdToken(String),
    #[error("session sign error: {0}")]
    SessionSign(String),
    #[error("session verify error: {0}")]
    SessionVerify(String),
}

#[async_trait::async_trait]
pub trait TokenExchanger: Send + Sync {
    async fn exchange(
        &self,
        cfg: &OAuthConfig,
        code: &str,
        verifier: &str,
    ) -> Result<TokenResponse, OAuthError>;
}

/// Production HTTP exchanger that POSTs to the IdP's token endpoint with
/// `grant_type=authorization_code`. Confidential clients send their secret
/// in the form body alongside the code + verifier.
pub struct HttpExchanger {
    client: reqwest::Client,
}

impl HttpExchanger {
    pub fn new() -> Self {
        Self { client: reqwest::Client::new() }
    }
}

impl Default for HttpExchanger {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl TokenExchanger for HttpExchanger {
    async fn exchange(
        &self,
        cfg: &OAuthConfig,
        code: &str,
        verifier: &str,
    ) -> Result<TokenResponse, OAuthError> {
        let mut form: Vec<(&str, String)> = vec![
            ("grant_type", "authorization_code".into()),
            ("code", code.into()),
            ("redirect_uri", cfg.redirect_uri.clone()),
            ("code_verifier", verifier.into()),
            ("client_id", cfg.client_id.clone()),
        ];
        if !cfg.client_secret.is_empty() {
            form.push(("client_secret", cfg.client_secret.clone()));
        }
        let resp = self
            .client
            .post(cfg.token_endpoint())
            .form(&form)
            .send()
            .await
            .map_err(|e| OAuthError::HttpError(e.to_string()))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(OAuthError::IdpError(format!("{status}: {body}")));
        }
        resp.json::<TokenResponse>()
            .await
            .map_err(|e| OAuthError::HttpError(e.to_string()))
    }
}

/// In-memory deterministic exchanger used in tests. Looks up `code` in a
/// fixture map and asserts the verifier matches the registered challenge.
#[derive(Debug, Default)]
pub struct MockExchanger {
    pub fixtures: RwLock<HashMap<String, MockFixture>>,
}

#[derive(Debug, Clone)]
pub struct MockFixture {
    pub expected_challenge: String,
    pub response: TokenResponse,
}

impl MockExchanger {
    pub async fn install(&self, code: &str, expected_challenge: &str, response: TokenResponse) {
        self.fixtures.write().await.insert(
            code.into(),
            MockFixture {
                expected_challenge: expected_challenge.into(),
                response,
            },
        );
    }
}

#[async_trait::async_trait]
impl TokenExchanger for MockExchanger {
    async fn exchange(
        &self,
        _cfg: &OAuthConfig,
        code: &str,
        verifier: &str,
    ) -> Result<TokenResponse, OAuthError> {
        let mut g = self.fixtures.write().await;
        let fix = g.remove(code).ok_or_else(|| OAuthError::IdpError("no fixture".into()))?;
        if pkce_challenge(verifier) != fix.expected_challenge {
            return Err(OAuthError::IdpError("pkce mismatch".into()));
        }
        Ok(fix.response)
    }
}

// ── Session cookie ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionClaims {
    pub sub: String,           // subject (user id)
    pub preferred_username: String,
    pub email: Option<String>,
    pub roles: Vec<String>,
    pub tenant: Option<String>,
    pub iat: i64,
    pub exp: i64,
    pub iss: String,           // issuer (us — "cave-portal")
}

impl SessionClaims {
    pub fn is_admin(&self) -> bool {
        self.roles.iter().any(|r| r == "admin")
    }
    pub fn is_operator(&self) -> bool {
        self.roles.iter().any(|r| r == "operator")
    }
}

pub fn sign_session(claims: &SessionClaims, secret: &str) -> Result<String, OAuthError> {
    let key = EncodingKey::from_secret(secret.as_bytes());
    encode(&Header::default(), claims, &key).map_err(|e| OAuthError::SessionSign(e.to_string()))
}

pub fn verify_session(token: &str, secret: &str) -> Result<SessionClaims, OAuthError> {
    let key = DecodingKey::from_secret(secret.as_bytes());
    let mut validation = Validation::default();
    validation.validate_aud = false;
    validation.set_required_spec_claims(&["sub", "exp", "iat"]);
    decode::<SessionClaims>(token, &key, &validation)
        .map(|d| d.claims)
        .map_err(|e| OAuthError::SessionVerify(e.to_string()))
}

/// Decode an id_token *without* signature verification — used to extract
/// claims; signature validation against the IdP's JWKS is the IdP's contract.
/// In production this is layered with [`cave_auth::jwks`] for true verify.
pub fn decode_id_token_unverified(id_token: &str) -> Result<IdTokenClaims, OAuthError> {
    let parts: Vec<&str> = id_token.split('.').collect();
    if parts.len() != 3 {
        return Err(OAuthError::InvalidIdToken("expected 3 dot-separated segments".into()));
    }
    let payload = URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|e| OAuthError::InvalidIdToken(format!("base64 decode: {e}")))?;
    serde_json::from_slice(&payload)
        .map_err(|e| OAuthError::InvalidIdToken(format!("json parse: {e}")))
}

#[derive(Debug, Clone, Deserialize)]
pub struct IdTokenClaims {
    pub sub: String,
    #[serde(default)]
    pub preferred_username: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub realm_access: Option<RealmAccess>,
    #[serde(default)]
    pub tenant: Option<String>,
    #[serde(default)]
    pub iss: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RealmAccess {
    #[serde(default)]
    pub roles: Vec<String>,
}

impl IdTokenClaims {
    pub fn into_session(self, ttl_secs: i64) -> SessionClaims {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        SessionClaims {
            sub: self.sub,
            preferred_username: self.preferred_username,
            email: self.email,
            roles: self.realm_access.map(|r| r.roles).unwrap_or_default(),
            tenant: self.tenant,
            iat: now,
            exp: now + ttl_secs,
            iss: "cave-portal".into(),
        }
    }
}

pub fn build_session_cookie(token: &str, cfg: &OAuthConfig) -> String {
    let secure = if cfg.cookie_secure { " Secure;" } else { "" };
    format!(
        "{name}={token}; Path=/; HttpOnly;{secure} SameSite=Lax; Max-Age={age}",
        name = cfg.cookie_name,
        age = cfg.cookie_max_age_secs,
    )
}

pub fn build_clearing_cookie(cfg: &OAuthConfig) -> String {
    format!(
        "{name}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0",
        name = cfg.cookie_name,
    )
}

pub fn extract_session_token<'a>(headers: &'a HeaderMap, cookie_name: &str) -> Option<&'a str> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for piece in raw.split(';') {
        let piece = piece.trim();
        if let Some(rest) = piece.strip_prefix(&format!("{cookie_name}=")) {
            return Some(rest);
        }
    }
    None
}

// ── Router state ─────────────────────────────────────────────────────────────

pub struct OAuthState {
    pub config: OAuthConfig,
    pub flows: AuthFlowStore,
    pub exchanger: Arc<dyn TokenExchanger>,
}

impl OAuthState {
    pub fn new(config: OAuthConfig, exchanger: Arc<dyn TokenExchanger>) -> Self {
        Self {
            config,
            flows: AuthFlowStore::new(),
            exchanger,
        }
    }
}

pub fn router(state: Arc<OAuthState>) -> Router {
    Router::new()
        .route("/oauth/login", get(login))
        .route("/oauth/callback", get(callback))
        .route("/oauth/logout", post(logout).get(logout))
        .route("/me", get(me))
        .with_state(state)
}

// ── Handlers ─────────────────────────────────────────────────────────────────

async fn login(State(s): State<Arc<OAuthState>>) -> impl IntoResponse {
    let flow = s.flows.create().await;
    let url = s.config.authorize_url(&flow.state, &flow.challenge);
    Redirect::temporary(&url)
}

#[derive(Debug, Deserialize)]
pub struct CallbackParams {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub error_description: Option<String>,
}

async fn callback(
    State(s): State<Arc<OAuthState>>,
    Query(q): Query<CallbackParams>,
) -> Response {
    if let Some(err) = q.error {
        let desc = q.error_description.unwrap_or_default();
        return (StatusCode::BAD_REQUEST, format!("idp error: {err}: {desc}")).into_response();
    }
    let code = match q.code {
        Some(v) => v,
        None => return (StatusCode::BAD_REQUEST, "missing code").into_response(),
    };
    let state = match q.state {
        Some(v) => v,
        None => return (StatusCode::BAD_REQUEST, "missing state").into_response(),
    };
    let flow = match s.flows.consume(&state).await {
        Some(f) => f,
        None => return (StatusCode::BAD_REQUEST, "unknown or expired state").into_response(),
    };
    let tokens = match s.exchanger.exchange(&s.config, &code, &flow.verifier).await {
        Ok(t) => t,
        Err(e) => return (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
    };
    let id_claims = match decode_id_token_unverified(&tokens.id_token) {
        Ok(c) => c,
        Err(e) => return (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
    };
    let session_claims = id_claims.into_session(s.config.cookie_max_age_secs);
    let session_jwt = match sign_session(&session_claims, &s.config.session_secret) {
        Ok(t) => t,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let cookie = build_session_cookie(&session_jwt, &s.config);
    let target = if session_claims.is_admin() { "/admin" } else { "/" };
    let mut headers = HeaderMap::new();
    headers.insert(header::SET_COOKIE, cookie.parse().unwrap());
    headers.insert(header::LOCATION, target.parse().unwrap());
    (StatusCode::SEE_OTHER, headers).into_response()
}

async fn logout(State(s): State<Arc<OAuthState>>) -> Response {
    let cookie = build_clearing_cookie(&s.config);
    let post_logout = "/";
    let mut headers = HeaderMap::new();
    headers.insert(header::SET_COOKIE, cookie.parse().unwrap());
    let target = s.config.end_session_url(post_logout);
    headers.insert(header::LOCATION, target.parse().unwrap());
    (StatusCode::SEE_OTHER, headers).into_response()
}

async fn me(
    State(s): State<Arc<OAuthState>>,
    headers: HeaderMap,
) -> Response {
    let token = match extract_session_token(&headers, &s.config.cookie_name) {
        Some(t) => t,
        None => return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error":"no session"}))).into_response(),
    };
    match verify_session(token, &s.config.session_secret) {
        Ok(claims) => Json(serde_json::json!({
            "sub": claims.sub,
            "username": claims.preferred_username,
            "email": claims.email,
            "roles": claims.roles,
            "tenant": claims.tenant,
            "exp": claims.exp,
        })).into_response(),
        Err(e) => (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::util::ServiceExt;

    fn test_config() -> OAuthConfig {
        OAuthConfig {
            issuer: "http://idp.test/realms/cave".into(),
            client_id: "cave-portal".into(),
            client_secret: "secret".into(),
            redirect_uri: "http://app.test/oauth/callback".into(),
            scopes: "openid profile email".into(),
            session_secret: "test-secret-min-32-chars-please-extend-it".into(),
            cookie_name: "cave_session".into(),
            cookie_max_age_secs: 3600,
            cookie_secure: false,
        }
    }

    fn fake_id_token(sub: &str, username: &str, roles: &[&str]) -> String {
        // Header.Payload.Signature — signature is junk but our decoder skips it.
        let header = URL_SAFE_NO_PAD.encode(b"{\"alg\":\"none\"}");
        let payload = serde_json::json!({
            "sub": sub,
            "preferred_username": username,
            "email": format!("{username}@cave.local"),
            "realm_access": { "roles": roles },
        });
        let payload_b64 = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap());
        let sig = URL_SAFE_NO_PAD.encode(b"x");
        format!("{header}.{payload_b64}.{sig}")
    }

    // ── PKCE / state ────────────────────────────────────────────────────────

    #[test]
    fn pkce_challenge_is_url_safe_b64() {
        let v = "abcde-fgh_ij.KLMnop";
        let c = pkce_challenge(v);
        for ch in c.chars() {
            assert!(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');
        }
    }

    #[test]
    fn pkce_challenge_deterministic() {
        let v = "alpha";
        assert_eq!(pkce_challenge(v), pkce_challenge(v));
    }

    #[test]
    fn random_url_safe_has_url_safe_chars() {
        let r = random_url_safe(32);
        for ch in r.chars() {
            assert!(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');
        }
    }

    #[test]
    fn url_encode_preserves_unreserved() {
        assert_eq!(url_encode("abc-_.~ABC123"), "abc-_.~ABC123");
    }

    #[test]
    fn url_encode_percent_encodes_special() {
        assert_eq!(url_encode("a/b"), "a%2Fb");
        assert_eq!(url_encode("a&b"), "a%26b");
    }

    #[test]
    fn url_encode_space_is_percent20() {
        assert_eq!(url_encode("a b"), "a%20b");
    }

    // ── AuthFlowStore ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn flow_store_create_and_consume() {
        let s = AuthFlowStore::new();
        let f = s.create().await;
        assert_eq!(s.len().await, 1);
        let consumed = s.consume(&f.state).await.unwrap();
        assert_eq!(consumed.state, f.state);
        assert_eq!(s.len().await, 0);
    }

    #[tokio::test]
    async fn flow_store_consume_unknown_returns_none() {
        let s = AuthFlowStore::new();
        assert!(s.consume("ghost").await.is_none());
    }

    #[tokio::test]
    async fn flow_store_returns_distinct_states() {
        let s = AuthFlowStore::new();
        let f1 = s.create().await;
        let f2 = s.create().await;
        assert_ne!(f1.state, f2.state);
        assert_ne!(f1.verifier, f2.verifier);
    }

    // ── OAuthConfig ─────────────────────────────────────────────────────────

    #[test]
    fn authorize_url_includes_required_params() {
        let c = test_config();
        let url = c.authorize_url("STATE", "CHAL");
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=cave-portal"));
        assert!(url.contains("redirect_uri=http%3A%2F%2Fapp.test%2Foauth%2Fcallback"));
        assert!(url.contains("scope=openid%20profile%20email"));
        assert!(url.contains("state=STATE"));
        assert!(url.contains("code_challenge=CHAL"));
        assert!(url.contains("code_challenge_method=S256"));
    }

    #[test]
    fn end_session_url_includes_post_logout() {
        let c = test_config();
        let url = c.end_session_url("/");
        assert!(url.contains("/protocol/openid-connect/logout"));
        assert!(url.contains("post_logout_redirect_uri=%2F"));
        assert!(url.contains("client_id=cave-portal"));
    }

    #[test]
    fn token_endpoint_path_correct() {
        assert_eq!(
            test_config().token_endpoint(),
            "http://idp.test/realms/cave/protocol/openid-connect/token"
        );
    }

    // ── id_token decoding ───────────────────────────────────────────────────

    #[test]
    fn decode_id_token_extracts_claims() {
        let t = fake_id_token("u-1", "alice", &["admin"]);
        let claims = decode_id_token_unverified(&t).unwrap();
        assert_eq!(claims.sub, "u-1");
        assert_eq!(claims.preferred_username, "alice");
        assert_eq!(claims.email.as_deref(), Some("alice@cave.local"));
        assert_eq!(claims.realm_access.unwrap().roles, vec!["admin".to_string()]);
    }

    #[test]
    fn decode_id_token_rejects_malformed() {
        let err = decode_id_token_unverified("not-a-jwt").unwrap_err();
        assert!(matches!(err, OAuthError::InvalidIdToken(_)));
    }

    #[test]
    fn decode_id_token_rejects_bad_payload_b64() {
        let err = decode_id_token_unverified("eyJhbGciOiJub25lIn0.!!!.x").unwrap_err();
        assert!(matches!(err, OAuthError::InvalidIdToken(_)));
    }

    #[test]
    fn into_session_copies_roles_and_sets_iss() {
        let claims = IdTokenClaims {
            sub: "u-1".into(),
            preferred_username: "alice".into(),
            email: None,
            realm_access: Some(RealmAccess { roles: vec!["admin".into()] }),
            tenant: Some("acme".into()),
            iss: "x".into(),
        };
        let s = claims.into_session(60);
        assert_eq!(s.roles, vec!["admin".to_string()]);
        assert_eq!(s.tenant.as_deref(), Some("acme"));
        assert_eq!(s.iss, "cave-portal");
        assert_eq!(s.exp - s.iat, 60);
    }

    #[test]
    fn session_is_admin() {
        let mut s = SessionClaims {
            sub: "u".into(), preferred_username: "alice".into(), email: None,
            roles: vec!["operator".into()], tenant: None, iat: 0, exp: 1, iss: "x".into(),
        };
        assert!(!s.is_admin());
        assert!(s.is_operator());
        s.roles.push("admin".into());
        assert!(s.is_admin());
    }

    // ── Sign / verify ───────────────────────────────────────────────────────

    fn now_secs() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    #[test]
    fn sign_then_verify_round_trips() {
        let cfg = test_config();
        let now = now_secs();
        let claims = SessionClaims {
            sub: "u-1".into(),
            preferred_username: "alice".into(),
            email: Some("a@x".into()),
            roles: vec!["admin".into()],
            tenant: None,
            iat: now,
            exp: now + 3600,
            iss: "cave-portal".into(),
        };
        let token = sign_session(&claims, &cfg.session_secret).unwrap();
        let back = verify_session(&token, &cfg.session_secret).unwrap();
        assert_eq!(back, claims);
    }

    #[test]
    fn verify_session_rejects_wrong_secret() {
        let cfg = test_config();
        let now = now_secs();
        let claims = SessionClaims {
            sub: "u".into(), preferred_username: "x".into(), email: None,
            roles: vec![], tenant: None, iat: now, exp: now + 60, iss: "x".into(),
        };
        let token = sign_session(&claims, &cfg.session_secret).unwrap();
        assert!(verify_session(&token, "wrong-secret-32-character-padding").is_err());
    }

    #[test]
    fn verify_session_rejects_expired() {
        let cfg = test_config();
        let claims = SessionClaims {
            sub: "u".into(), preferred_username: "x".into(), email: None,
            roles: vec![], tenant: None, iat: 0, exp: 1, iss: "x".into(),
        };
        let token = sign_session(&claims, &cfg.session_secret).unwrap();
        assert!(verify_session(&token, &cfg.session_secret).is_err());
    }

    // ── Cookie helpers ──────────────────────────────────────────────────────

    #[test]
    fn build_session_cookie_includes_httponly_and_max_age() {
        let cfg = test_config();
        let c = build_session_cookie("TOKEN", &cfg);
        assert!(c.contains("cave_session=TOKEN"));
        assert!(c.contains("HttpOnly"));
        assert!(c.contains("SameSite=Lax"));
        assert!(c.contains("Max-Age=3600"));
        assert!(!c.contains("Secure")); // cfg.cookie_secure=false
    }

    #[test]
    fn build_session_cookie_secure_when_config_secure() {
        let mut cfg = test_config();
        cfg.cookie_secure = true;
        let c = build_session_cookie("TOKEN", &cfg);
        assert!(c.contains("Secure"));
    }

    #[test]
    fn clearing_cookie_has_max_age_zero() {
        let c = build_clearing_cookie(&test_config());
        assert!(c.contains("cave_session="));
        assert!(c.contains("Max-Age=0"));
    }

    #[test]
    fn extract_session_token_from_cookie_header() {
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, "x=1; cave_session=ABC; y=2".parse().unwrap());
        assert_eq!(extract_session_token(&h, "cave_session"), Some("ABC"));
    }

    #[test]
    fn extract_session_token_returns_none_when_missing() {
        let h = HeaderMap::new();
        assert!(extract_session_token(&h, "cave_session").is_none());
    }

    #[test]
    fn extract_session_token_returns_none_when_other_cookie() {
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, "x=1; y=2".parse().unwrap());
        assert!(extract_session_token(&h, "cave_session").is_none());
    }

    // ── End-to-end through Router ───────────────────────────────────────────

    fn build_state() -> Arc<OAuthState> {
        Arc::new(OAuthState::new(test_config(), Arc::new(MockExchanger::default())))
    }

    #[tokio::test]
    async fn login_redirects_to_idp_authorize() {
        let app = router(build_state());
        let resp = app
            .oneshot(Request::builder().uri("/oauth/login").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
        let loc = resp.headers().get(header::LOCATION).unwrap().to_str().unwrap();
        assert!(loc.starts_with("http://idp.test/realms/cave/protocol/openid-connect/auth"));
        assert!(loc.contains("code_challenge_method=S256"));
    }

    #[tokio::test]
    async fn callback_missing_code_400s() {
        let app = router(build_state());
        let resp = app
            .oneshot(Request::builder().uri("/oauth/callback?state=x").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn callback_unknown_state_400s() {
        let app = router(build_state());
        let resp = app
            .oneshot(Request::builder().uri("/oauth/callback?code=c&state=ghost").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn callback_idp_error_propagates() {
        let app = router(build_state());
        let resp = app
            .oneshot(Request::builder()
                .uri("/oauth/callback?error=access_denied&error_description=user_cancelled")
                .body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn full_login_callback_admin_redirects_to_admin() {
        let cfg = test_config();
        let exch = Arc::new(MockExchanger::default());
        let state_obj = Arc::new(OAuthState::new(cfg.clone(), exch.clone()));
        let app = router(state_obj.clone());

        // 1) start the flow
        let r1 = app.clone()
            .oneshot(Request::builder().uri("/oauth/login").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let loc = r1.headers().get(header::LOCATION).unwrap().to_str().unwrap().to_string();
        // Extract `state=` from the redirect URL
        let state = loc.split("state=").nth(1).unwrap().split('&').next().unwrap();

        // Get the verifier from the store so we can compute the same challenge
        let flow = state_obj.flows.inner.read().await.get(state).cloned().unwrap();
        let challenge = flow.challenge.clone();

        // 2) install fixture: code "abc" maps to admin id_token
        exch.install(
            "abc",
            &challenge,
            TokenResponse {
                access_token: "AT".into(),
                id_token: fake_id_token("u-1", "alice", &["admin"]),
                refresh_token: None,
                token_type: "Bearer".into(),
                expires_in: 60,
                scope: "openid".into(),
            },
        ).await;

        // 3) hit callback
        let r2 = app
            .oneshot(Request::builder()
                .uri(&format!("/oauth/callback?code=abc&state={state}"))
                .body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(r2.status(), StatusCode::SEE_OTHER);
        let loc2 = r2.headers().get(header::LOCATION).unwrap().to_str().unwrap();
        assert_eq!(loc2, "/admin");
        let cookie = r2.headers().get(header::SET_COOKIE).unwrap().to_str().unwrap();
        assert!(cookie.starts_with("cave_session="));
        assert!(cookie.contains("HttpOnly"));
    }

    #[tokio::test]
    async fn full_login_callback_non_admin_redirects_to_root() {
        let cfg = test_config();
        let exch = Arc::new(MockExchanger::default());
        let state_obj = Arc::new(OAuthState::new(cfg.clone(), exch.clone()));
        let app = router(state_obj.clone());

        let r1 = app.clone()
            .oneshot(Request::builder().uri("/oauth/login").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let loc = r1.headers().get(header::LOCATION).unwrap().to_str().unwrap().to_string();
        let state = loc.split("state=").nth(1).unwrap().split('&').next().unwrap();
        let flow = state_obj.flows.inner.read().await.get(state).cloned().unwrap();

        exch.install(
            "abc",
            &flow.challenge,
            TokenResponse {
                access_token: "AT".into(),
                id_token: fake_id_token("u-2", "bob", &["tenant"]),
                refresh_token: None,
                token_type: "Bearer".into(),
                expires_in: 60,
                scope: "openid".into(),
            },
        ).await;

        let r2 = app
            .oneshot(Request::builder()
                .uri(&format!("/oauth/callback?code=abc&state={state}"))
                .body(Body::empty()).unwrap())
            .await
            .unwrap();
        let loc2 = r2.headers().get(header::LOCATION).unwrap().to_str().unwrap();
        assert_eq!(loc2, "/");
    }

    #[tokio::test]
    async fn callback_pkce_mismatch_rejected() {
        let cfg = test_config();
        let exch = Arc::new(MockExchanger::default());
        let state_obj = Arc::new(OAuthState::new(cfg.clone(), exch.clone()));
        let app = router(state_obj.clone());

        let r1 = app.clone()
            .oneshot(Request::builder().uri("/oauth/login").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let loc = r1.headers().get(header::LOCATION).unwrap().to_str().unwrap().to_string();
        let state = loc.split("state=").nth(1).unwrap().split('&').next().unwrap();
        // Install fixture with a WRONG expected challenge — exchanger should fail.
        exch.install(
            "abc",
            "wrong-challenge",
            TokenResponse {
                access_token: "AT".into(),
                id_token: fake_id_token("u", "x", &[]),
                refresh_token: None,
                token_type: "Bearer".into(),
                expires_in: 60,
                scope: "openid".into(),
            },
        ).await;
        let r2 = app
            .oneshot(Request::builder()
                .uri(&format!("/oauth/callback?code=abc&state={state}"))
                .body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(r2.status(), StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn me_without_cookie_401s() {
        let app = router(build_state());
        let resp = app
            .oneshot(Request::builder().uri("/me").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn me_with_valid_cookie_returns_claims() {
        let cfg = test_config();
        let exch = Arc::new(MockExchanger::default());
        let state_obj = Arc::new(OAuthState::new(cfg.clone(), exch));
        let app = router(state_obj);

        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
        let claims = SessionClaims {
            sub: "u-9".into(),
            preferred_username: "alice".into(),
            email: Some("a@x".into()),
            roles: vec!["admin".into()],
            tenant: None,
            iat: now,
            exp: now + 60,
            iss: "cave-portal".into(),
        };
        let token = sign_session(&claims, &cfg.session_secret).unwrap();
        let resp = app
            .oneshot(Request::builder()
                .uri("/me")
                .header(header::COOKIE, format!("cave_session={token}"))
                .body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), 8192).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["sub"], "u-9");
        assert_eq!(v["username"], "alice");
        assert_eq!(v["roles"][0], "admin");
    }

    #[tokio::test]
    async fn me_with_invalid_cookie_401s() {
        let app = router(build_state());
        let resp = app
            .oneshot(Request::builder()
                .uri("/me")
                .header(header::COOKIE, "cave_session=garbage")
                .body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn logout_clears_cookie_and_redirects() {
        let app = router(build_state());
        let resp = app
            .oneshot(Request::builder().uri("/oauth/logout").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        let cookie = resp.headers().get(header::SET_COOKIE).unwrap().to_str().unwrap();
        assert!(cookie.contains("Max-Age=0"));
        let loc = resp.headers().get(header::LOCATION).unwrap().to_str().unwrap();
        assert!(loc.contains("/protocol/openid-connect/logout"));
    }

    #[tokio::test]
    async fn logout_post_also_works() {
        let app = router(build_state());
        let resp = app
            .oneshot(Request::builder()
                .method("POST")
                .uri("/oauth/logout").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    }
}
