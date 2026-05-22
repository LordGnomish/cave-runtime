// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../protocol/oidc/endpoints/
//
//! OAuth 2.0 / OIDC endpoints — completes the cave-auth Keycloak port
//! beyond the `token`/`userinfo`/`introspect`/`logout` endpoints already
//! in `keycloak::token_endpoint`.
//!
//! ## Endpoints implemented in this module
//! | Path | RFC / OIDC spec | Module |
//! |------|-----------------|--------|
//! | `GET  /realms/{realm}/protocol/openid-connect/auth` | RFC 6749 §4.1, OIDC Core | [`authorize`] |
//! | `POST /realms/{realm}/protocol/openid-connect/auth` | OIDC Core form_post | [`authorize`] |
//! | `POST /realms/{realm}/protocol/openid-connect/auth/device` | RFC 8628 device authorization | [`device_code`] |
//! | `POST /realms/{realm}/protocol/openid-connect/ext/ciba/auth` | OpenID CIBA Core | [`ciba`] |
//! | `POST /realms/{realm}/protocol/openid-connect/revoke` | RFC 7009 | [`revoke`] |
//! | `POST /realms/{realm}/protocol/openid-connect/ext/par/request` | RFC 9126 PAR | [`par`] |
//!
//! All endpoints share the `OAuthEndpointsState` set of stores
//! (authorization code, PAR records, device codes, CIBA requests).
//! Existing realm/client/user stores are injected from `keycloak::*`.

pub mod authorize;
pub mod authz_request;
pub mod ciba;
pub mod device_code;
pub mod par;
pub mod pkce;
pub mod revoke;

#[cfg(test)]
pub mod tests;

use axum::Router;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::keycloak::{client::ClientStore, realm::RealmStore, user::UserStore};

// ─── Authorization code store ────────────────────────────────────────────────

/// Single authorization-code record minted by `/auth` and exchanged at `/token`.
#[derive(Debug, Clone)]
pub struct AuthorizationCode {
    pub code: String,
    pub realm: String,
    pub client_id: String,
    pub user_sub: String,
    pub redirect_uri: String,
    pub scope: String,
    pub state: Option<String>,
    pub nonce: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<pkce::PkceMethod>,
    pub exp_unix: i64,
}

#[derive(Clone, Default)]
pub struct AuthorizationCodeStore {
    inner: Arc<RwLock<HashMap<String, AuthorizationCode>>>,
}

impl AuthorizationCodeStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn put(&self, code: AuthorizationCode) {
        self.inner.write().await.insert(code.code.clone(), code);
    }

    pub async fn take(&self, code: &str) -> Option<AuthorizationCode> {
        self.inner.write().await.remove(code)
    }

    pub async fn len(&self) -> usize {
        self.inner.read().await.len()
    }
}

// ─── Pushed Authorization Request store (RFC 9126) ───────────────────────────

#[derive(Debug, Clone)]
pub struct ParRecord {
    pub request_uri: String,
    pub client_id: String,
    pub realm: String,
    /// Stored serialised query string of the original authorize parameters.
    pub stored_request: String,
    pub exp_unix: i64,
}

#[derive(Clone, Default)]
pub struct ParStore {
    inner: Arc<RwLock<HashMap<String, ParRecord>>>,
}

impl ParStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn put(&self, rec: ParRecord) {
        self.inner
            .write()
            .await
            .insert(rec.request_uri.clone(), rec);
    }

    pub async fn take(&self, uri: &str) -> Option<ParRecord> {
        self.inner.write().await.remove(uri)
    }

    pub async fn len(&self) -> usize {
        self.inner.read().await.len()
    }
}

// ─── Device code store (RFC 8628) ────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct DeviceAuthorization {
    pub device_code: String,
    pub user_code: String,
    pub realm: String,
    pub client_id: String,
    pub scope: String,
    pub exp_unix: i64,
    /// Polling interval in seconds (RFC 8628 §3.2 default 5).
    pub interval: i64,
    /// `pending` | `approved` | `denied` | `expired`
    pub status: DeviceStatus,
    pub approved_user_sub: Option<String>,
    pub last_poll_unix: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceStatus {
    Pending,
    Approved,
    Denied,
    Expired,
}

