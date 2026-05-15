// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@b825ba97 services/src/main/java/org/keycloak/protocol/oidc/grants/ciba/CibaGrantType.java
// Source: keycloak/keycloak@b825ba97 services/src/main/java/org/keycloak/protocol/oidc/grants/ciba/endpoints/BackchannelAuthenticationEndpoint.java
//
//! OIDC Client-Initiated Backchannel Authentication (CIBA) — Core 1.0.
//!
//! Endpoints:
//!   POST /realms/{realm}/protocol/openid-connect/ext/ciba/auth
//!     → 200 JSON { auth_req_id, expires_in, interval }
//!
//! Polling lives on the token endpoint (`grant_type=urn:openid:params:grant-type:ciba`).
//!
//! Authentication mode for the user's device is **out-of-band** — Keycloak
//! ships poll/ping/push modes; we ship a headless poll-mode equivalent
//! that operators approve via `cavectl auth ciba approve <auth_req_id>`.

use axum::{
    extract::{Form, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::keycloak::{client::ClientStore, realm::RealmStore, user::UserStore};

// ─── Constants ────────────────────────────────────────────────────────────────

pub const CIBA_REQUEST_TTL: i64 = 600;
pub const CIBA_DEFAULT_INTERVAL: i64 = 5;

// ─── CIBA-request state ───────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CibaStatus {
    Pending,
    Approved,
    Denied,
    Expired,
}

#[derive(Debug, Clone)]
pub struct CibaRequest {
    pub auth_req_id: String,
    pub realm: String,
    pub client_id: String,
    pub user_id: String,
    pub username: String,
    pub email: Option<String>,
    pub scope: String,
    pub binding_message: Option<String>,
    pub acr_values: Option<String>,
    pub exp: i64,
    pub interval: i64,
    pub last_polled: i64,
    pub status: CibaStatus,
}

#[derive(Clone, Default)]
pub struct CibaRequestStore {
    inner: Arc<RwLock<HashMap<String, CibaRequest>>>,
}

impl CibaRequestStore {
    pub fn new() -> Self { Self::default() }

    pub async fn insert(&self, req: CibaRequest) {
        self.inner.write().await.insert(req.auth_req_id.clone(), req);
    }

    pub async fn approve(&self, auth_req_id: &str) -> bool {
        let mut store = self.inner.write().await;
        if let Some(r) = store.get_mut(auth_req_id) {
            r.status = CibaStatus::Approved;
            true
        } else { false }
    }

    pub async fn deny(&self, auth_req_id: &str) -> bool {
        let mut store = self.inner.write().await;
        if let Some(r) = store.get_mut(auth_req_id) {
            r.status = CibaStatus::Denied;
            true
        } else { false }
    }

    pub async fn list(&self) -> Vec<CibaRequest> {
        self.inner.read().await.values().cloned().collect()
    }

    pub async fn poll(&self, auth_req_id: &str) -> CibaPollOutcome {
        let now = Utc::now().timestamp();
        let mut store = self.inner.write().await;
        let Some(entry) = store.get_mut(auth_req_id) else {
            return CibaPollOutcome::Unknown;
        };
        if entry.exp < now {
            entry.status = CibaStatus::Expired;
            return CibaPollOutcome::Expired;
        }
        match &entry.status {
            CibaStatus::Pending => {
                if now - entry.last_polled < entry.interval {
                    CibaPollOutcome::SlowDown
                } else {
                    entry.last_polled = now;
                    CibaPollOutcome::Pending
                }
            }
            CibaStatus::Approved => CibaPollOutcome::Approved(entry.clone()),
            CibaStatus::Denied => CibaPollOutcome::Denied,
            CibaStatus::Expired => CibaPollOutcome::Expired,
        }
    }
}

#[derive(Debug, Clone)]
pub enum CibaPollOutcome {
    Pending,
    SlowDown,
    Approved(CibaRequest),
    Denied,
    Expired,
    Unknown,
}

// ─── Service ──────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct CibaService {
    pub realms: RealmStore,
    pub users: UserStore,
    pub clients: ClientStore,
    pub requests: CibaRequestStore,
}

