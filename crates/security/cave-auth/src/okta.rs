// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Okta Authorization Server integration.
//!
//! ## What this module covers
//!
//! 1. **OktaAuthServer** — thin async client for the Okta OAuth 2.0 /
//!    Authorization Server REST API: `/authorize`, `/token`, `/introspect`,
//!    `/revoke`.
//! 2. **Group sync** — periodically fetches Okta group memberships and maps
//!    them to CAVE roles (runs as a background `tokio` task).
//! 3. **SCIM 2.0 provisioning** — axum route handlers implementing the SCIM
//!    User resource so that Okta can push user create/update/deactivate events
//!    directly into CAVE.  Deactivating a user revokes all their PATs.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::tokens::TokenStore;

// ─── Okta Authorization Server client ────────────────────────────────────────

/// Okta custom authorization server configuration.
#[derive(Debug, Clone)]
pub struct OktaAuthServerConfig {
    /// Okta org domain, e.g. "https://company.okta.com"
    pub domain: String,
    /// Custom authorization server ID (or "default")
    pub auth_server_id: String,
    /// OAuth 2.0 client ID
    pub client_id: String,
    /// OAuth 2.0 client secret
    pub client_secret: String,
}

impl OktaAuthServerConfig {
    fn base_url(&self) -> String {
        format!("{}/oauth2/{}", self.domain, self.auth_server_id)
    }

    /// JWKS URI for this authorization server.
    pub fn jwks_uri(&self) -> String {
        format!("{}/v1/keys", self.base_url())
    }

    /// Issuer URL used in JWT `iss` claim.
    pub fn issuer(&self) -> String {
        self.base_url()
    }
}

// ─── Okta API response types ──────────────────────────────────────────────────

/// Response from `/v1/token`.
#[derive(Debug, Deserialize, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
    #[serde(default)]
    pub scope: String,
    pub id_token: Option<String>,
    pub refresh_token: Option<String>,
}

/// Response from `/v1/introspect`.
#[derive(Debug, Deserialize, Serialize)]
pub struct IntrospectResponse {
    /// Whether the token is active (not expired / not revoked).
    pub active: bool,
    pub sub: Option<String>,
    pub username: Option<String>,
    pub scope: Option<String>,
    pub client_id: Option<String>,
    pub exp: Option<i64>,
    pub iat: Option<i64>,
    pub uid: Option<String>,
    #[serde(default)]
    pub groups: Vec<String>,
}

/// A single Okta group returned from the Groups API.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OktaGroup {
    pub id: String,
    #[serde(rename = "profile")]
    pub profile: OktaGroupProfile,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OktaGroupProfile {
    pub name: String,
    pub description: Option<String>,
}

/// A single Okta user returned from the Users API.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OktaUser {
    pub id: String,
    pub status: String,
    pub profile: OktaUserProfile,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OktaUserProfile {
    pub login: String,
    pub email: String,
    #[serde(rename = "firstName")]
    pub first_name: String,
    #[serde(rename = "lastName")]
    pub last_name: String,
}

// ─── Okta AS client ───────────────────────────────────────────────────────────

/// Async Okta Authorization Server client.
#[derive(Clone)]
pub struct OktaAuthServer {
    config: OktaAuthServerConfig,
    client: reqwest::Client,
}

impl OktaAuthServer {
    pub fn new(config: OktaAuthServerConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    // ── /v1/token ────────────────────────────────────────────────────────

    /// Exchange an authorization code for tokens (Authorization Code flow).
    pub async fn exchange_code(
        &self,
        code: &str,
        redirect_uri: &str,
    ) -> Result<TokenResponse, String> {
        let url = format!("{}/v1/token", self.config.base_url());
        let resp = self
            .client
            .post(&url)
            .basic_auth(&self.config.client_id, Some(&self.config.client_secret))
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", redirect_uri),
            ])
            .send()
            .await
            .map_err(|e| format!("token exchange failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("token endpoint {status}: {body}"));
        }

        resp.json::<TokenResponse>()
            .await
            .map_err(|e| format!("token parse failed: {e}"))
    }

