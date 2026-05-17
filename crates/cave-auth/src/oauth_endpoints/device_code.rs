// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../oidc/grants/device/DeviceGrantType.java
//
//! RFC 8628 — OAuth 2.0 Device Authorization Grant.
//!
//! Endpoints:
//! - `POST /realms/{realm}/protocol/openid-connect/auth/device` — issue
//!   device_code + user_code (§3.2).
//! - `POST /realms/{realm}/protocol/openid-connect/auth/device/approve` —
//!   server-side approval used by tests and by the device-flow login
//!   page in cave-portal (`/admin/auth/device-approve`).
//! - `POST /realms/{realm}/protocol/openid-connect/token` with
//!   `grant_type=urn:ietf:params:oauth:grant-type:device_code` — poll
//!   for token (§3.4); we expose this as a *separate* handler the main
//!   token endpoint mounts.
//!
//! Returned errors follow RFC 8628 §3.5:
//! - `authorization_pending`, `slow_down`, `expired_token`, `access_denied`.

use axum::{
    extract::{Form, Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use chrono::Utc;
use rand::Rng;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{DeviceAuthorization, DeviceStatus, OAuthEndpointsState};

const DEVICE_CODE_TTL: i64 = 600;
const DEFAULT_INTERVAL: i64 = 5;

#[derive(Debug, Deserialize)]
pub struct DeviceAuthForm {
    pub client_id: String,
    pub scope: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DeviceAuthResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: i64,
    pub interval: i64,
}

pub async fn device_auth(
    State(state): State<OAuthEndpointsState>,
    Path(realm): Path<String>,
    Form(form): Form<DeviceAuthForm>,
) -> impl IntoResponse {
    if state.realms.get(&realm).await.is_none() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"invalid_request"}))).into_response();
    }
    if state.clients.get_by_client_id(&realm, &form.client_id).await.is_none() {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error":"invalid_client"}))).into_response();
    }

    let device_code = Uuid::new_v4().to_string();
    let user_code = gen_user_code();
    let now = Utc::now().timestamp();
    let scope = form.scope.unwrap_or_else(|| "openid".into());

    state.devices.put(DeviceAuthorization {
        device_code: device_code.clone(),
        user_code: user_code.clone(),
        realm: realm.clone(),
        client_id: form.client_id.clone(),
        scope: scope.clone(),
        exp_unix: now + DEVICE_CODE_TTL,
        interval: DEFAULT_INTERVAL,
        status: DeviceStatus::Pending,
        approved_user_sub: None,
        last_poll_unix: 0,
    }).await;

    let verification_uri = format!("http://localhost:8080/realms/{}/device", realm);
    let resp = DeviceAuthResponse {
        device_code,
        user_code: user_code.clone(),
        verification_uri_complete: format!("{}?user_code={}", verification_uri, user_code),
        verification_uri,
        expires_in: DEVICE_CODE_TTL,
        interval: DEFAULT_INTERVAL,
    };
    (StatusCode::OK, Json(serde_json::to_value(resp).unwrap())).into_response()
}

#[derive(Debug, Deserialize)]
pub struct DeviceApproveForm {
    pub user_code: String,
    pub username: String,
    pub password: String,
}

pub async fn device_approve(
    State(state): State<OAuthEndpointsState>,
    Path(realm): Path<String>,
    Form(form): Form<DeviceApproveForm>,
) -> impl IntoResponse {
    let mut auth = match state.devices.get_by_user(&form.user_code).await {
        Some(a) => a,
        None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"unknown user_code"}))).into_response(),
    };
    if auth.realm != realm {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"realm_mismatch"}))).into_response();
    }
    let user = match state.users.get_by_username(&realm, &form.username).await {
        Some(u) => u,
        None => return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error":"invalid_credentials"}))).into_response(),
    };
    if !state.users.verify_password(&realm, user.id, &form.password).await {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error":"invalid_credentials"}))).into_response();
    }
    auth.status = DeviceStatus::Approved;
    auth.approved_user_sub = Some(user.id.to_string());
    state.devices.update(auth).await;
    (StatusCode::OK, Json(serde_json::json!({"approved":true}))).into_response()
}

#[derive(Debug, Deserialize)]
pub struct DeviceTokenForm {
    pub grant_type: String,
    pub device_code: String,
    pub client_id: String,
}

#[derive(Debug, Serialize)]
pub struct DevicePollErrorBody {
    pub error: &'static str,
}