impl CibaService {
    pub fn new(realms: RealmStore, users: UserStore, clients: ClientStore) -> Self {
        Self {
            realms,
            users,
            clients,
            requests: CibaRequestStore::new(),
        }
    }
}

// ─── /ext/ciba/auth — initiate ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CibaAuthForm {
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub login_hint: Option<String>,
    #[serde(default)]
    pub login_hint_token: Option<String>,
    #[serde(default)]
    pub id_token_hint: Option<String>,
    #[serde(default)]
    pub binding_message: Option<String>,
    #[serde(default)]
    pub acr_values: Option<String>,
    #[serde(default)]
    pub requested_expiry: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct CibaAuthResponse {
    pub auth_req_id: String,
    pub expires_in: i64,
    pub interval: i64,
}

pub async fn ciba_auth_endpoint(
    State(svc): State<CibaService>,
    Path(realm): Path<String>,
    Form(form): Form<CibaAuthForm>,
) -> Response {
    // Realm + client check.
    if svc.realms.get(&realm).await.is_none() {
        super::metrics::inc_ciba(&realm, "invalid_request");
        let body = serde_json::json!({"error":"invalid_request","error_description":"unknown realm"});
        return (StatusCode::BAD_REQUEST, Json(body)).into_response();
    }
    let client = match svc.clients.get_by_client_id(&realm, &form.client_id).await {
        Some(c) => c,
        None => {
            super::metrics::inc_ciba(&realm, "invalid_client");
            let body = serde_json::json!({"error":"invalid_client","error_description":"unknown client"});
            return (StatusCode::UNAUTHORIZED, Json(body)).into_response();
        }
    };
    if !client.public_client {
        let provided = form.client_secret.as_deref().unwrap_or("");
        let expected = client.secret.as_deref().unwrap_or("");
        if provided != expected {
            super::metrics::inc_ciba(&realm, "invalid_client_secret");
            let body = serde_json::json!({"error":"invalid_client","error_description":"bad secret"});
            return (StatusCode::UNAUTHORIZED, Json(body)).into_response();
        }
    }

    // At least one hint is required (OIDC CIBA §7.1).
    let Some(hint) = form.login_hint.clone().or(form.login_hint_token.clone()).or(form.id_token_hint.clone()) else {
        super::metrics::inc_ciba(&realm, "missing_user_code");
        let body = serde_json::json!({"error":"invalid_request","error_description":"login_hint required"});
        return (StatusCode::BAD_REQUEST, Json(body)).into_response();
    };

    // Resolve to a user — we treat any of the three hints as a username for the headless path.
    let user = match svc.users.get_by_username(&realm, &hint).await {
        Some(u) => u,
        None => {
            super::metrics::inc_ciba(&realm, "unknown_user_id");
            let body = serde_json::json!({"error":"unknown_user_id","error_description":"user not found"});
            return (StatusCode::BAD_REQUEST, Json(body)).into_response();
        }
    };

    let ttl = form.requested_expiry.unwrap_or(CIBA_REQUEST_TTL).min(CIBA_REQUEST_TTL).max(60);
    let auth_req_id = format!("ciba-{}", Uuid::new_v4());
    let now = Utc::now().timestamp();

    svc.requests.insert(CibaRequest {
        auth_req_id: auth_req_id.clone(),
        realm: realm.clone(),
        client_id: form.client_id.clone(),
        user_id: user.id.to_string(),
        username: user.username.clone(),
        email: user.email.clone(),
        scope: form.scope.unwrap_or_else(|| "openid".to_string()),
        binding_message: form.binding_message,
        acr_values: form.acr_values,
        exp: now + ttl,
        interval: CIBA_DEFAULT_INTERVAL,
        last_polled: 0,
        status: CibaStatus::Pending,
    }).await;
    super::metrics::inc_ciba(&realm, "issued");

    let resp = CibaAuthResponse {
        auth_req_id,
        expires_in: ttl,
        interval: CIBA_DEFAULT_INTERVAL,
    };
    (StatusCode::OK, Json(resp)).into_response()
}