    /// Client Credentials flow — for service-to-service calls.
    pub async fn client_credentials(&self, scope: &str) -> Result<TokenResponse, String> {
        let url = format!("{}/v1/token", self.config.base_url());
        let resp = self
            .client
            .post(&url)
            .basic_auth(&self.config.client_id, Some(&self.config.client_secret))
            .form(&[("grant_type", "client_credentials"), ("scope", scope)])
            .send()
            .await
            .map_err(|e| format!("client_credentials failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("token endpoint {status}: {body}"));
        }

        resp.json::<TokenResponse>()
            .await
            .map_err(|e| format!("token parse failed: {e}"))
    }

    // ── /v1/introspect ────────────────────────────────────────────────────

    /// Introspect a token to check liveness + claims.
    pub async fn introspect(&self, token: &str) -> Result<IntrospectResponse, String> {
        let url = format!("{}/v1/introspect", self.config.base_url());
        let resp = self
            .client
            .post(&url)
            .basic_auth(&self.config.client_id, Some(&self.config.client_secret))
            .form(&[("token", token), ("token_type_hint", "access_token")])
            .send()
            .await
            .map_err(|e| format!("introspect request failed: {e}"))?;

        resp.json::<IntrospectResponse>()
            .await
            .map_err(|e| format!("introspect parse failed: {e}"))
    }

    // ── /v1/revoke ────────────────────────────────────────────────────────

    /// Revoke an access or refresh token at Okta.
    pub async fn revoke(&self, token: &str) -> Result<(), String> {
        let url = format!("{}/v1/revoke", self.config.base_url());
        let resp = self
            .client
            .post(&url)
            .basic_auth(&self.config.client_id, Some(&self.config.client_secret))
            .form(&[("token", token)])
            .send()
            .await
            .map_err(|e| format!("revoke request failed: {e}"))?;

        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("revoke returned {}", resp.status()))
        }
    }

    // ── Groups API ────────────────────────────────────────────────────────

    /// Fetch all groups from the Okta Groups API.
    pub async fn list_groups(&self, api_token: &str) -> Result<Vec<OktaGroup>, String> {
        let url = format!("{}/api/v1/groups", self.config.domain);
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("SSWS {api_token}"))
            .send()
            .await
            .map_err(|e| format!("list_groups failed: {e}"))?;

        resp.json::<Vec<OktaGroup>>()
            .await
            .map_err(|e| format!("list_groups parse failed: {e}"))
    }

    /// Fetch group members for a given group ID.
    pub async fn get_group_members(
        &self,
        api_token: &str,
        group_id: &str,
    ) -> Result<Vec<OktaUser>, String> {
        let url = format!("{}/api/v1/groups/{}/users", self.config.domain, group_id);
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("SSWS {api_token}"))
            .send()
            .await
            .map_err(|e| format!("get_group_members failed: {e}"))?;

        resp.json::<Vec<OktaUser>>()
            .await
            .map_err(|e| format!("get_group_members parse failed: {e}"))
    }

    // ── Group sync ────────────────────────────────────────────────────────

    /// Sync Okta groups → CAVE role mappings and return a group→members map.
    /// The caller (e.g. `RbacEngine`) is responsible for updating role bindings.
    pub async fn sync_groups(
        &self,
        api_token: &str,
    ) -> Result<HashMap<String, Vec<String>>, String> {
        let groups = self.list_groups(api_token).await?;
        let mut result: HashMap<String, Vec<String>> = HashMap::new();

        for group in &groups {
            let group_name = &group.profile.name;
            match self.get_group_members(api_token, &group.id).await {
                Ok(members) => {
                    let logins: Vec<String> =
                        members.iter().map(|u| u.profile.login.clone()).collect();
                    info!(
                        group = %group_name,
                        members = logins.len(),
                        "Okta group synced"
                    );
                    result.insert(group_name.clone(), logins);
                }
                Err(e) => {
                    warn!(group = %group_name, error = %e, "Failed to fetch group members");
                }
            }
        }

        Ok(result)
    }

    /// Start a background task that syncs groups every `interval_secs` seconds.
    pub fn start_group_sync(self: Arc<Self>, api_token: String, interval_secs: u64) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            loop {
                interval.tick().await;
                match self.sync_groups(&api_token).await {
                    Ok(groups) => {
                        info!(groups = groups.len(), "Okta group sync complete");
                    }
                    Err(e) => {
                        warn!(error = %e, "Okta group sync failed");
                    }
                }
            }
        });
    }
}

// ─── SCIM 2.0 provisioning ────────────────────────────────────────────────────

