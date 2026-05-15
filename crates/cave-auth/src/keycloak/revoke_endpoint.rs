// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@b825ba97 services/src/main/java/org/keycloak/protocol/oidc/endpoints/LogoutEndpoint.java
//
//! RFC 7009 — OAuth 2.0 Token Revocation.
//!
//! Endpoint:
//!   POST /realms/{realm}/protocol/openid-connect/revoke
//!     Content-Type: application/x-www-form-urlencoded
//!     Body: token=<value>[&token_type_hint=access_token|refresh_token]
//!     → 200 OK with empty body (RFC 7009 §2.2 — always 200 regardless of
//!       whether the token was known, to avoid token-existence oracles).

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
    Router,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::keycloak::client::ClientStore;

// ─── Revoked-token set ────────────────────────────────────────────────────────

/// In-memory denylist of revoked token signatures.  The deployment layer is
/// expected to wire this to a persistent store (e.g. cave-cache TTL keys).
#[derive(Clone, Default)]
pub struct RevokedTokenStore {
    inner: Arc<RwLock<HashMap<String, RevokedTokenEntry>>>,
}

#[derive(Debug, Clone)]
pub struct RevokedTokenEntry {
    pub token: String,
    pub token_type_hint: Option<String>,
    pub client_id: String,
    pub revoked_at: i64,
}

impl RevokedTokenStore {
    pub fn new() -> Self { Self::default() }

    pub async fn revoke(&self, entry: RevokedTokenEntry) {
        self.inner.write().await.insert(entry.token.clone(), entry);
    }

    pub async fn is_revoked(&self, token: &str) -> bool {
        self.inner.read().await.contains_key(token)
    }

    pub async fn list(&self) -> Vec<RevokedTokenEntry> {
        self.inner.read().await.values().cloned().collect()
    }
}

// ─── Service ──────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct RevokeService {
    pub clients: ClientStore,
    pub revoked: RevokedTokenStore,
}

impl RevokeService {
    pub fn new(clients: ClientStore) -> Self {
        Self {
            clients,
            revoked: RevokedTokenStore::new(),
        }
    }
}

// ─── Handler ──────────────────────────────────────────────────────────────────

pub async fn revoke_endpoint(
    State(svc): State<RevokeService>,
    Path(realm): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let mut params: HashMap<String, String> = HashMap::new();
    for (k, v) in form_urlencoded::parse(body.as_bytes()) {
        params.insert(k.into_owned(), v.into_owned());
    }
    let token = params.get("token").cloned().unwrap_or_default();
    let token_type_hint = params.get("token_type_hint").cloned();
    let hint_label = token_type_hint.clone().unwrap_or_else(|| "none".to_string());

    let Some(client_id) = params.get("client_id").cloned().or_else(|| basic_user(&headers)) else {
        super::metrics::inc_revoke(&realm, &hint_label, "no_client");
        return (StatusCode::UNAUTHORIZED, "").into_response();
    };
    // Client auth required (RFC 7009 §2.1).
    let client = match svc.clients.get_by_client_id(&realm, &client_id).await {
        Some(c) => c,
        None => {
            super::metrics::inc_revoke(&realm, &hint_label, "unknown_client");
            return (StatusCode::UNAUTHORIZED, "").into_response();
        }
    };
    if !client.public_client {
        let secret = params.get("client_secret").cloned().or_else(|| basic_pass(&headers));
        let provided = secret.unwrap_or_default();
        let expected = client.secret.as_deref().unwrap_or("");
        if provided != expected {
            super::metrics::inc_revoke(&realm, &hint_label, "bad_secret");
            return (StatusCode::UNAUTHORIZED, "").into_response();
        }
    }

    if token.is_empty() {
        super::metrics::inc_revoke(&realm, &hint_label, "missing_token");
        // RFC 7009 §2.2.1 — missing required parameter ⇒ 400 invalid_request.
        return (StatusCode::BAD_REQUEST, "").into_response();
    }

    svc.revoked.revoke(RevokedTokenEntry {
        token: token.clone(),
        token_type_hint: token_type_hint.clone(),
        client_id: client_id.clone(),
        revoked_at: chrono::Utc::now().timestamp(),
    }).await;
    super::metrics::inc_revoke(&realm, &hint_label, "ok");

    // RFC 7009 §2.2 — return 200 even if the token was unknown / already revoked.
    StatusCode::OK.into_response()
}

fn basic_user(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let tok = raw.strip_prefix("Basic ")?;
    let bytes = decode_b64(tok)?;
    let s = String::from_utf8(bytes).ok()?;
    let (u, _) = s.split_once(':')?;
    Some(u.to_string())
}

fn basic_pass(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let tok = raw.strip_prefix("Basic ")?;
    let bytes = decode_b64(tok)?;
    let s = String::from_utf8(bytes).ok()?;
    let (_, p) = s.split_once(':')?;
    Some(p.to_string())
}

fn decode_b64(s: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s).ok()
}

pub fn router(svc: RevokeService) -> Router {
    Router::new()
        .route("/realms/{realm}/protocol/openid-connect/revoke", post(revoke_endpoint))
        .with_state(svc)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keycloak::client::CreateClientRequest;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    async fn setup() -> (Router, RevokedTokenStore) {
        let clients = ClientStore::new();
        clients.create("r", CreateClientRequest {
            client_id: "cli1".into(), name: None, description: None, enabled: Some(true),
            public_client: Some(false), secret: Some("sec".into()),
            redirect_uris: None, web_origins: None, protocol: None,
        }).await.unwrap();
        let svc = RevokeService::new(clients);
        let store = svc.revoked.clone();
        (router(svc), store)
    }

    #[tokio::test]
    async fn revoke_returns_200_and_records_token() {
        let (app, store) = setup().await;
        let body = "token=abc.def.ghi&token_type_hint=access_token&client_id=cli1&client_secret=sec";
        let req = Request::post("/realms/r/protocol/openid-connect/revoke")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body)).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(store.is_revoked("abc.def.ghi").await);
    }

    #[tokio::test]
    async fn revoke_unknown_token_still_returns_200() {
        // RFC 7009 §2.2 — must not leak whether the token existed.
        let (app, _) = setup().await;
        let body = "token=anything&client_id=cli1&client_secret=sec";
        let req = Request::post("/realms/r/protocol/openid-connect/revoke")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body)).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn revoke_requires_client_auth() {
        let (app, _) = setup().await;
        let body = "token=x&client_id=cli1&client_secret=wrong";
        let req = Request::post("/realms/r/protocol/openid-connect/revoke")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body)).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn revoke_missing_token_is_400() {
        let (app, _) = setup().await;
        let body = "client_id=cli1&client_secret=sec";
        let req = Request::post("/realms/r/protocol/openid-connect/revoke")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body)).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn revoke_supports_basic_auth() {
        let (app, store) = setup().await;
        use base64::Engine;
        let basic = base64::engine::general_purpose::STANDARD.encode("cli1:sec");
        let body = "token=tok1";
        let req = Request::post("/realms/r/protocol/openid-connect/revoke")
            .header("content-type", "application/x-www-form-urlencoded")
            .header("authorization", format!("Basic {basic}"))
            .body(Body::from(body)).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(store.is_revoked("tok1").await);
    }
}