#[derive(Clone, Default)]
pub struct DeviceCodeStore {
    by_device: Arc<RwLock<HashMap<String, DeviceAuthorization>>>,
    by_user: Arc<RwLock<HashMap<String, String>>>, // user_code → device_code
}

impl DeviceCodeStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn put(&self, auth: DeviceAuthorization) {
        self.by_user
            .write()
            .await
            .insert(auth.user_code.clone(), auth.device_code.clone());
        self.by_device
            .write()
            .await
            .insert(auth.device_code.clone(), auth);
    }

    pub async fn get_by_device(&self, device_code: &str) -> Option<DeviceAuthorization> {
        self.by_device.read().await.get(device_code).cloned()
    }

    pub async fn get_by_user(&self, user_code: &str) -> Option<DeviceAuthorization> {
        let dc = self.by_user.read().await.get(user_code).cloned()?;
        self.get_by_device(&dc).await
    }

    pub async fn update(&self, auth: DeviceAuthorization) {
        self.by_device
            .write()
            .await
            .insert(auth.device_code.clone(), auth);
    }

    pub async fn len(&self) -> usize {
        self.by_device.read().await.len()
    }
}

// ─── CIBA store (OpenID Client-Initiated Backchannel Auth) ───────────────────

#[derive(Debug, Clone)]
pub struct CibaRequest {
    pub auth_req_id: String,
    pub realm: String,
    pub client_id: String,
    pub user_sub: String,
    pub scope: String,
    pub exp_unix: i64,
    pub interval: i64,
    pub status: CibaStatus,
    pub last_poll_unix: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CibaStatus {
    Pending,
    Approved,
    Denied,
    Expired,
}

#[derive(Clone, Default)]
pub struct CibaStore {
    inner: Arc<RwLock<HashMap<String, CibaRequest>>>,
}

impl CibaStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn put(&self, r: CibaRequest) {
        self.inner.write().await.insert(r.auth_req_id.clone(), r);
    }

    pub async fn get(&self, id: &str) -> Option<CibaRequest> {
        self.inner.read().await.get(id).cloned()
    }

    pub async fn update(&self, r: CibaRequest) {
        self.inner.write().await.insert(r.auth_req_id.clone(), r);
    }

    pub async fn len(&self) -> usize {
        self.inner.read().await.len()
    }
}

// ─── Token revocation list (RFC 7009) ────────────────────────────────────────

#[derive(Clone, Default)]
pub struct RevocationStore {
    inner: Arc<RwLock<std::collections::HashSet<String>>>,
}

impl RevocationStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn revoke(&self, token: &str) {
        self.inner.write().await.insert(token.to_string());
    }

    pub async fn is_revoked(&self, token: &str) -> bool {
        self.inner.read().await.contains(token)
    }

    pub async fn len(&self) -> usize {
        self.inner.read().await.len()
    }
}

// ─── Shared state ────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct OAuthEndpointsState {
    pub realms: RealmStore,
    pub clients: ClientStore,
    pub users: UserStore,
    pub codes: AuthorizationCodeStore,
    pub par: ParStore,
    pub devices: DeviceCodeStore,
    pub ciba: CibaStore,
    pub revocations: RevocationStore,
}

impl OAuthEndpointsState {
    pub fn new(realms: RealmStore, clients: ClientStore, users: UserStore) -> Self {
        Self {
            realms,
            clients,
            users,
            codes: AuthorizationCodeStore::new(),
            par: ParStore::new(),
            devices: DeviceCodeStore::new(),
            ciba: CibaStore::new(),
            revocations: RevocationStore::new(),
        }
    }
}

/// Build the full OAuth-endpoints router.
pub fn oauth_endpoints_router(state: OAuthEndpointsState) -> Router {
    Router::new()
        .merge(authorize::router(state.clone()))
        .merge(device_code::router(state.clone()))
        .merge(ciba::router(state.clone()))
        .merge(revoke::router(state.clone()))
        .merge(par::router(state))
}