pub fn router(svc: CibaService) -> Router {
    Router::new()
        .route("/realms/{realm}/protocol/openid-connect/ext/ciba/auth", post(ciba_auth_endpoint))
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

    async fn setup() -> CibaService {
        let realms = RealmStore::new();
        realms.create(RealmRequest {
            id: "myrealm".into(), display_name: None, enabled: None, ssl_required: None,
            registration_allowed: None, login_with_email_allowed: None,
            duplicate_emails_allowed: None, access_token_lifespan: None,
            sso_session_idle_timeout: None,
        }).await.unwrap();
        let users = UserStore::new();
        users.create("myrealm", CreateUserRequest {
            username: "bob".into(), email: Some("bob@example.com".into()), email_verified: Some(true),
            first_name: None, last_name: None, enabled: Some(true), attributes: None,
            password: Some("p".into()),
        }).await.unwrap();
        let clients = ClientStore::new();
        clients.create("myrealm", CreateClientRequest {
            client_id: "ciba-app".into(), name: None, description: None, enabled: Some(true),
            public_client: Some(false), secret: Some("s".into()), redirect_uris: None,
            web_origins: None, protocol: None,
        }).await.unwrap();
        CibaService::new(realms, users, clients)
    }

    #[tokio::test]
    async fn approve_changes_status() {
        let svc = setup().await;
        let now = Utc::now().timestamp();
        svc.requests.insert(CibaRequest {
            auth_req_id: "r1".into(), realm: "myrealm".into(), client_id: "ciba-app".into(),
            user_id: "u1".into(), username: "bob".into(), email: None, scope: "openid".into(),
            binding_message: None, acr_values: None, exp: now + 600, interval: 5,
            last_polled: 0, status: CibaStatus::Pending,
        }).await;
        assert!(svc.requests.approve("r1").await);
        assert!(matches!(svc.requests.poll("r1").await, CibaPollOutcome::Approved(_)));
    }

    #[tokio::test]
    async fn deny_changes_status() {
        let svc = setup().await;
        let now = Utc::now().timestamp();
        svc.requests.insert(CibaRequest {
            auth_req_id: "r2".into(), realm: "myrealm".into(), client_id: "ciba-app".into(),
            user_id: "u".into(), username: "bob".into(), email: None, scope: "openid".into(),
            binding_message: None, acr_values: None, exp: now + 600, interval: 5,
            last_polled: 0, status: CibaStatus::Pending,
        }).await;
        assert!(svc.requests.deny("r2").await);
        assert!(matches!(svc.requests.poll("r2").await, CibaPollOutcome::Denied));
    }

    #[tokio::test]
    async fn slow_down_on_rapid_poll() {
        let svc = setup().await;
        let now = Utc::now().timestamp();
        svc.requests.insert(CibaRequest {
            auth_req_id: "r3".into(), realm: "myrealm".into(), client_id: "ciba-app".into(),
            user_id: "u".into(), username: "bob".into(), email: None, scope: "openid".into(),
            binding_message: None, acr_values: None, exp: now + 600, interval: 5,
            last_polled: now, status: CibaStatus::Pending,
        }).await;
        assert!(matches!(svc.requests.poll("r3").await, CibaPollOutcome::SlowDown));
    }

    #[tokio::test]
    async fn expired_request() {
        let svc = setup().await;
        let now = Utc::now().timestamp();
        svc.requests.insert(CibaRequest {
            auth_req_id: "r4".into(), realm: "myrealm".into(), client_id: "ciba-app".into(),
            user_id: "u".into(), username: "bob".into(), email: None, scope: "openid".into(),
            binding_message: None, acr_values: None, exp: now - 1, interval: 5,
            last_polled: 0, status: CibaStatus::Pending,
        }).await;
        assert!(matches!(svc.requests.poll("r4").await, CibaPollOutcome::Expired));
    }

    #[tokio::test]
    async fn unknown_returns_unknown() {
        let svc = setup().await;
        assert!(matches!(svc.requests.poll("nope").await, CibaPollOutcome::Unknown));
    }
}
