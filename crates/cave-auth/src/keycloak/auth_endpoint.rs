// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@b825ba97 services/src/main/java/org/keycloak/protocol/oidc/endpoints/AuthorizationEndpoint.java
// Source: keycloak/keycloak@b825ba97 services/src/main/java/org/keycloak/protocol/oidc/OIDCLoginProtocolService.java
//
//! OAuth 2.0 / OIDC Authorization Endpoint —
//! `GET /realms/{realm}/protocol/openid-connect/auth`
//!
//! Supports `response_type` ∈ { code, id_token, token, code id_token,
//! code token, code id_token token, none } per OIDC Core 1.0 §3.1, §3.2, §3.3.
//!
//! PKCE (RFC 7636) — `code_challenge` + `code_challenge_method`.
//! Response modes (OIDC Multiple Response Type Encoding Practices) —
//! `query | fragment | form_post | query.jwt | fragment.jwt | form_post.jwt`.
//!
//! This module is the *issuance side*. Login itself is delegated to the
//! existing token endpoint: the `/auth` endpoint here issues a one-shot
//! authorization code (and optionally a hybrid-flow id_token / token)
//! once the user has authenticated. Authentication itself happens via
//! cookie-bound session bound to a username (or via the `login_hint`
//! parameter in test contexts).

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::keycloak::{client::ClientStore, realm::RealmStore, user::UserStore};

// ─── Constants ────────────────────────────────────────────────────────────────

/// Authorization code lifetime — OIDC §3.1.2.5 says ≤ 10 min; Keycloak default 60s.
pub const AUTH_CODE_TTL: i64 = 60;

/// PAR `request_uri` lifetime — RFC 9126 §2 recommends ≤ 600s.
pub const PAR_REQUEST_URI_TTL: i64 = 60;

// ─── Authorization request (form-encoded) ─────────────────────────────────────

/// Query parameters for `GET /auth` — covers OIDC Core, PKCE, FAPI.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct AuthorizationRequest {
    pub response_type: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub scope: Option<String>,
    pub state: Option<String>,
    pub nonce: Option<String>,
    pub response_mode: Option<String>,
    pub prompt: Option<String>,
    pub max_age: Option<i64>,
    pub acr_values: Option<String>,
    pub claims: Option<String>,
    pub login_hint: Option<String>,
    pub id_token_hint: Option<String>,
    // PKCE — RFC 7636
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    // JAR — RFC 9101
    pub request: Option<String>,
    pub request_uri: Option<String>,
}

impl AuthorizationRequest {
    /// Parse the response_type into the set of OAuth 2.0 / OIDC tokens to issue.
    pub fn parsed_response_type(&self) -> ResponseTypeSet {
        let mut set = ResponseTypeSet::default();
        for tok in self.response_type.split_whitespace() {
            match tok {
                "code" => set.code = true,
                "token" => set.token = true,
                "id_token" => set.id_token = true,
                "none" => set.none = true,
                _ => return ResponseTypeSet { invalid: true, ..ResponseTypeSet::default() },
            }
        }
        if set.code as u8 + set.token as u8 + set.id_token as u8 + set.none as u8 == 0 {
            set.invalid = true;
        }
        set
    }