/// SCIM User resource (simplified to the subset Okta sends).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScimUser {
    pub id: Option<String>,
    pub user_name: String,
    pub active: bool,
    pub name: Option<ScimName>,
    #[serde(default)]
    pub emails: Vec<ScimEmail>,
    #[serde(default)]
    pub groups: Vec<ScimGroupMembership>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScimName {
    pub formatted: Option<String>,
    #[serde(rename = "givenName")]
    pub given_name: Option<String>,
    #[serde(rename = "familyName")]
    pub family_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScimEmail {
    pub value: String,
    pub primary: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScimGroupMembership {
    pub value: String,
    pub display: Option<String>,
}

/// SCIM ListResponse wrapper.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScimListResponse<T> {
    pub schemas: Vec<String>,
    pub total_results: usize,
    pub start_index: usize,
    pub items_per_page: usize,
    #[serde(rename = "Resources")]
    pub resources: Vec<T>,
}

/// Shared state for SCIM handlers.
#[derive(Clone)]
pub struct ScimState {
    /// In-memory SCIM user store (keyed by SCIM id = cave_uid string)
    users: Arc<RwLock<HashMap<String, ScimUser>>>,
    token_store: Arc<TokenStore>,
}

impl ScimState {
    pub fn new(token_store: Arc<TokenStore>) -> Self {
        Self {
            users: Arc::new(RwLock::new(HashMap::new())),
            token_store,
        }
    }
}

// ── SCIM handlers ─────────────────────────────────────────────────────────────

#[allow(dead_code)]
const SCIM_USER_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:User";
const SCIM_LIST_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:ListResponse";

async fn scim_list_users(State(state): State<Arc<ScimState>>) -> impl IntoResponse {
    let users = state.users.read().await;
    let list = users.values().cloned().collect::<Vec<_>>();
    let resp = ScimListResponse {
        schemas: vec![SCIM_LIST_SCHEMA.to_string()],
        total_results: list.len(),
        start_index: 1,
        items_per_page: list.len(),
        resources: list,
    };
    (StatusCode::OK, Json(resp))
}

async fn scim_create_user(
    State(state): State<Arc<ScimState>>,
    Json(mut user): Json<ScimUser>,
) -> impl IntoResponse {
    let id = uuid::Uuid::new_v4().to_string();
    user.id = Some(id.clone());

    info!(scim_user = %user.user_name, "SCIM: user provisioned");
    state.users.write().await.insert(id.clone(), user.clone());

    (StatusCode::CREATED, Json(user))
}

async fn scim_get_user(
    State(state): State<Arc<ScimState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let users = state.users.read().await;
    match users.get(&id) {
        Some(user) => (StatusCode::OK, Json(user.clone())).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "User not found" })),
        )
            .into_response(),
    }
}

async fn scim_replace_user(
    State(state): State<Arc<ScimState>>,
    Path(id): Path<String>,
    Json(mut user): Json<ScimUser>,
) -> impl IntoResponse {
    user.id = Some(id.clone());
    state.users.write().await.insert(id, user.clone());
    (StatusCode::OK, Json(user))
}

async fn scim_patch_user(
    State(state): State<Arc<ScimState>>,
    Path(id): Path<String>,
    Json(patch): Json<serde_json::Value>,
) -> impl IntoResponse {
    let mut users = state.users.write().await;
    let Some(user) = users.get_mut(&id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "User not found" })),
        )
            .into_response();
    };

    // Handle "active: false" deactivation from Okta
    if let Some(ops) = patch.get("Operations").and_then(|o| o.as_array()) {
        for op in ops {
            if op.get("op").and_then(|o| o.as_str()) == Some("replace") {
                if let Some(val) = op.get("value") {
                    if val.get("active") == Some(&serde_json::Value::Bool(false)) {
                        user.active = false;
                        let cave_uid_str = user.id.clone().unwrap_or_default();
                        if let Ok(uid) = uuid::Uuid::parse_str(&cave_uid_str) {
                            let token_store = state.token_store.clone();
                            tokio::spawn(async move {
                                token_store.revoke_all_for_user(uid).await;
                            });
                        }
                        warn!(scim_user = %user.user_name, "SCIM: user deactivated — revoking all PATs");
                    }
                }
            }
        }
    }

    (StatusCode::OK, Json(user.clone())).into_response()
}

async fn scim_delete_user(
    State(state): State<Arc<ScimState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let removed = state.users.write().await.remove(&id);
    if let Some(user) = removed {
        info!(scim_user = %user.user_name, "SCIM: user deprovisioned");
        // Revoke all PATs if we can parse the cave_uid from the SCIM id
        if let Ok(uid) = uuid::Uuid::parse_str(&id) {
            let token_store = state.token_store.clone();
            tokio::spawn(async move {
                token_store.revoke_all_for_user(uid).await;
            });
        }
        StatusCode::NO_CONTENT.into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "User not found" })),
        )
            .into_response()
    }
}

/// Build the SCIM 2.0 axum router.  Mount at the app level with
/// `app.merge(scim_router(token_store))`.
pub fn scim_router(token_store: Arc<TokenStore>) -> Router {
    let state = Arc::new(ScimState::new(token_store));

    Router::new()
        .route(
            "/scim/v2/Users",
            get(scim_list_users).post(scim_create_user),
        )
        .route(
            "/scim/v2/Users/{id}",
            get(scim_get_user)
                .put(scim_replace_user)
                .patch(scim_patch_user)
                .delete(scim_delete_user),
        )
        .with_state(state)
}