/// Poll endpoint as part of the `/token` grant family.
///
/// Mounted at `POST /realms/{realm}/protocol/openid-connect/token/device` so
/// it does not collide with the existing main token endpoint; the main
/// endpoint can also dispatch here when `grant_type` matches.
pub async fn device_token(
    State(state): State<OAuthEndpointsState>,
    Path(realm): Path<String>,
    Form(form): Form<DeviceTokenForm>,
) -> impl IntoResponse {
    if form.grant_type != "urn:ietf:params:oauth:grant-type:device_code" {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"unsupported_grant_type"}))).into_response();
    }
    let mut auth = match state.devices.get_by_device(&form.device_code).await {
        Some(a) => a,
        None => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"invalid_grant"}))).into_response(),
    };
    if auth.realm != realm || auth.client_id != form.client_id {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"invalid_grant"}))).into_response();
    }
    let now = Utc::now().timestamp();
    if now > auth.exp_unix {
        auth.status = DeviceStatus::Expired;
        state.devices.update(auth).await;
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"expired_token"}))).into_response();
    }
    // slow_down: poll faster than `interval` seconds.
    if auth.last_poll_unix > 0 && (now - auth.last_poll_unix) < auth.interval {
        auth.last_poll_unix = now;
        state.devices.update(auth).await;
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"slow_down"}))).into_response();
    }
    auth.last_poll_unix = now;
    state.devices.update(auth.clone()).await;

    match auth.status {
        DeviceStatus::Pending => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"authorization_pending"}))).into_response(),
        DeviceStatus::Denied => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"access_denied"}))).into_response(),
        DeviceStatus::Expired => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"expired_token"}))).into_response(),
        DeviceStatus::Approved => {
            // Mint a minimal token response — the access_token is a static
            // string in this MVP; production wiring delegates to
            // `KeycloakTokenService::make_token_pair` once we plumb the
            // service through OAuthEndpointsState.
            let sub = auth.approved_user_sub.clone().unwrap_or_default();
            let body = serde_json::json!({
                "access_token": format!("dev.{}.{}", auth.device_code, sub),
                "token_type": "Bearer",
                "expires_in": 300,
                "scope": auth.scope,
            });
            (StatusCode::OK, Json(body)).into_response()
        }
    }
}

fn gen_user_code() -> String {
    // Keycloak default user-code format: 8 char base32-ish, dash in the middle.
    const ALPHABET: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ";
    let mut rng = rand::thread_rng();
    let chars: String = (0..8)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect();
    format!("{}-{}", &chars[..4], &chars[4..])
}