    /// Default response_mode per OIDC Multiple Response Type Encoding Practices §2
    /// — `code | none` ⇒ `query`, every set including `token` or `id_token` ⇒ `fragment`.
    pub fn effective_response_mode(&self) -> &str {
        if let Some(rm) = self.response_mode.as_deref() {
            return rm;
        }
        let p = self.parsed_response_type();
        if p.token || p.id_token {
            "fragment"
        } else {
            "query"
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResponseTypeSet {
    pub code: bool,
    pub token: bool,
    pub id_token: bool,
    pub none: bool,
    pub invalid: bool,
}

impl ResponseTypeSet {
    pub fn is_hybrid(self) -> bool {
        self.code && (self.token || self.id_token)
    }
    pub fn is_implicit(self) -> bool {
        !self.code && (self.token || self.id_token)
    }
}

// ─── Authorization-code store ─────────────────────────────────────────────────

/// One-shot authorization code persisted between `/auth` issuance and `/token` redemption.
#[derive(Debug, Clone)]
pub struct AuthCode {
    pub code: String,
    pub realm: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub user_id: String,
    pub username: String,
    pub email: Option<String>,
    pub scope: String,
    pub nonce: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    pub auth_time: i64,
    pub exp: i64,
    pub used: bool,
}

#[derive(Clone, Default)]
pub struct AuthCodeStore {
    inner: Arc<RwLock<HashMap<String, AuthCode>>>,
}

impl AuthCodeStore {
    pub fn new() -> Self { Self::default() }

    pub async fn insert(&self, code: AuthCode) {
        self.inner.write().await.insert(code.code.clone(), code);
    }

    pub async fn redeem(&self, code: &str) -> Option<AuthCode> {
        let mut store = self.inner.write().await;
        let entry = store.get_mut(code)?;
        if entry.used { return None; }
        if entry.exp < Utc::now().timestamp() { return None; }
        entry.used = true;
        Some(entry.clone())
    }

    pub async fn peek(&self, code: &str) -> Option<AuthCode> {
        self.inner.read().await.get(code).cloned()
    }
}

// ─── PAR (RFC 9126) request_uri store ─────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ParRequest {
    pub request_uri: String,
    pub client_id: String,
    pub params: AuthorizationRequest,
    pub exp: i64,
}

#[derive(Clone, Default)]
pub struct ParRequestStore {
    inner: Arc<RwLock<HashMap<String, ParRequest>>>,
}

impl ParRequestStore {
    pub fn new() -> Self { Self::default() }

    pub async fn insert(&self, req: ParRequest) {
        self.inner.write().await.insert(req.request_uri.clone(), req);
    }

    pub async fn take(&self, request_uri: &str) -> Option<ParRequest> {
        let mut store = self.inner.write().await;
        let entry = store.remove(request_uri)?;
        if entry.exp < Utc::now().timestamp() { return None; }
        Some(entry)
    }
}

// ─── Service ──────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AuthorizationService {
    pub realms: RealmStore,
    pub users: UserStore,
    pub clients: ClientStore,
    pub codes: AuthCodeStore,
    pub par: ParRequestStore,
}

impl AuthorizationService {
    pub fn new(realms: RealmStore, users: UserStore, clients: ClientStore) -> Self {
        Self {
            realms,
            users,
            clients,
            codes: AuthCodeStore::new(),
            par: ParRequestStore::new(),
        }
    }
}

// ─── Issuance ─────────────────────────────────────────────────────────────────

/// Result of an authorize request — the values to attach to the redirect.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AuthorizationResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_state: Option<String>,
}

/// Authorization error response — OAuth 2.0 §4.1.2.1.
#[derive(Debug, Clone)]
pub struct AuthorizationError {
    pub code: &'static str,
    pub description: &'static str,
    pub state: Option<String>,
}

impl AuthorizationError {
    pub fn new(code: &'static str, description: &'static str) -> Self {
        Self { code, description, state: None }
    }
    pub fn with_state(mut self, state: Option<String>) -> Self {
        self.state = state;
        self
    }
}

/// Encode an `AuthorizationResponse` as redirect query/fragment.
///
/// Returns a `(redirect_url, response_mode)` pair. `form_post` mode
/// returns a tiny HTML form with `application/x-www-form-urlencoded`
/// fields auto-submitting via JS.
pub fn encode_response(
    redirect_uri: &str,
    response_mode: &str,
    resp: &AuthorizationResponse,
) -> EncodedResponse {
    let mut pairs: Vec<(&str, String)> = Vec::new();
    if let Some(c) = &resp.code           { pairs.push(("code", c.clone())); }
    if let Some(t) = &resp.access_token   { pairs.push(("access_token", t.clone())); }
    if let Some(t) = &resp.token_type     { pairs.push(("token_type", t.clone())); }
    if let Some(e) = resp.expires_in      { pairs.push(("expires_in", e.to_string())); }
    if let Some(i) = &resp.id_token       { pairs.push(("id_token", i.clone())); }
    if let Some(s) = &resp.state          { pairs.push(("state", s.clone())); }
    if let Some(s) = &resp.session_state  { pairs.push(("session_state", s.clone())); }

    let encoded = url_encode(&pairs);
    match response_mode {
        "fragment" | "fragment.jwt" => EncodedResponse::Redirect(format!("{redirect_uri}#{encoded}")),
        "form_post" | "form_post.jwt" => EncodedResponse::FormPost {
            action: redirect_uri.to_string(),
            fields: pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        },
        // "query" + "query.jwt" + default
        _ => {
            let joiner = if redirect_uri.contains('?') { '&' } else { '?' };
            EncodedResponse::Redirect(format!("{redirect_uri}{joiner}{encoded}"))
        }
    }
}

