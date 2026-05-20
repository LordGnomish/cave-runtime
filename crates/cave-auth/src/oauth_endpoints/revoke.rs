// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../oidc/endpoints/TokenRevocationEndpoint.java
//
//! RFC 7009 — OAuth 2.0 Token Revocation.
//!
//! `POST /realms/{realm}/protocol/openid-connect/revoke` accepts a token
//! and optional `token_type_hint`. Per §2.2 the endpoint MUST return 200
//! for unknown tokens to prevent token-scanning attacks, but it MUST
//! return 400 with `unsupported_token_type` for hints it doesn't grok.
//!
//! Authentication of the client is required (RFC 7009 §2.1) but we accept
//! `client_id` in the form body (public client) or HTTP Basic (confidential
//! client) — same as Keycloak's TokenRevocationEndpoint.

use axum::{
    Json, Router,
    extract::{Form, Path, State},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    routing::post,
};
use base64::Engine as _;
use serde::Deserialize;

use super::OAuthEndpointsState;

#[derive(Debug, Deserialize)]
pub struct RevokeForm {
    pub token: String,
    pub token_type_hint: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
}

pub async fn revoke(
    State(state): State<OAuthEndpointsState>,
    Path(realm): Path<String>,
    headers: HeaderMap,
    Form(form): Form<RevokeForm>,
) -> impl IntoResponse {
    if state.realms.get(&realm).await.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error":"invalid_request"})),
        )
            .into_response();
    }
    // Validate token_type_hint.
    if let Some(hint) = form.token_type_hint.as_deref() {
        if !matches!(hint, "access_token" | "refresh_token") {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error":"unsupported_token_type"})),
            )
                .into_response();
        }
    }
    // Resolve client_id from form or Basic auth (RFC 6749 §2.3.1).
    let (client_id, client_secret) = match resolve_client_auth(&headers, &form) {
        Some(pair) => pair,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error":"invalid_client"})),
            )
                .into_response();
        }
    };
    let client = match state.clients.get_by_client_id(&realm, &client_id).await {
        Some(c) => c,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error":"invalid_client"})),
            )
                .into_response();
        }
    };
    if !client.public_client {
        let expect = client.secret.as_deref().unwrap_or("");
        if expect != client_secret.as_deref().unwrap_or("") {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error":"invalid_client"})),
            )
                .into_response();
        }
    }

    // Add to revocation list. RFC 7009: 200 even on unknown token.
    state.revocations.revoke(&form.token).await;
    (StatusCode::OK, Json(serde_json::json!({}))).into_response()
}

fn resolve_client_auth(headers: &HeaderMap, form: &RevokeForm) -> Option<(String, Option<String>)> {
    // HTTP Basic.
    if let Some(auth) = headers.get(header::AUTHORIZATION) {
        let s = auth.to_str().ok()?;
        if let Some(b64) = s.strip_prefix("Basic ") {
            let decoded = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
            let s = String::from_utf8(decoded).ok()?;
            let (id, secret) = s.split_once(':')?;
            return Some((id.to_string(), Some(secret.to_string())));
        }
    }
    let cid = form.client_id.clone()?;
    Some((cid, form.client_secret.clone()))
}

pub fn router(state: OAuthEndpointsState) -> Router {
    Router::new()
        .route(
            "/realms/{realm}/protocol/openid-connect/revoke",
            post(revoke),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keycloak::{
        client::{ClientStore, CreateClientRequest},
        realm::{RealmRequest, RealmStore},
        user::UserStore,
    };
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    async fn setup() -> (Router, OAuthEndpointsState) {
        let realms = RealmStore::new();
        realms
            .create(RealmRequest {
                id: "r".into(),
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
        let clients = ClientStore::new();
        clients
            .create(
                "r",
                CreateClientRequest {
                    client_id: "conf".into(),
                    name: None,
                    description: None,
                    enabled: Some(true),
                    public_client: Some(false),
                    secret: Some("sec".into()),
                    redirect_uris: None,
                    web_origins: None,
                    protocol: None,
                },
            )
            .await
            .unwrap();
        clients
            .create(
                "r",
                CreateClientRequest {
                    client_id: "pub".into(),
                    name: None,
                    description: None,
                    enabled: Some(true),
                    public_client: Some(true),
                    secret: None,
                    redirect_uris: None,
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

    // upstream: keycloak/keycloak TokenRevocationTest.java:revokeKnownTokenReturns200
    #[tokio::test]
    async fn revoke_known_returns_200() {
        let (app, state) = setup().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/realms/r/protocol/openid-connect/revoke")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from("token=tok123&client_id=conf&client_secret=sec"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(state.revocations.is_revoked("tok123").await);
    }

    // upstream: keycloak/keycloak TokenRevocationTest.java:revokeUnknownToken200Anyway
    #[tokio::test]
    async fn revoke_unknown_returns_200_per_rfc7009() {
        let (app, _) = setup().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/realms/r/protocol/openid-connect/revoke")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from("token=nope&client_id=pub"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // upstream: keycloak/keycloak TokenRevocationTest.java:unsupportedTokenTypeRejected
    #[tokio::test]
    async fn unsupported_token_type_rejected() {
        let (app, _) = setup().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/realms/r/protocol/openid-connect/revoke")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from("token=t&token_type_hint=jwt&client_id=pub"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // upstream: keycloak/keycloak TokenRevocationTest.java:basicAuthAccepted
    #[tokio::test]
    async fn basic_auth_accepted() {
        let (app, state) = setup().await;
        let basic = base64::engine::general_purpose::STANDARD.encode("conf:sec");
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/realms/r/protocol/openid-connect/revoke")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .header("authorization", format!("Basic {}", basic))
                    .body(Body::from("token=bt"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(state.revocations.is_revoked("bt").await);
    }

    // upstream: keycloak/keycloak TokenRevocationTest.java:wrongClientSecretRejected
    #[tokio::test]
    async fn wrong_client_secret_rejected() {
        let (app, _) = setup().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/realms/r/protocol/openid-connect/revoke")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from("token=t&client_id=conf&client_secret=wrong"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // upstream: keycloak/keycloak TokenRevocationTest.java:missingClientRejected
    #[tokio::test]
    async fn missing_client_rejected() {
        let (app, _) = setup().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/realms/r/protocol/openid-connect/revoke")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from("token=t"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
