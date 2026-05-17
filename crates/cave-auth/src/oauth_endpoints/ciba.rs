// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../oidc/grants/ciba/CibaGrantType.java
//
//! OpenID Connect Client-Initiated Backchannel Authentication (CIBA, draft-12).
//!
//! Endpoints:
//! - `POST /realms/{realm}/protocol/openid-connect/ext/ciba/auth` — initiate
//!   the backchannel auth request (returns `auth_req_id`).
//! - `POST /realms/{realm}/protocol/openid-connect/ext/ciba/approve` —
//!   approve a pending request (server-side test/admin handler).
//! - `POST /realms/{realm}/protocol/openid-connect/token/ciba` — poll for
//!   token via `grant_type=urn:openid:params:grant-type:ciba`.
//!
//! Compared to the device flow CIBA carries an explicit subject hint
//! (`login_hint` or `id_token_hint`) instead of issuing a user_code.

use axum::{
    extract::{Form, Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use super::{CibaRequest, CibaStatus, OAuthEndpointsState};

const CIBA_TTL: i64 = 600;
const DEFAULT_INTERVAL: i64 = 5;

#[derive(Debug, Deserialize)]
pub struct CibaInitForm {
    pub client_id: String,
    pub scope: Option<String>,
    /// One of `login_hint` / `id_token_hint` / `login_hint_token` must be present.
    pub login_hint: Option<String>,
    pub id_token_hint: Option<String>,
    pub login_hint_token: Option<String>,
    pub binding_message: Option<String>,
}

pub async fn ciba_auth(
    State(state): State<OAuthEndpointsState>,
    Path(realm): Path<String>,
    Form(form): Form<CibaInitForm>,
) -> impl IntoResponse {
    if state.realms.get(&realm).await.is_none() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"invalid_request"}))).into_response();
    }
    if state.clients.get_by_client_id(&realm, &form.client_id).await.is_none() {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error":"invalid_client"}))).into_response();
    }
    if form.login_hint.is_none() && form.id_token_hint.is_none() && form.login_hint_token.is_none() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"missing_user_code","error_description":"one of login_hint / id_token_hint / login_hint_token required"}))).into_response();
    }
    // Resolve user from login_hint.
    let user_sub = if let Some(hint) = form.login_hint.as_deref() {
        match state.users.get_by_username(&realm, hint).await {
            Some(u) => u.id.to_string(),
            None => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"unknown_user_id"}))).into_response(),
        }
    } else {
        // For id_token_hint / login_hint_token we'd decode JWT; MVP treats hints as opaque.
        String::new()
    };

    let auth_req_id = Uuid::new_v4().to_string();
    let now = Utc::now().timestamp();
    state.ciba.put(CibaRequest {
        auth_req_id: auth_req_id.clone(),
        realm: realm.clone(),
        client_id: form.client_id,
        user_sub,
        scope: form.scope.unwrap_or_else(|| "openid".into()),
        exp_unix: now + CIBA_TTL,
        interval: DEFAULT_INTERVAL,
        status: CibaStatus::Pending,
        last_poll_unix: 0,
    }).await;

    (StatusCode::OK, Json(serde_json::json!({
        "auth_req_id": auth_req_id,
        "expires_in": CIBA_TTL,
        "interval": DEFAULT_INTERVAL,
    }))).into_response()
}

#[derive(Debug, Deserialize)]
pub struct CibaApproveForm {
    pub auth_req_id: String,
    pub approve: Option<bool>,
}

pub async fn ciba_approve(
    State(state): State<OAuthEndpointsState>,
    Path(realm): Path<String>,
    Form(form): Form<CibaApproveForm>,
) -> impl IntoResponse {
    let mut req = match state.ciba.get(&form.auth_req_id).await {
        Some(r) => r,
        None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"unknown auth_req_id"}))).into_response(),
    };
    if req.realm != realm {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"realm_mismatch"}))).into_response();
    }
    req.status = if form.approve.unwrap_or(true) { CibaStatus::Approved } else { CibaStatus::Denied };
    state.ciba.update(req).await;
    (StatusCode::OK, Json(serde_json::json!({"status":"ok"}))).into_response()
}

#[derive(Debug, Deserialize)]
pub struct CibaTokenForm {
    pub grant_type: String,
    pub auth_req_id: String,
    pub client_id: String,
}

