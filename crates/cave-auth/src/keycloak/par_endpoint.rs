// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@b825ba97 services/src/main/java/org/keycloak/protocol/oidc/par/endpoints/PushedAuthzRequestEndpoint.java
//
//! RFC 9126 — OAuth 2.0 Pushed Authorization Requests (PAR).
//!
//! Endpoint:
//!   POST /realms/{realm}/protocol/openid-connect/ext/par/request
//!     → 201 JSON { request_uri, expires_in }
//!
//! The receive side of PAR lives in `auth_endpoint::handle_authorize`:
//! a subsequent `/auth?request_uri=…&client_id=…` redirects to a redirect
//! built from the parameters that were pushed here.

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use chrono::Utc;
use serde::Serialize;
use uuid::Uuid;

use crate::keycloak::{
    auth_endpoint::{AuthorizationRequest, ParRequest, ParRequestStore, PAR_REQUEST_URI_TTL},
    client::ClientStore,
};

#[derive(Clone)]
pub struct ParService {
    pub store: ParRequestStore,
    pub clients: ClientStore,
}

impl ParService {
    pub fn new(store: ParRequestStore, clients: ClientStore) -> Self {
        Self { store, clients }
    }
}

#[derive(Debug, Serialize)]
pub struct ParResponse {
    pub request_uri: String,
    pub expires_in: i64,
}

pub async fn par_endpoint(
    State(svc): State<ParService>,
    Path(realm): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    // Parse application/x-www-form-urlencoded manually so we can collect
    // every PAR parameter — Keycloak treats arbitrary OAuth params here.
    let mut params: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for (k, v) in form_urlencoded::parse(body.as_bytes()) {
        params.insert(k.into_owned(), v.into_owned());
    }
    // Client auth: HTTP Basic or client_secret_post — minimum: client_id present.
    let client_id = match params.get("client_id").cloned().or_else(|| basic_auth_user(&headers)) {
        Some(c) => c,
        None => {
            super::metrics::inc_par(&realm, "no_client");
            return error_resp(StatusCode::UNAUTHORIZED, "invalid_client", "client_id required");
        }
    };
    let client = match svc.clients.get_by_client_id(&realm, &client_id).await {
        Some(c) => c,
        None => {
            super::metrics::inc_par(&realm, "unknown_client");
            return error_resp(StatusCode::UNAUTHORIZED, "invalid_client", "unknown client");
        }
    };
    if !client.public_client {
        let secret = params.get("client_secret").cloned().or_else(|| basic_auth_pass(&headers));
        let provided = secret.unwrap_or_default();
        let expected = client.secret.as_deref().unwrap_or("");
        if provided != expected {
            super::metrics::inc_par(&realm, "bad_secret");
            return error_resp(StatusCode::UNAUTHORIZED, "invalid_client", "bad secret");
        }
    }

    // request_uri pushed via PAR is rejected per RFC 9126 §2.1.
    if params.contains_key("request_uri") {
        super::metrics::inc_par(&realm, "request_uri_forbidden");
        return error_resp(StatusCode::BAD_REQUEST, "invalid_request", "request_uri not allowed");
    }

    // Construct the AuthorizationRequest.
    let auth_req = AuthorizationRequest {
        response_type: params.get("response_type").cloned().unwrap_or_default(),
        client_id: client_id.clone(),
        redirect_uri: params.get("redirect_uri").cloned().unwrap_or_default(),
        scope: params.get("scope").cloned(),
        state: params.get("state").cloned(),
        nonce: params.get("nonce").cloned(),
        response_mode: params.get("response_mode").cloned(),
        prompt: params.get("prompt").cloned(),
        max_age: params.get("max_age").and_then(|v| v.parse().ok()),
        acr_values: params.get("acr_values").cloned(),
        claims: params.get("claims").cloned(),
        login_hint: params.get("login_hint").cloned(),
        id_token_hint: params.get("id_token_hint").cloned(),
        code_challenge: params.get("code_challenge").cloned(),
        code_challenge_method: params.get("code_challenge_method").cloned(),
        request: params.get("request").cloned(),
        request_uri: None, // forbidden
    };

    let request_uri = format!("urn:ietf:params:oauth:request_uri:{}", Uuid::new_v4());
    let now = Utc::now().timestamp();
    svc.store.insert(ParRequest {
        request_uri: request_uri.clone(),
        client_id: client_id.clone(),
        params: auth_req,
        exp: now + PAR_REQUEST_URI_TTL,
    }).await;
    super::metrics::inc_par(&realm, "ok");

    let body = ParResponse {
        request_uri,
        expires_in: PAR_REQUEST_URI_TTL,
    };
    (StatusCode::CREATED, Json(body)).into_response()
}