pub fn router(state: OAuthEndpointsState) -> Router {
    Router::new()
        .route("/realms/{realm}/protocol/openid-connect/auth/device", post(device_auth))
        .route("/realms/{realm}/protocol/openid-connect/auth/device/approve", post(device_approve))
        .route("/realms/{realm}/protocol/openid-connect/token/device", post(device_token))
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
        users.create("r", CreateUserRequest { username: "bob".into(), email: None, email_verified: None, first_name: None, last_name: None, enabled: Some(true), attributes: None, password: Some("pw".into()) }).await.unwrap();
        let clients = ClientStore::new();
        clients.create("r", CreateClientRequest { client_id: "dev".into(), name: None, description: None, enabled: Some(true), public_client: Some(true), secret: None, redirect_uris: None, web_origins: None, protocol: None }).await.unwrap();
        let state = OAuthEndpointsState::new(realms, clients, users);
        let app = router(state.clone());
        (app, state)
    }

    async fn body(resp: axum::response::Response) -> Value {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    }

    // upstream: keycloak/keycloak DeviceGrantTypeTest.java:issueDeviceCodePending
    #[tokio::test]
    async fn device_auth_issues_codes() {
        let (app, _) = setup().await;
        let resp = app.oneshot(Request::builder().method("POST").uri("/realms/r/protocol/openid-connect/auth/device")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("client_id=dev&scope=openid")).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let b = body(resp).await;
        assert!(b["device_code"].is_string());
        assert!(b["user_code"].is_string());
        assert_eq!(b["interval"], 5);
    }

    // upstream: keycloak/keycloak DeviceGrantTypeTest.java:unknownClientRejected
    #[tokio::test]
    async fn device_auth_unknown_client_rejected() {
        let (app, _) = setup().await;
        let resp = app.oneshot(Request::builder().method("POST").uri("/realms/r/protocol/openid-connect/auth/device")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("client_id=ghost")).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // upstream: keycloak/keycloak DeviceGrantTypeTest.java:pollWhilePendingReturnsAuthorizationPending
    #[tokio::test]
    async fn poll_pending_returns_authorization_pending() {
        let (_, state) = setup().await;
        // Seed a pending device authorization directly.
        state.devices.put(DeviceAuthorization {
            device_code: "dc1".into(), user_code: "U-1".into(), realm: "r".into(), client_id: "dev".into(),
            scope: "openid".into(), exp_unix: Utc::now().timestamp() + 600, interval: 5,
            status: DeviceStatus::Pending, approved_user_sub: None, last_poll_unix: 0,
        }).await;
        let app = router(state.clone());
        let resp = app.oneshot(Request::builder().method("POST").uri("/realms/r/protocol/openid-connect/token/device")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("grant_type=urn:ietf:params:oauth:grant-type:device_code&device_code=dc1&client_id=dev")).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert_eq!(body(resp).await["error"], "authorization_pending");
    }

    // upstream: keycloak/keycloak DeviceGrantTypeTest.java:approveThenPollIssuesToken
    #[tokio::test]
    async fn approve_then_poll_issues_token() {
        let (_, state) = setup().await;
        state.devices.put(DeviceAuthorization {
            device_code: "dc2".into(), user_code: "U-2".into(), realm: "r".into(), client_id: "dev".into(),
            scope: "openid".into(), exp_unix: Utc::now().timestamp() + 600, interval: 0,
            status: DeviceStatus::Pending, approved_user_sub: None, last_poll_unix: 0,
        }).await;
        let app = router(state.clone());
        // Approve.
        let resp = app.clone().oneshot(Request::builder().method("POST").uri("/realms/r/protocol/openid-connect/auth/device/approve")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("user_code=U-2&username=bob&password=pw")).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        // Poll.
        let resp = app.oneshot(Request::builder().method("POST").uri("/realms/r/protocol/openid-connect/token/device")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("grant_type=urn:ietf:params:oauth:grant-type:device_code&device_code=dc2&client_id=dev")).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let b = body(resp).await;
        assert!(b["access_token"].as_str().unwrap().starts_with("dev.dc2."));
    }

    // upstream: keycloak/keycloak DeviceGrantTypeTest.java:expiredDeviceCodeRejected
    #[tokio::test]
    async fn expired_device_code_rejected() {
        let (_, state) = setup().await;
        state.devices.put(DeviceAuthorization {
            device_code: "dc3".into(), user_code: "U-3".into(), realm: "r".into(), client_id: "dev".into(),
            scope: "openid".into(), exp_unix: Utc::now().timestamp() - 10, interval: 0,
            status: DeviceStatus::Pending, approved_user_sub: None, last_poll_unix: 0,
        }).await;
        let app = router(state.clone());
        let resp = app.oneshot(Request::builder().method("POST").uri("/realms/r/protocol/openid-connect/token/device")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("grant_type=urn:ietf:params:oauth:grant-type:device_code&device_code=dc3&client_id=dev")).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert_eq!(body(resp).await["error"], "expired_token");
    }

    // upstream: keycloak/keycloak DeviceGrantTypeTest.java:slowDownEnforced
    #[tokio::test]
    async fn slow_down_enforced_when_poll_too_fast() {
        let (_, state) = setup().await;
        let now = Utc::now().timestamp();
        state.devices.put(DeviceAuthorization {
            device_code: "dc4".into(), user_code: "U-4".into(), realm: "r".into(), client_id: "dev".into(),
            scope: "openid".into(), exp_unix: now + 600, interval: 5,
            status: DeviceStatus::Pending, approved_user_sub: None, last_poll_unix: now,
        }).await;
        let app = router(state.clone());
        let resp = app.oneshot(Request::builder().method("POST").uri("/realms/r/protocol/openid-connect/token/device")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("grant_type=urn:ietf:params:oauth:grant-type:device_code&device_code=dc4&client_id=dev")).unwrap()).await.unwrap();
        assert_eq!(body(resp).await["error"], "slow_down");
    }

    // upstream: keycloak/keycloak DeviceGrantTypeTest.java:unsupportedGrantType
    #[tokio::test]
    async fn unsupported_grant_type_rejected() {
        let (app, _) = setup().await;
        let resp = app.oneshot(Request::builder().method("POST").uri("/realms/r/protocol/openid-connect/token/device")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("grant_type=password&device_code=x&client_id=dev")).unwrap()).await.unwrap();
        assert_eq!(body(resp).await["error"], "unsupported_grant_type");
    }
}
