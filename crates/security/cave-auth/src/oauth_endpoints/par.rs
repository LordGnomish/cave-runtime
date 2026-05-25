// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../oidc/par/endpoints/PushedAuthzRequestEndpoint.java
//
//! RFC 9126 — OAuth 2.0 Pushed Authorization Requests (PAR).
//!
//! `POST /realms/{realm}/protocol/openid-connect/ext/par/request` accepts
//! the same form parameters as `/auth` and returns:
//! ```json
//! {
//!   "request_uri": "urn:ietf:params:oauth:request_uri:abc...",
//!   "expires_in": 60
//! }
//! ```
//! The client then redirects the user agent to `/auth?request_uri=<uri>`
//! and the authorize endpoint resolves it via `ParStore::take`.

use axum::{
    Json, Router,
    extract::{Form, Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::post,
};
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use super::{OAuthEndpointsState, ParRecord, authz_request::validate};

const PAR_TTL: i64 = 60;
const PAR_URI_PREFIX: &str = "urn:ietf:params:oauth:request_uri:";

#[derive(Debug, Deserialize)]
pub struct ParForm {
    pub client_id: String,
    pub redirect_uri: String,
    pub response_type: String,
    pub scope: Option<String>,
    pub state: Option<String>,
    pub nonce: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    pub prompt: Option<String>,
    pub max_age: Option<i64>,
    pub login_hint: Option<String>,
    pub response_mode: Option<String>,
}

pub async fn par_request(
    State(state): State<OAuthEndpointsState>,
    Path(realm): Path<String>,
    Form(form): Form<ParForm>,
) -> impl IntoResponse {
    if state.realms.get(&realm).await.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error":"invalid_request"})),
        )
            .into_response();
    }
    if state
        .clients
        .get_by_client_id(&realm, &form.client_id)
        .await
        .is_none()
    {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error":"invalid_client"})),
        )
            .into_response();
    }

    // Build a fresh AuthzRequest and validate it eagerly.
    let req = super::authz_request::AuthzRequest {
        client_id: form.client_id.clone(),
        redirect_uri: form.redirect_uri.clone(),
        response_type: form.response_type.clone(),
        scope: form.scope.clone(),
        state: form.state.clone(),
        nonce: form.nonce.clone(),
        code_challenge: form.code_challenge.clone(),
        code_challenge_method: form.code_challenge_method.clone(),
        prompt: form.prompt.clone(),
        max_age: form.max_age,
        login_hint: form.login_hint.clone(),
        request_uri: None,
        response_mode: form.response_mode.clone(),
    };
    if let Err(e) = validate(req) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e.error, "error_description": e.error_description})),
        )
            .into_response();
    }

    let request_uri = format!("{}{}", PAR_URI_PREFIX, Uuid::new_v4());
    let stored = encode_form(&form);
    state
        .par
        .put(ParRecord {
            request_uri: request_uri.clone(),
            client_id: form.client_id,
            realm,
            stored_request: stored,
            exp_unix: Utc::now().timestamp() + PAR_TTL,
        })
        .await;

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "request_uri": request_uri,
            "expires_in": PAR_TTL,
        })),
    )
        .into_response()
}

fn encode_form(f: &ParForm) -> String {
    let mut pairs: Vec<(&str, String)> = vec![
        ("client_id", f.client_id.clone()),
        ("redirect_uri", f.redirect_uri.clone()),
        ("response_type", f.response_type.clone()),
    ];
    if let Some(v) = &f.scope {
        pairs.push(("scope", v.clone()));
    }
    if let Some(v) = &f.state {
        pairs.push(("state", v.clone()));
    }
    if let Some(v) = &f.nonce {
        pairs.push(("nonce", v.clone()));
    }
    if let Some(v) = &f.code_challenge {
        pairs.push(("code_challenge", v.clone()));
    }
    if let Some(v) = &f.code_challenge_method {
        pairs.push(("code_challenge_method", v.clone()));
    }
    if let Some(v) = &f.prompt {
        pairs.push(("prompt", v.clone()));
    }
    if let Some(v) = f.max_age {
        pairs.push(("max_age", v.to_string()));
    }
    if let Some(v) = &f.login_hint {
        pairs.push(("login_hint", v.clone()));
    }
    if let Some(v) = &f.response_mode {
        pairs.push(("response_mode", v.clone()));
    }
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

pub fn router(state: OAuthEndpointsState) -> Router {
    Router::new()
        .route(
            "/realms/{realm}/protocol/openid-connect/ext/par/request",
            post(par_request),
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
    use serde_json::Value;
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
                    client_id: "pc".into(),
                    name: None,
                    description: None,
                    enabled: Some(true),
                    public_client: Some(true),
                    secret: None,
                    redirect_uris: Some(vec!["https://app/cb".into()]),
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

    async fn body(resp: axum::response::Response) -> Value {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    }

    // upstream: keycloak/keycloak PushedAuthzRequestEndpointTest.java:storesRequestAndIssuesUri
    #[tokio::test]
    async fn par_stores_request_and_issues_uri() {
        let (app, state) = setup().await;
        let resp = app.oneshot(Request::builder().method("POST").uri("/realms/r/protocol/openid-connect/ext/par/request")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("client_id=pc&redirect_uri=https://app/cb&response_type=code&scope=openid&state=abc")).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let b = body(resp).await;
        let uri = b["request_uri"].as_str().unwrap();
        assert!(uri.starts_with("urn:ietf:params:oauth:request_uri:"));
        assert_eq!(b["expires_in"], 60);
        assert_eq!(state.par.len().await, 1);
    }

    // upstream: keycloak/keycloak PushedAuthzRequestEndpointTest.java:invalidResponseTypeRejected
    #[tokio::test]
    async fn par_invalid_response_type_rejected() {
        let (app, _) = setup().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/realms/r/protocol/openid-connect/ext/par/request")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(
                        "client_id=pc&redirect_uri=https://app/cb&response_type=bogus",
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // upstream: keycloak/keycloak PushedAuthzRequestEndpointTest.java:unknownClientRejected
    #[tokio::test]
    async fn par_unknown_client_rejected() {
        let (app, _) = setup().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/realms/r/protocol/openid-connect/ext/par/request")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(
                        "client_id=ghost&redirect_uri=https://app/cb&response_type=code",
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // upstream: keycloak/keycloak PushedAuthzRequestEndpointTest.java:unknownRealm404
    #[tokio::test]
    async fn par_unknown_realm_404() {
        let (app, _) = setup().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/realms/no/protocol/openid-connect/ext/par/request")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(
                        "client_id=pc&redirect_uri=https://app/cb&response_type=code",
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