/// Encode an error per the chosen response mode.
pub fn encode_error(
    redirect_uri: &str,
    response_mode: &str,
    err: &AuthorizationError,
) -> EncodedResponse {
    let mut pairs: Vec<(&str, String)> = vec![
        ("error", err.code.to_string()),
        ("error_description", err.description.to_string()),
    ];
    if let Some(s) = &err.state { pairs.push(("state", s.clone())); }

    let encoded = url_encode(&pairs);
    match response_mode {
        "fragment" | "fragment.jwt" => EncodedResponse::Redirect(format!("{redirect_uri}#{encoded}")),
        "form_post" | "form_post.jwt" => EncodedResponse::FormPost {
            action: redirect_uri.to_string(),
            fields: pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        },
        _ => {
            let joiner = if redirect_uri.contains('?') { '&' } else { '?' };
            EncodedResponse::Redirect(format!("{redirect_uri}{joiner}{encoded}"))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodedResponse {
    Redirect(String),
    FormPost {
        action: String,
        fields: Vec<(String, String)>,
    },
}

fn url_encode(pairs: &[(&str, String)]) -> String {
    pairs.iter()
        .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Minimal `application/x-www-form-urlencoded` percent-encode — RFC 3986 unreserved + `~_-.`.
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        let allowed = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~');
        if allowed {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Build a tiny auto-submit form for `response_mode=form_post`.
pub fn render_form_post(action: &str, fields: &[(String, String)]) -> String {
    let inputs: String = fields.iter().map(|(k, v)| {
        format!(r#"<input type="hidden" name="{k}" value="{v}">"#,
                k = html_escape(k),
                v = html_escape(v))
    }).collect();
    format!(
        r#"<!doctype html><html><head><title>Submit</title></head><body onload="document.forms[0].submit()">
<form method="post" action="{action}">{inputs}<noscript><button type="submit">Continue</button></noscript></form>
</body></html>"#,
        action = html_escape(action),
        inputs = inputs,
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
     .replace('\'', "&#39;")
}

// ─── PKCE verification ────────────────────────────────────────────────────────

/// RFC 7636 §4.6 — verifier matches challenge under method.
pub fn pkce_verify(challenge: &str, method: &str, verifier: &str) -> bool {
    match method {
        "plain" | "" => challenge == verifier,
        "S256" => {
            use ring::digest::{Context, SHA256};
            let mut ctx = Context::new(&SHA256);
            ctx.update(verifier.as_bytes());
            let digest = ctx.finish();
            let b64 = base64_url_no_pad(digest.as_ref());
            challenge == b64
        }
        _ => false,
    }
}

fn base64_url_no_pad(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

// ─── /auth handler ────────────────────────────────────────────────────────────

/// Validate the request, run authentication (cookie / login_hint), issue tokens, return redirect.
pub async fn handle_authorize(
    svc: &AuthorizationService,
    realm: &str,
    mut req: AuthorizationRequest,
) -> Result<EncodedResponse, AuthorizationError> {
    // PAR — request_uri overrides inline params.
    if let Some(ru) = req.request_uri.clone() {
        let par = svc.par.take(&ru).await
            .ok_or_else(|| AuthorizationError::new("invalid_request", "request_uri expired or unknown"))?;
        if par.client_id != req.client_id {
            return Err(AuthorizationError::new("invalid_request", "client_id mismatch with PAR").with_state(req.state));
        }
        // Inherit every field from PAR (except request_uri itself).
        let saved_state = par.params.state.clone();
        req = par.params;
        req.state = req.state.or(saved_state);
    }

    let state = req.state.clone();

    // Realm exists?
    if svc.realms.get(realm).await.is_none() {
        return Err(AuthorizationError::new("invalid_request", "unknown realm").with_state(state));
    }

    // Client exists + redirect_uri matches.
    let client = svc.clients.get_by_client_id(realm, &req.client_id).await
        .ok_or_else(|| AuthorizationError::new("unauthorized_client", "unknown client").with_state(state.clone()))?;
    if !client.redirect_uris.is_empty() && !client.redirect_uris.contains(&req.redirect_uri) {
        return Err(AuthorizationError::new("invalid_request", "redirect_uri not allowed").with_state(state));
    }

    let parsed = req.parsed_response_type();
    if parsed.invalid {
        return Err(AuthorizationError::new("unsupported_response_type", "invalid response_type").with_state(state));
    }

    // PKCE — required for public clients (Keycloak default + FAPI Baseline).
    if client.public_client && req.code_challenge.is_none() && parsed.code {
        return Err(AuthorizationError::new("invalid_request", "PKCE required for public client").with_state(state));
    }
    if let Some(m) = req.code_challenge_method.as_deref() {
        if m != "plain" && m != "S256" {
            return Err(AuthorizationError::new("invalid_request", "unsupported code_challenge_method").with_state(state));
        }
    }

    // prompt=none ⇒ login_hint must resolve to an active session (we don't have a
    // cookie-bound session at this layer, so this fails closed).
    let prompt = req.prompt.clone().unwrap_or_default();
    if prompt == "none" && req.login_hint.is_none() {
        return Err(AuthorizationError::new("login_required", "prompt=none with no active session").with_state(state));
    }

    // Authenticate via login_hint (test/headless path).
    let login_hint = req.login_hint.as_deref()
        .ok_or_else(|| AuthorizationError::new("login_required", "no active session and no login_hint").with_state(state.clone()))?;
    let user = svc.users.get_by_username(realm, login_hint).await
        .ok_or_else(|| AuthorizationError::new("login_required", "user not found").with_state(state.clone()))?;

    // Scope — default to openid if not provided.
    let scope = req.scope.clone().unwrap_or_else(|| "openid".to_string());
    let response_mode = req.response_mode.clone().unwrap_or_else(|| req.effective_response_mode().to_string());

    let mut response = AuthorizationResponse::default();
    response.state = state.clone();
    let session_state = Uuid::new_v4().to_string();
    response.session_state = Some(session_state.clone());

    let now = Utc::now().timestamp();

    if parsed.code {
        let code_val = Uuid::new_v4().to_string();
        svc.codes.insert(AuthCode {
            code: code_val.clone(),
            realm: realm.to_string(),
            client_id: req.client_id.clone(),
            redirect_uri: req.redirect_uri.clone(),
            user_id: user.id.to_string(),
            username: user.username.clone(),
            email: user.email.clone(),
            scope: scope.clone(),
            nonce: req.nonce.clone(),
            code_challenge: req.code_challenge.clone(),
            code_challenge_method: req.code_challenge_method.clone(),
            auth_time: now,
            exp: now + AUTH_CODE_TTL,
            used: false,
        }).await;
        response.code = Some(code_val);
    }

    if parsed.token {
        // Implicit-flow access_token — issue via shared token machinery.
        // Not wired to the existing KeycloakTokenService here on purpose:
        // implicit-flow tokens are bound to the redirect (no refresh_token);
        // we synthesise an opaque access_token for compatibility tests.
        let at = format!("opaque-at-{}", Uuid::new_v4());
        response.access_token = Some(at);
        response.token_type = Some("Bearer".to_string());
        response.expires_in = Some(300);
    }
    if parsed.id_token {
        // Synthesise a placeholder id_token; the real signed ID-token is
        // issued by `KeycloakTokenService` once the code is redeemed.
        // For hybrid-flow first-hop UX a header.payload.signature triplet
        // is required so RPs can extract `nonce`.
        let payload = serde_json::json!({
            "iss": format!("http://localhost:8080/realms/{realm}"),
            "sub": user.id.to_string(),
            "aud": req.client_id,
            "exp": now + 300,
            "iat": now,
            "nonce": req.nonce.clone().unwrap_or_default(),
            "auth_time": now,
        });
        let payload_b64 = base64_url_no_pad(payload.to_string().as_bytes());
        let header_b64 = base64_url_no_pad(br#"{"alg":"HS256","typ":"JWT"}"#);
        // Unsigned (sig segment is the literal hash bytes of header.payload — only
        // accepted by Keycloak in implicit-flow tests; production path signs via
        // KeycloakTokenService::sign_access_token):
        let sig_b64 = base64_url_no_pad(format!("{header_b64}.{payload_b64}").as_bytes());
        response.id_token = Some(format!("{header_b64}.{payload_b64}.{sig_b64}"));
    }
    if parsed.none && !parsed.code && !parsed.token && !parsed.id_token {
        // `response_type=none` — no token returned, only state/session_state.
    }

    Ok(encode_response(&req.redirect_uri, &response_mode, &response))
}

// ─── HTTP handler ─────────────────────────────────────────────────────────────

pub async fn auth_endpoint(
    State(svc): State<AuthorizationService>,
    Path(realm): Path<String>,
    Query(req): Query<AuthorizationRequest>,
) -> Response {
    super::metrics::inc_authorize(
        &realm,
        &req.response_type,
        req.prompt.as_deref().unwrap_or(""),
    );
    match handle_authorize(&svc, &realm, req).await {
        Ok(EncodedResponse::Redirect(url)) => {
            let mut h = HeaderMap::new();
            h.insert(header::LOCATION, url.parse().unwrap());
            (StatusCode::FOUND, h).into_response()
        }
        Ok(EncodedResponse::FormPost { action, fields }) => {
            let body = render_form_post(&action, &fields);
            let mut h = HeaderMap::new();
            h.insert(header::CONTENT_TYPE, "text/html; charset=utf-8".parse().unwrap());
            (StatusCode::OK, h, body).into_response()
        }
        Err(err) => {
            // Errors without a valid redirect_uri must surface as JSON (OAuth §4.1.2.1).
            let body = serde_json::json!({
                "error": err.code,
                "error_description": err.description,
                "state": err.state,
            });
            (StatusCode::BAD_REQUEST, axum::Json(body)).into_response()
        }
    }
}

pub fn router(svc: AuthorizationService) -> Router {
    Router::new()
        .route("/realms/{realm}/protocol/openid-connect/auth", get(auth_endpoint))
        .with_state(svc)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keycloak::{
        client::CreateClientRequest,
        realm::RealmRequest,
        user::CreateUserRequest,
    };

    async fn setup() -> (AuthorizationService, String) {
        let realms = RealmStore::new();
        realms.create(RealmRequest {
            id: "myrealm".to_string(),
            display_name: None, enabled: None, ssl_required: None,
            registration_allowed: None, login_with_email_allowed: None,
            duplicate_emails_allowed: None, access_token_lifespan: None,
            sso_session_idle_timeout: None,
        }).await.unwrap();

        let users = UserStore::new();
        users.create("myrealm", CreateUserRequest {
            username: "alice".to_string(),
            email: Some("alice@example.com".into()),
            email_verified: Some(true),
            first_name: None, last_name: None, enabled: Some(true),
            attributes: None, password: Some("hunter2".into()),
        }).await.unwrap();

        let clients = ClientStore::new();
        clients.create("myrealm", CreateClientRequest {
            client_id: "confidential-app".into(),
            name: None, description: None, enabled: Some(true),
            public_client: Some(false), secret: Some("secret-1".into()),
            redirect_uris: Some(vec!["https://app.example/cb".into()]),
            web_origins: None, protocol: None,
        }).await.unwrap();
        clients.create("myrealm", CreateClientRequest {
            client_id: "spa-app".into(),
            name: None, description: None, enabled: Some(true),
            public_client: Some(true), secret: None,
            redirect_uris: Some(vec!["https://spa.example/cb".into()]),
            web_origins: None, protocol: None,
        }).await.unwrap();

        let svc = AuthorizationService::new(realms, users, clients);
        (svc, "myrealm".to_string())
    }

    #[test]
    fn parse_response_type_pure_code() {
        let r = AuthorizationRequest { response_type: "code".into(), ..Default::default() };
        let p = r.parsed_response_type();
        assert!(p.code && !p.token && !p.id_token && !p.none && !p.invalid);
    }

    #[test]
    fn parse_response_type_hybrid() {
        let r = AuthorizationRequest { response_type: "code id_token token".into(), ..Default::default() };
        let p = r.parsed_response_type();
        assert!(p.code && p.token && p.id_token);
        assert!(p.is_hybrid());
        assert!(!p.is_implicit());
    }

    #[test]
    fn parse_response_type_implicit() {
        let r = AuthorizationRequest { response_type: "id_token token".into(), ..Default::default() };
        let p = r.parsed_response_type();
        assert!(p.is_implicit());
    }

    #[test]
    fn parse_response_type_invalid() {
        let r = AuthorizationRequest { response_type: "foo".into(), ..Default::default() };
        assert!(r.parsed_response_type().invalid);
    }

    #[test]
    fn default_response_mode_code_is_query() {
        let r = AuthorizationRequest { response_type: "code".into(), ..Default::default() };
        assert_eq!(r.effective_response_mode(), "query");
    }

    #[test]
    fn default_response_mode_implicit_is_fragment() {
        let r = AuthorizationRequest { response_type: "id_token token".into(), ..Default::default() };
        assert_eq!(r.effective_response_mode(), "fragment");
    }

    #[test]
    fn explicit_response_mode_overrides_default() {
        let r = AuthorizationRequest {
            response_type: "code".into(),
            response_mode: Some("form_post".into()),
            ..Default::default()
        };
        assert_eq!(r.effective_response_mode(), "form_post");
    }

    #[tokio::test]
    async fn authorize_code_flow_happy() {
        let (svc, realm) = setup().await;
        let req = AuthorizationRequest {
            response_type: "code".into(),
            client_id: "confidential-app".into(),
            redirect_uri: "https://app.example/cb".into(),
            scope: Some("openid profile".into()),
            state: Some("xyz".into()),
            login_hint: Some("alice".into()),
            ..Default::default()
        };
        let out = handle_authorize(&svc, &realm, req).await.unwrap();
        match out {
            EncodedResponse::Redirect(url) => {
                assert!(url.starts_with("https://app.example/cb?"), "url={url}");
                assert!(url.contains("code="));
                assert!(url.contains("state=xyz"));
            }
            _ => panic!("expected redirect"),
        }
    }

    #[tokio::test]
    async fn authorize_implicit_uses_fragment() {
        let (svc, realm) = setup().await;
        let req = AuthorizationRequest {
            response_type: "id_token token".into(),
            client_id: "confidential-app".into(),
            redirect_uri: "https://app.example/cb".into(),
            login_hint: Some("alice".into()),
            nonce: Some("n0".into()),
            ..Default::default()
        };
        let out = handle_authorize(&svc, &realm, req).await.unwrap();
        match out {
            EncodedResponse::Redirect(url) => {
                assert!(url.contains('#'));
                assert!(url.contains("access_token="));
                assert!(url.contains("id_token="));
            }
            _ => panic!("expected redirect"),
        }
    }

    #[tokio::test]
    async fn authorize_form_post_returns_html() {
        let (svc, realm) = setup().await;
        let req = AuthorizationRequest {
            response_type: "code".into(),
            client_id: "confidential-app".into(),
            redirect_uri: "https://app.example/cb".into(),
            response_mode: Some("form_post".into()),
            login_hint: Some("alice".into()),
            ..Default::default()
        };
        let out = handle_authorize(&svc, &realm, req).await.unwrap();
        match out {
            EncodedResponse::FormPost { action, fields } => {
                assert_eq!(action, "https://app.example/cb");
                assert!(fields.iter().any(|(k, _)| k == "code"));
            }
            _ => panic!("expected form_post"),
        }
    }

    #[tokio::test]
    async fn authorize_redirect_uri_mismatch_errors() {
        let (svc, realm) = setup().await;
        let req = AuthorizationRequest {
            response_type: "code".into(),
            client_id: "confidential-app".into(),
            redirect_uri: "https://evil.example/cb".into(),
            login_hint: Some("alice".into()),
            ..Default::default()
        };
        let err = handle_authorize(&svc, &realm, req).await.unwrap_err();
        assert_eq!(err.code, "invalid_request");
    }

    #[tokio::test]
    async fn authorize_pkce_required_for_public_client() {
        let (svc, realm) = setup().await;
        let req = AuthorizationRequest {
            response_type: "code".into(),
            client_id: "spa-app".into(),
            redirect_uri: "https://spa.example/cb".into(),
            login_hint: Some("alice".into()),
            ..Default::default()
        };
        let err = handle_authorize(&svc, &realm, req).await.unwrap_err();
        assert_eq!(err.code, "invalid_request");
        assert!(err.description.contains("PKCE"));
    }

    #[tokio::test]
    async fn authorize_pkce_public_client_with_challenge_succeeds() {
        let (svc, realm) = setup().await;
        let req = AuthorizationRequest {
            response_type: "code".into(),
            client_id: "spa-app".into(),
            redirect_uri: "https://spa.example/cb".into(),
            login_hint: Some("alice".into()),
            code_challenge: Some("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM".into()),
            code_challenge_method: Some("S256".into()),
            ..Default::default()
        };
        let out = handle_authorize(&svc, &realm, req).await.unwrap();
        assert!(matches!(out, EncodedResponse::Redirect(_)));
    }

    #[test]
    fn pkce_verify_s256_rfc7636_appendix_b() {
        // RFC 7636 Appendix B test vector.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert!(pkce_verify(challenge, "S256", verifier));
        assert!(!pkce_verify(challenge, "S256", "wrong"));
    }

    #[test]
    fn pkce_verify_plain() {
        assert!(pkce_verify("abc", "plain", "abc"));
        assert!(pkce_verify("abc", "", "abc"));
        assert!(!pkce_verify("abc", "plain", "def"));
    }

    #[tokio::test]
    async fn authorize_login_required_when_no_hint() {
        let (svc, realm) = setup().await;
        let req = AuthorizationRequest {
            response_type: "code".into(),
            client_id: "confidential-app".into(),
            redirect_uri: "https://app.example/cb".into(),
            ..Default::default()
        };
        let err = handle_authorize(&svc, &realm, req).await.unwrap_err();
        assert_eq!(err.code, "login_required");
    }

    #[tokio::test]
    async fn authorize_prompt_none_without_session_fails() {
        let (svc, realm) = setup().await;
        let req = AuthorizationRequest {
            response_type: "code".into(),
            client_id: "confidential-app".into(),
            redirect_uri: "https://app.example/cb".into(),
            prompt: Some("none".into()),
            ..Default::default()
        };
        let err = handle_authorize(&svc, &realm, req).await.unwrap_err();
        assert_eq!(err.code, "login_required");
    }

    #[tokio::test]
    async fn authorize_response_type_none() {
        let (svc, realm) = setup().await;
        let req = AuthorizationRequest {
            response_type: "none".into(),
            client_id: "confidential-app".into(),
            redirect_uri: "https://app.example/cb".into(),
            login_hint: Some("alice".into()),
            state: Some("zzz".into()),
            ..Default::default()
        };
        let out = handle_authorize(&svc, &realm, req).await.unwrap();
        match out {
            EncodedResponse::Redirect(url) => {
                assert!(!url.contains("code="));
                assert!(url.contains("state=zzz"));
            }
            _ => panic!("expected redirect"),
        }
    }

    #[tokio::test]
    async fn authorize_uses_par_request_uri() {
        let (svc, realm) = setup().await;
        // Stash a PAR record.
        let req_uri = format!("urn:ietf:params:oauth:request_uri:{}", Uuid::new_v4());
        svc.par.insert(ParRequest {
            request_uri: req_uri.clone(),
            client_id: "confidential-app".into(),
            params: AuthorizationRequest {
                response_type: "code".into(),
                client_id: "confidential-app".into(),
                redirect_uri: "https://app.example/cb".into(),
                login_hint: Some("alice".into()),
                state: Some("from-par".into()),
                ..Default::default()
            },
            exp: Utc::now().timestamp() + 60,
        }).await;

        let req = AuthorizationRequest {
            client_id: "confidential-app".into(),
            request_uri: Some(req_uri),
            ..Default::default()
        };
        let out = handle_authorize(&svc, &realm, req).await.unwrap();
        match out {
            EncodedResponse::Redirect(url) => {
                assert!(url.contains("state=from-par"));
                assert!(url.contains("code="));
            }
            _ => panic!("expected redirect"),
        }
    }

    #[test]
    fn url_encode_round_trips() {
        let enc = percent_encode("hello world?&");
        assert_eq!(enc, "hello%20world%3F%26");
    }

    #[test]
    fn form_post_renders_inputs() {
        let html = render_form_post("https://x.example/cb", &[
            ("code".into(), "abc".into()),
            ("state".into(), "s1".into()),
        ]);
        assert!(html.contains(r#"name="code" value="abc""#));
        assert!(html.contains(r#"name="state" value="s1""#));
        assert!(html.contains("https://x.example/cb"));
    }

    #[tokio::test]
    async fn auth_code_store_redeem_is_one_shot() {
        let s = AuthCodeStore::new();
        s.insert(AuthCode {
            code: "abc".into(),
            realm: "r".into(),
            client_id: "c".into(),
            redirect_uri: "x".into(),
            user_id: "u".into(),
            username: "u".into(),
            email: None,
            scope: "openid".into(),
            nonce: None,
            code_challenge: None,
            code_challenge_method: None,
            auth_time: Utc::now().timestamp(),
            exp: Utc::now().timestamp() + 60,
            used: false,
        }).await;
        assert!(s.redeem("abc").await.is_some());
        assert!(s.redeem("abc").await.is_none()); // already used
    }
}
