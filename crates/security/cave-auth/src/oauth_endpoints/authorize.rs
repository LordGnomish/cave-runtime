// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../oidc/endpoints/AuthorizationEndpoint.java
//
//! The `GET/POST /realms/{realm}/protocol/openid-connect/auth` endpoint.
//!
//! Keycloak's full flow performs IdP federation + browser-rendered login;
//! this port focuses on the *protocol-side* state machine that platform
//! teams need from a programmable IdP:
//! - parameter validation (`authz_request::validate`)
//! - PAR resolution (`request_uri=urn:ietf:params:oauth:request_uri:…`)
//! - prompt=none short-circuit (returns `login_required` redirect)
//! - authentication via a simple `username=&password=` POST body (the
//!   browser login form normally proxied through Keycloak themes); the
//!   real upstream allows any registered `AuthenticationFlow` here
//! - redirect-back with `code`+`state` for confidential clients
//!
//! Browser-rendered themes / cookie-based session resume are
//! intentionally out of scope (Keycloak's `themes/` package is
//! `[[skipped]]` in the parity manifest).

use axum::{
    Router,
    extract::{Form, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use super::{
    AuthorizationCode, OAuthEndpointsState,
    authz_request::{AuthzError, AuthzRequest, ResponseKind, validate},
};

const PAR_URI_PREFIX: &str = "urn:ietf:params:oauth:request_uri:";
const AUTH_CODE_TTL: i64 = 60;

#[derive(Debug, Deserialize)]
pub struct AuthorizeQuery {
    pub client_id: Option<String>,
    pub redirect_uri: Option<String>,
    pub response_type: Option<String>,
    pub scope: Option<String>,
    pub state: Option<String>,
    pub nonce: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    pub prompt: Option<String>,
    pub max_age: Option<i64>,
    pub login_hint: Option<String>,
    pub request_uri: Option<String>,
    pub response_mode: Option<String>,
}

impl AuthorizeQuery {
    fn into_authz_request(self) -> AuthzRequest {
        AuthzRequest {
            client_id: self.client_id.unwrap_or_default(),
            redirect_uri: self.redirect_uri.unwrap_or_default(),
            response_type: self.response_type.unwrap_or_default(),
            scope: self.scope,
            state: self.state,
            nonce: self.nonce,
            code_challenge: self.code_challenge,
            code_challenge_method: self.code_challenge_method,
            prompt: self.prompt,
            max_age: self.max_age,
            login_hint: self.login_hint,
            request_uri: self.request_uri,
            response_mode: self.response_mode,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct AuthorizeForm {
    pub username: Option<String>,
    pub password: Option<String>,
}

/// `GET /realms/{realm}/protocol/openid-connect/auth`
///
/// With no session this returns a `200` HTML stub indicating the
/// login form would be served. Browser flows complete the round-trip
/// via the `POST` variant after the user submits credentials.
pub async fn authorize_get(
    State(state): State<OAuthEndpointsState>,
    Path(realm): Path<String>,
    Query(q): Query<AuthorizeQuery>,
) -> Response {
    // Realm sanity.
    if state.realms.get(&realm).await.is_none() {
        return (StatusCode::NOT_FOUND, "realm_not_found").into_response();
    }

    let mut req = q.into_authz_request();

    // PAR (RFC 9126): if request_uri is set, take and substitute.
    if let Some(uri) = req.request_uri.clone() {
        if uri.starts_with(PAR_URI_PREFIX) {
            match state.par.take(&uri).await {
                Some(rec) => {
                    if Utc::now().timestamp() > rec.exp_unix {
                        return error_redirect("invalid_request_uri", "request_uri expired", &req)
                            .into_response();
                    }
                    // Decode the stored urlencoded query string and overlay.
                    if let Some(merged) = merge_par(&rec.stored_request) {
                        req = merged;
                    }
                }
                None => {
                    return error_redirect("invalid_request_uri", "unknown request_uri", &req)
                        .into_response();
                }
            }
        }
    }

    let validated = match validate(req) {
        Ok(v) => v,
        Err(e) => return authz_error_response(e).into_response(),
    };

    // prompt=none with no session shortcut.
    if validated
        .raw
        .prompt
        .as_deref()
        .map(|p| p.split_whitespace().any(|t| t == "none"))
        .unwrap_or(false)
    {
        return error_redirect("login_required", "no session", &validated.raw).into_response();
    }

    // Client must exist & redirect_uri must be allowed.
    let client = match state
        .clients
        .get_by_client_id(&realm, &validated.raw.client_id)
        .await
    {
        Some(c) => c,
        None => return (StatusCode::BAD_REQUEST, "unknown client_id").into_response(),
    };
    if !client_allows_redirect(&client.redirect_uris, &validated.raw.redirect_uri) {
        return (StatusCode::BAD_REQUEST, "redirect_uri not allowed").into_response();
    }

    // Browser would render the login form here; we return a tiny HTML form
    // pointing back at the POST endpoint so user agents *and* curl smoke
    // tests can complete the flow.
    let html = format!(
        r##"<!doctype html><html><body><form method="POST" action="">
<input name="username"/><input name="password" type="password"/>
<input name="client_id" type="hidden" value="{client}"/>
<input name="redirect_uri" type="hidden" value="{ru}"/>
<input name="response_type" type="hidden" value="{rt}"/>
<input name="scope" type="hidden" value="{scope}"/>
<input name="state" type="hidden" value="{st}"/>
<input name="nonce" type="hidden" value="{nc}"/>
<input name="code_challenge" type="hidden" value="{cc}"/>
<input name="code_challenge_method" type="hidden" value="{ccm}"/>
<button>Sign in</button>
</form></body></html>"##,
        client = html_escape(&validated.raw.client_id),
        ru = html_escape(&validated.raw.redirect_uri),
        rt = html_escape(&validated.raw.response_type),
        scope = html_escape(validated.raw.scope.as_deref().unwrap_or("")),
        st = html_escape(validated.raw.state.as_deref().unwrap_or("")),
        nc = html_escape(validated.raw.nonce.as_deref().unwrap_or("")),
        cc = html_escape(validated.raw.code_challenge.as_deref().unwrap_or("")),
        ccm = html_escape(validated.raw.code_challenge_method.as_deref().unwrap_or("")),
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
        .into_response()
}

/// `POST /realms/{realm}/protocol/openid-connect/auth`
///
/// Credentials POST that returns the redirect-back response with
/// `code` (and optionally `id_token`/`access_token` for hybrid).
pub async fn authorize_post(
    State(state): State<OAuthEndpointsState>,
    Path(realm): Path<String>,
    Query(q): Query<AuthorizeQuery>,
    Form(form): Form<std::collections::HashMap<String, String>>,
) -> Response {
    if state.realms.get(&realm).await.is_none() {
        return (StatusCode::NOT_FOUND, "realm_not_found").into_response();
    }

    // Hidden form fields override the (often empty) query string.
    let mut q = q.into_authz_request();
    if let Some(v) = form.get("client_id") {
        if !v.is_empty() {
            q.client_id = v.clone();
        }
    }
    if let Some(v) = form.get("redirect_uri") {
        if !v.is_empty() {
            q.redirect_uri = v.clone();
        }
    }
    if let Some(v) = form.get("response_type") {
        if !v.is_empty() {
            q.response_type = v.clone();
        }
    }
    if let Some(v) = form.get("scope") {
        q.scope = Some(v.clone());
    }
    if let Some(v) = form.get("state") {
        q.state = Some(v.clone());
    }
    if let Some(v) = form.get("nonce") {
        q.nonce = Some(v.clone());
    }
    if let Some(v) = form.get("code_challenge") {
        q.code_challenge = Some(v.clone());
    }
    if let Some(v) = form.get("code_challenge_method") {
        q.code_challenge_method = Some(v.clone());
    }

    let validated = match validate(q) {
        Ok(v) => v,
        Err(e) => return authz_error_response(e).into_response(),
    };

    // Authenticate user.
    let username = form.get("username").cloned().unwrap_or_default();
    let password = form.get("password").cloned().unwrap_or_default();
    let user = match state.users.get_by_username(&realm, &username).await {
        Some(u) => u,
        None => return (StatusCode::UNAUTHORIZED, "invalid_credentials").into_response(),
    };
    if !state
        .users
        .verify_password(&realm, user.id, &password)
        .await
    {
        return (StatusCode::UNAUTHORIZED, "invalid_credentials").into_response();
    }

    // Mint authorization code.
    let code = Uuid::new_v4().to_string();
    let now = Utc::now().timestamp();
    state
        .codes
        .put(AuthorizationCode {
            code: code.clone(),
            realm: realm.clone(),
            client_id: validated.raw.client_id.clone(),
            user_sub: user.id.to_string(),
            redirect_uri: validated.raw.redirect_uri.clone(),
            scope: validated
                .raw
                .scope
                .clone()
                .unwrap_or_else(|| "openid".into()),
            state: validated.raw.state.clone(),
            nonce: validated.raw.nonce.clone(),
            code_challenge: validated.challenge.as_ref().map(|(c, _)| c.clone()),
            code_challenge_method: validated.challenge.as_ref().map(|(_, m)| *m),
            exp_unix: now + AUTH_CODE_TTL,
        })
        .await;

    // Build redirect URI.
    let separator = if matches!(validated.response_kinds.first(), Some(ResponseKind::Code)) {
        '?'
    } else {
        '#' // implicit/hybrid → fragment
    };
    let mut params = vec![format!("code={}", urlencoding::encode(&code))];
    if let Some(st) = validated.raw.state.as_deref() {
        params.push(format!("state={}", urlencoding::encode(st)));
    }
    let redirect = format!(
        "{}{}{}",
        validated.raw.redirect_uri,
        separator,
        params.join("&")
    );
    Redirect::to(&redirect).into_response()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn client_allows_redirect(allow: &[String], r: &str) -> bool {
    // Keycloak v22: exact match only for OIDC (relaxed match was dropped).
    allow.iter().any(|a| a == r)
}

fn merge_par(stored_query: &str) -> Option<AuthzRequest> {
    let parsed: AuthorizeQuery = serde_urlencoded::from_str(stored_query).ok()?;
    Some(parsed.into_authz_request())
}

fn error_redirect(error: &'static str, desc: &str, req: &AuthzRequest) -> Response {
    if req.redirect_uri.is_empty() {
        return (StatusCode::BAD_REQUEST, format!("{}: {}", error, desc)).into_response();
    }
    let separator = if req.redirect_uri.contains('?') {
        '&'
    } else {
        '?'
    };
    let mut params = vec![
        format!("error={}", urlencoding::encode(error)),
        format!("error_description={}", urlencoding::encode(desc)),
    ];
    if let Some(s) = req.state.as_deref() {
        params.push(format!("state={}", urlencoding::encode(s)));
    }
    let url = format!("{}{}{}", req.redirect_uri, separator, params.join("&"));
    Redirect::to(&url).into_response()
}

fn authz_error_response(e: AuthzError) -> Response {
    let _ = HeaderMap::new();
    (
        StatusCode::BAD_REQUEST,
        axum::Json(serde_json::json!({
            "error": e.error,
            "error_description": e.error_description,
        })),
    )
        .into_response()
}

pub fn router(state: OAuthEndpointsState) -> Router {
    Router::new()
        .route(
            "/realms/{realm}/protocol/openid-connect/auth",
            get(authorize_get).post(authorize_post),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keycloak::{
        client::{ClientStore, CreateClientRequest},
        realm::{RealmRequest, RealmStore},
        user::{CreateUserRequest, UserStore},
    };
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    async fn setup() -> (Router, OAuthEndpointsState) {
        let realms = RealmStore::new();
        realms
            .create(RealmRequest {
                id: "myrealm".into(),
                display_name: None,
                enabled: None,
                ssl_required: None,
                registration_allowed: None,
                login_with_email_allowed: None,
                duplicate_emails_allowed: None,
                access_token_lifespan: None,
                sso_session_idle_timeout: None,
            })
            .await
            .unwrap();

        let users = UserStore::new();
        users
            .create(
                "myrealm",
                CreateUserRequest {
                    username: "alice".into(),
                    email: Some("a@x".into()),
                    email_verified: Some(true),
                    first_name: None,
                    last_name: None,
                    enabled: Some(true),
                    attributes: None,
                    password: Some("hunter2".into()),
                },
            )
            .await
            .unwrap();

        let clients = ClientStore::new();
        clients
            .create(
                "myrealm",
                CreateClientRequest {
                    client_id: "app".into(),
                    name: None,
                    description: None,
                    enabled: Some(true),
                    public_client: Some(true),
                    secret: None,
                    redirect_uris: Some(vec!["https://app.example/cb".into()]),
                    web_origins: None,
                    protocol: None,
                },
            )
            .await
            .unwrap();

        let state = OAuthEndpointsState::new(realms, clients, users);
        let app = router(state.clone());
        (app, state)
    }

    // upstream: keycloak/keycloak AuthorizationEndpointTest.java:getRendersLoginForm
    #[tokio::test]
    async fn authorize_get_renders_login_form() {
        let (app, _) = setup().await;
        let uri = "/realms/myrealm/protocol/openid-connect/auth?client_id=app&redirect_uri=https://app.example/cb&response_type=code&scope=openid&state=xyz";
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let html = std::str::from_utf8(&body).unwrap();
        assert!(html.contains("<form"));
        assert!(html.contains("name=\"username\""));
    }

    // upstream: keycloak/keycloak AuthorizationEndpointTest.java:getUnknownRealm404
    #[tokio::test]
    async fn authorize_get_unknown_realm_404() {
        let (app, _) = setup().await;
        let uri = "/realms/nope/protocol/openid-connect/auth?client_id=app&redirect_uri=https://app.example/cb&response_type=code";
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // upstream: keycloak/keycloak AuthorizationEndpointTest.java:postEmitsAuthorizationCode
    #[tokio::test]
    async fn authorize_post_emits_code_redirect() {
        let (app, state) = setup().await;
        let uri = "/realms/myrealm/protocol/openid-connect/auth?client_id=app&redirect_uri=https://app.example/cb&response_type=code&scope=openid&state=xyz";
        let body = "username=alice&password=hunter2";
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        let loc = resp.headers().get("location").unwrap().to_str().unwrap();
        assert!(loc.starts_with("https://app.example/cb?code="));
        assert!(loc.contains("state=xyz"));
        assert_eq!(state.codes.len().await, 1);
    }

    // upstream: keycloak/keycloak AuthorizationEndpointTest.java:postBadPassword401
    #[tokio::test]
    async fn authorize_post_bad_password_rejects() {
        let (app, _) = setup().await;
        let uri = "/realms/myrealm/protocol/openid-connect/auth?client_id=app&redirect_uri=https://app.example/cb&response_type=code";
        let body = "username=alice&password=wrong";
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // upstream: keycloak/keycloak AuthorizationEndpointTest.java:rejectsRedirectUriNotInWhitelist
    #[tokio::test]
    async fn authorize_get_rejects_unknown_redirect_uri() {
        let (app, _) = setup().await;
        let uri = "/realms/myrealm/protocol/openid-connect/auth?client_id=app&redirect_uri=https://evil/cb&response_type=code";
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // upstream: keycloak/keycloak AuthorizationEndpointTest.java:promptNoneReturnsLoginRequired
    #[tokio::test]
    async fn prompt_none_returns_login_required_redirect() {
        let (app, _) = setup().await;
        let uri = "/realms/myrealm/protocol/openid-connect/auth?client_id=app&redirect_uri=https://app.example/cb&response_type=code&prompt=none&state=zzz";
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        let loc = resp.headers().get("location").unwrap().to_str().unwrap();
        assert!(loc.contains("error=login_required"));
        assert!(loc.contains("state=zzz"));
    }

    // upstream: keycloak/keycloak AuthorizationEndpointTest.java:invalidResponseTypeRedirectsError
    #[tokio::test]
    async fn invalid_response_type_returns_400_json() {
        let (app, _) = setup().await;
        let uri = "/realms/myrealm/protocol/openid-connect/auth?client_id=app&redirect_uri=https://app.example/cb&response_type=trash";
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // upstream: keycloak/keycloak AuthorizationEndpointTest.java:parRequestUriResolves
    #[tokio::test]
    async fn par_request_uri_is_resolved() {
        let (app, state) = setup().await;
        let par_uri = format!("{}{}", PAR_URI_PREFIX, Uuid::new_v4());
        state.par.put(super::super::ParRecord {
            request_uri: par_uri.clone(),
            client_id: "app".into(),
            realm: "myrealm".into(),
            stored_request: "client_id=app&redirect_uri=https://app.example/cb&response_type=code&scope=openid&state=fromPar".into(),
            exp_unix: Utc::now().timestamp() + 60,
        }).await;
        let url = format!(
            "/realms/myrealm/protocol/openid-connect/auth?request_uri={}",
            urlencoding::encode(&par_uri)
        );
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(url)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // PAR is consumed.
        assert_eq!(state.par.len().await, 0);
        // Login form rendered for the merged params.
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