pub async fn ciba_token(
    State(state): State<OAuthEndpointsState>,
    Path(realm): Path<String>,
    Form(form): Form<CibaTokenForm>,
) -> impl IntoResponse {
    if form.grant_type != "urn:openid:params:grant-type:ciba" {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"unsupported_grant_type"}))).into_response();
    }
    let mut req = match state.ciba.get(&form.auth_req_id).await {
        Some(r) => r,
        None => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"invalid_grant"}))).into_response(),
    };
    if req.realm != realm || req.client_id != form.client_id {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"invalid_grant"}))).into_response();
    }
    let now = Utc::now().timestamp();
    if now > req.exp_unix {
        req.status = CibaStatus::Expired;
        state.ciba.update(req).await;
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"expired_token"}))).into_response();
    }
    if req.last_poll_unix > 0 && (now - req.last_poll_unix) < req.interval {
        req.last_poll_unix = now;
        state.ciba.update(req).await;
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"slow_down"}))).into_response();
    }
    req.last_poll_unix = now;
    state.ciba.update(req.clone()).await;
    match req.status {
        CibaStatus::Pending => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"authorization_pending"}))).into_response(),
        CibaStatus::Denied => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"access_denied"}))).into_response(),
        CibaStatus::Expired => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"expired_token"}))).into_response(),
        CibaStatus::Approved => {
            let body = serde_json::json!({
                "access_token": format!("ciba.{}.{}", req.auth_req_id, req.user_sub),
                "token_type": "Bearer",
                "expires_in": 300,
                "scope": req.scope,
            });
            (StatusCode::OK, Json(body)).into_response()
        }
    }
}