fn basic_auth_user(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let token = raw.strip_prefix("Basic ")?;
    let decoded = base64_decode(token)?;
    let s = String::from_utf8(decoded).ok()?;
    let (u, _) = s.split_once(':')?;
    Some(u.to_string())
}

fn basic_auth_pass(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let token = raw.strip_prefix("Basic ")?;
    let decoded = base64_decode(token)?;
    let s = String::from_utf8(decoded).ok()?;
    let (_, p) = s.split_once(':')?;
    Some(p.to_string())
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s).ok()
}

fn error_resp(status: StatusCode, code: &str, msg: &str) -> Response {
    let body = serde_json::json!({"error": code, "error_description": msg});
    (status, Json(body)).into_response()
}

pub fn router(svc: ParService) -> Router {
    Router::new()
        .route("/realms/{realm}/protocol/openid-connect/ext/par/request", post(par_endpoint))
        .with_state(svc)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keycloak::{
        client::CreateClientRequest,
        realm::{RealmRequest, RealmStore},
    };
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    async fn setup() -> (Router, ParRequestStore, ClientStore) {
        let realms = RealmStore::new();
        realms.create(RealmRequest {
            id: "myrealm".into(), display_name: None, enabled: None, ssl_required: None,
            registration_allowed: None, login_with_email_allowed: None,
            duplicate_emails_allowed: None, access_token_lifespan: None,
            sso_session_idle_timeout: None,
        }).await.unwrap();
        let clients = ClientStore::new();
        clients.create("myrealm", CreateClientRequest {
            client_id: "cli1".into(), name: None, description: None, enabled: Some(true),
            public_client: Some(false), secret: Some("s1".into()),
            redirect_uris: Some(vec!["https://app.example/cb".into()]),
            web_origins: None, protocol: None,
        }).await.unwrap();
        let _ = realms;
        let store = ParRequestStore::new();
        let svc = ParService::new(store.clone(), clients.clone());
        let app = router(svc);
        (app, store, clients)
    }

    #[tokio::test]
    async fn par_happy_returns_201_with_request_uri() {
        let (app, store, _c) = setup().await;
        let body = "client_id=cli1&client_secret=s1&response_type=code\
                    &redirect_uri=https%3A%2F%2Fapp.example%2Fcb&state=x";
        let req = Request::post("/realms/myrealm/protocol/openid-connect/ext/par/request")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body)).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let request_uri = v["request_uri"].as_str().unwrap();
        assert!(request_uri.starts_with("urn:ietf:params:oauth:request_uri:"));

        // Retrieve.
        let entry = store.take(request_uri).await.unwrap();
        assert_eq!(entry.params.response_type, "code");
        assert_eq!(entry.params.state.as_deref(), Some("x"));
    }

    #[tokio::test]
    async fn par_rejects_inlined_request_uri() {
        let (app, _, _) = setup().await;
        let body = "client_id=cli1&client_secret=s1&request_uri=https%3A%2F%2Fevil";
        let req = Request::post("/realms/myrealm/protocol/openid-connect/ext/par/request")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body)).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn par_rejects_bad_client() {
        let (app, _, _) = setup().await;
        let body = "client_id=cli1&client_secret=wrong&response_type=code";
        let req = Request::post("/realms/myrealm/protocol/openid-connect/ext/par/request")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body)).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn par_supports_basic_auth() {
        let (app, _, _) = setup().await;
        use base64::Engine;
        let basic = base64::engine::general_purpose::STANDARD.encode("cli1:s1");
        let body = "response_type=code&redirect_uri=https%3A%2F%2Fapp.example%2Fcb";
        let req = Request::post("/realms/myrealm/protocol/openid-connect/ext/par/request")
            .header("content-type", "application/x-www-form-urlencoded")
            .header("authorization", format!("Basic {basic}"))
            .body(Body::from(body)).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }
}