pub fn router(state: OAuthEndpointsState) -> Router {
    Router::new()
        .route("/realms/{realm}/protocol/openid-connect/ext/ciba/auth", post(ciba_auth))
        .route("/realms/{realm}/protocol/openid-connect/ext/ciba/approve", post(ciba_approve))
        .route("/realms/{realm}/protocol/openid-connect/token/ciba", post(ciba_token))
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
    use serde_json::Value;
    use tower::ServiceExt;

    async fn setup() -> (Router, OAuthEndpointsState) {
        let realms = RealmStore::new();
        realms.create(RealmRequest { id: "r".into(), display_name: None, enabled: None, ssl_required: None, registration_allowed: None, login_with_email_allowed: None, duplicate_emails_allowed: None, access_token_lifespan: None, sso_session_idle_timeout: None }).await.unwrap();
        let users = UserStore::new();
        users.create("r", CreateUserRequest { username: "carol".into(), email: None, email_verified: None, first_name: None, last_name: None, enabled: Some(true), attributes: None, password: Some("pw".into()) }).await.unwrap();
        let clients = ClientStore::new();
        clients.create("r", CreateClientRequest { client_id: "back".into(), name: None, description: None, enabled: Some(true), public_client: Some(false), secret: Some("s".into()), redirect_uris: None, web_origins: None, protocol: None }).await.unwrap();
        let state = OAuthEndpointsState::new(realms, clients, users);
        let app = router(state.clone());
        (app, state)
    }

    async fn body(resp: axum::response::Response) -> Value {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    }

    // upstream: keycloak/keycloak CibaGrantTypeTest.java:initiateReturnsAuthReqId
    #[tokio::test]
    async fn ciba_initiate_returns_auth_req_id() {
        let (app, _) = setup().await;
        let resp = app.oneshot(Request::builder().method("POST").uri("/realms/r/protocol/openid-connect/ext/ciba/auth")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("client_id=back&login_hint=carol&scope=openid")).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let b = body(resp).await;
        assert!(b["auth_req_id"].is_string());
    }

    // upstream: keycloak/keycloak CibaGrantTypeTest.java:missingHintRejected
    #[tokio::test]
    async fn ciba_missing_hint_rejected() {
        let (app, _) = setup().await;
        let resp = app.oneshot(Request::builder().method("POST").uri("/realms/r/protocol/openid-connect/ext/ciba/auth")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("client_id=back&scope=openid")).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert_eq!(body(resp).await["error"], "missing_user_code");
    }

    // upstream: keycloak/keycloak CibaGrantTypeTest.java:unknownUserRejected
    #[tokio::test]
    async fn ciba_unknown_user_rejected() {
        let (app, _) = setup().await;
        let resp = app.oneshot(Request::builder().method("POST").uri("/realms/r/protocol/openid-connect/ext/ciba/auth")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("client_id=back&login_hint=ghost")).unwrap()).await.unwrap();
        assert_eq!(body(resp).await["error"], "unknown_user_id");
    }

    // upstream: keycloak/keycloak CibaGrantTypeTest.java:pollPendingReturnsAuthorizationPending
    #[tokio::test]
    async fn poll_pending_returns_authorization_pending() {
        let (_, state) = setup().await;
        state.ciba.put(CibaRequest {
            auth_req_id: "rq1".into(), realm: "r".into(), client_id: "back".into(),
            user_sub: "s".into(), scope: "openid".into(),
            exp_unix: Utc::now().timestamp() + 600, interval: 0,
            status: CibaStatus::Pending, last_poll_unix: 0,
        }).await;
        let app = router(state);
        let resp = app.oneshot(Request::builder().method("POST").uri("/realms/r/protocol/openid-connect/token/ciba")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("grant_type=urn:openid:params:grant-type:ciba&auth_req_id=rq1&client_id=back")).unwrap()).await.unwrap();
        assert_eq!(body(resp).await["error"], "authorization_pending");
    }

    // upstream: keycloak/keycloak CibaGrantTypeTest.java:approveThenPollIssuesToken
    #[tokio::test]
    async fn approve_then_poll_returns_token() {
        let (_, state) = setup().await;
        state.ciba.put(CibaRequest {
            auth_req_id: "rq2".into(), realm: "r".into(), client_id: "back".into(),
            user_sub: "user-sub-2".into(), scope: "openid".into(),
            exp_unix: Utc::now().timestamp() + 600, interval: 0,
            status: CibaStatus::Pending, last_poll_unix: 0,
        }).await;
        let app = router(state);
        // Approve.
        let resp = app.clone().oneshot(Request::builder().method("POST").uri("/realms/r/protocol/openid-connect/ext/ciba/approve")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("auth_req_id=rq2&approve=true")).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let resp = app.oneshot(Request::builder().method("POST").uri("/realms/r/protocol/openid-connect/token/ciba")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("grant_type=urn:openid:params:grant-type:ciba&auth_req_id=rq2&client_id=back")).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let b = body(resp).await;
        assert!(b["access_token"].as_str().unwrap().starts_with("ciba.rq2."));
    }

    // upstream: keycloak/keycloak CibaGrantTypeTest.java:deniedReturnsAccessDenied
    #[tokio::test]
    async fn denied_returns_access_denied() {
        let (_, state) = setup().await;
        state.ciba.put(CibaRequest {
            auth_req_id: "rq3".into(), realm: "r".into(), client_id: "back".into(),
            user_sub: "s".into(), scope: "openid".into(),
            exp_unix: Utc::now().timestamp() + 600, interval: 0,
            status: CibaStatus::Pending, last_poll_unix: 0,
        }).await;
        let app = router(state);
        let resp = app.clone().oneshot(Request::builder().method("POST").uri("/realms/r/protocol/openid-connect/ext/ciba/approve")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("auth_req_id=rq3&approve=false")).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let resp = app.oneshot(Request::builder().method("POST").uri("/realms/r/protocol/openid-connect/token/ciba")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("grant_type=urn:openid:params:grant-type:ciba&auth_req_id=rq3&client_id=back")).unwrap()).await.unwrap();
        assert_eq!(body(resp).await["error"], "access_denied");
    }

    // upstream: keycloak/keycloak CibaGrantTypeTest.java:expiredReturnsExpiredToken
    #[tokio::test]
    async fn expired_returns_expired_token() {
        let (_, state) = setup().await;
        state.ciba.put(CibaRequest {
            auth_req_id: "rq4".into(), realm: "r".into(), client_id: "back".into(),
            user_sub: "s".into(), scope: "openid".into(),
            exp_unix: Utc::now().timestamp() - 1, interval: 0,
            status: CibaStatus::Pending, last_poll_unix: 0,
        }).await;
        let app = router(state);
        let resp = app.oneshot(Request::builder().method("POST").uri("/realms/r/protocol/openid-connect/token/ciba")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("grant_type=urn:openid:params:grant-type:ciba&auth_req_id=rq4&client_id=back")).unwrap()).await.unwrap();
        assert_eq!(body(resp).await["error"], "expired_token");
    }
}
