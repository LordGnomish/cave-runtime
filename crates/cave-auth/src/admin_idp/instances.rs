// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../admin/IdentityProviderResource.java
//
//! Admin CRUD for identity-provider instances:
//!
//! - `GET    /admin/realms/{realm}/identity-provider/instances`
//! - `POST   /admin/realms/{realm}/identity-provider/instances`
//! - `GET    /admin/realms/{realm}/identity-provider/instances/{alias}`
//! - `PUT    /admin/realms/{realm}/identity-provider/instances/{alias}`
//! - `DELETE /admin/realms/{realm}/identity-provider/instances/{alias}`

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::AdminIdpState;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IdentityProvider {
    pub alias: String,
    pub display_name: Option<String>,
    pub provider_id: String,
    pub enabled: bool,
    /// Keycloak-style flat config map.
    #[serde(default)]
    pub config: HashMap<String, String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateIdpRequest {
    pub alias: String,
    pub display_name: Option<String>,
    pub provider_id: String,
    pub enabled: Option<bool>,
    #[serde(default)]
    pub config: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateIdpRequest {
    pub display_name: Option<String>,
    pub enabled: Option<bool>,
    pub config: Option<HashMap<String, String>>,
}

pub async fn list_idps(
    State(state): State<AdminIdpState>,
    Path(realm): Path<String>,
) -> impl IntoResponse {
    if state.realms.get(&realm).await.is_none() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"realm_not_found"}))).into_response();
    }
    let list = state.providers.list(&realm).await;
    (StatusCode::OK, Json(serde_json::to_value(list).unwrap())).into_response()
}

pub async fn create_idp(
    State(state): State<AdminIdpState>,
    Path(realm): Path<String>,
    Json(req): Json<CreateIdpRequest>,
) -> impl IntoResponse {
    if state.realms.get(&realm).await.is_none() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"realm_not_found"}))).into_response();
    }
    if req.alias.is_empty() || req.provider_id.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"invalid_request"}))).into_response();
    }
    let idp = IdentityProvider {
        alias: req.alias,
        display_name: req.display_name,
        provider_id: req.provider_id,
        enabled: req.enabled.unwrap_or(true),
        config: req.config,
        created_at: Utc::now(),
    };
    match state.providers.create(&realm, idp).await {
        Ok(idp) => (StatusCode::CREATED, Json(serde_json::to_value(idp).unwrap())).into_response(),
        Err("conflict") => (StatusCode::CONFLICT, Json(serde_json::json!({"error":"alias_exists"}))).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error":"server_error"}))).into_response(),
    }
}

pub async fn get_idp(
    State(state): State<AdminIdpState>,
    Path((realm, alias)): Path<(String, String)>,
) -> impl IntoResponse {
    match state.providers.get(&realm, &alias).await {
        Some(idp) => (StatusCode::OK, Json(serde_json::to_value(idp).unwrap())).into_response(),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"not_found"}))).into_response(),
    }
}

pub async fn update_idp(
    State(state): State<AdminIdpState>,
    Path((realm, alias)): Path<(String, String)>,
    Json(req): Json<UpdateIdpRequest>,
) -> impl IntoResponse {
    let mut existing = match state.providers.get(&realm, &alias).await {
        Some(i) => i,
        None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"not_found"}))).into_response(),
    };
    if let Some(v) = req.display_name { existing.display_name = Some(v); }
    if let Some(v) = req.enabled { existing.enabled = v; }
    if let Some(v) = req.config { existing.config = v; }
    match state.providers.update(&realm, &alias, existing).await {
        Ok(idp) => (StatusCode::OK, Json(serde_json::to_value(idp).unwrap())).into_response(),
        Err(_) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"not_found"}))).into_response(),
    }
}

pub async fn delete_idp(
    State(state): State<AdminIdpState>,
    Path((realm, alias)): Path<(String, String)>,
) -> impl IntoResponse {
    match state.providers.delete(&realm, &alias).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"not_found"}))).into_response(),
    }
}

pub fn router(state: AdminIdpState) -> Router {
    Router::new()
        .route("/admin/realms/{realm}/identity-provider/instances", get(list_idps).post(create_idp))
        .route(
            "/admin/realms/{realm}/identity-provider/instances/{alias}",
            get(get_idp).put(update_idp).delete(delete_idp),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keycloak::realm::{RealmRequest, RealmStore};
    use axum::{body::Body, http::Request};
    use serde_json::Value;
    use tower::ServiceExt;

    async fn setup() -> (Router, AdminIdpState) {
        let realms = RealmStore::new();
        realms.create(RealmRequest { id: "r".into(), display_name: None, enabled: None, ssl_required: None, registration_allowed: None, login_with_email_allowed: None, duplicate_emails_allowed: None, access_token_lifespan: None, sso_session_idle_timeout: None }).await.unwrap();
        let state = AdminIdpState::new(realms);
        let app = router(state.clone());
        (app, state)
    }

    async fn body(resp: axum::response::Response) -> Value {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    }

    // upstream: keycloak/keycloak IdentityProviderResourceTest.java:listEmpty
    #[tokio::test]
    async fn list_initially_empty() {
        let (app, _) = setup().await;
        let resp = app.oneshot(Request::builder().method("GET").uri("/admin/realms/r/identity-provider/instances").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let b = body(resp).await;
        assert!(b.as_array().unwrap().is_empty());
    }

    // upstream: keycloak/keycloak IdentityProviderResourceTest.java:createOidcInstance
    #[tokio::test]
    async fn create_oidc_instance() {
        let (app, _) = setup().await;
        let payload = serde_json::json!({"alias":"google","provider_id":"oidc","enabled":true,"config":{"clientId":"abc"}});
        let resp = app.oneshot(Request::builder().method("POST").uri("/admin/realms/r/identity-provider/instances")
            .header("content-type","application/json")
            .body(Body::from(payload.to_string())).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let b = body(resp).await;
        assert_eq!(b["alias"], "google");
        assert_eq!(b["provider_id"], "oidc");
        assert_eq!(b["config"]["clientId"], "abc");
    }

    // upstream: keycloak/keycloak IdentityProviderResourceTest.java:duplicateAliasReturns409
    #[tokio::test]
    async fn duplicate_alias_returns_conflict() {
        let (app, _) = setup().await;
        let payload = serde_json::json!({"alias":"a","provider_id":"oidc"});
        let r1 = app.clone().oneshot(Request::builder().method("POST").uri("/admin/realms/r/identity-provider/instances")
            .header("content-type","application/json").body(Body::from(payload.to_string())).unwrap()).await.unwrap();
        assert_eq!(r1.status(), StatusCode::CREATED);
        let r2 = app.oneshot(Request::builder().method("POST").uri("/admin/realms/r/identity-provider/instances")
            .header("content-type","application/json").body(Body::from(payload.to_string())).unwrap()).await.unwrap();
        assert_eq!(r2.status(), StatusCode::CONFLICT);
    }

    // upstream: keycloak/keycloak IdentityProviderResourceTest.java:getReturnsCreated
    #[tokio::test]
    async fn get_returns_created() {
        let (app, _) = setup().await;
        let payload = serde_json::json!({"alias":"github","provider_id":"github"});
        let _ = app.clone().oneshot(Request::builder().method("POST").uri("/admin/realms/r/identity-provider/instances")
            .header("content-type","application/json").body(Body::from(payload.to_string())).unwrap()).await.unwrap();
        let resp = app.oneshot(Request::builder().method("GET").uri("/admin/realms/r/identity-provider/instances/github").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body(resp).await["alias"], "github");
    }

    // upstream: keycloak/keycloak IdentityProviderResourceTest.java:getMissingReturns404
    #[tokio::test]
    async fn get_missing_returns_404() {
        let (app, _) = setup().await;
        let resp = app.oneshot(Request::builder().method("GET").uri("/admin/realms/r/identity-provider/instances/nope").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // upstream: keycloak/keycloak IdentityProviderResourceTest.java:updateChangesEnabledFlag
    #[tokio::test]
    async fn update_disables_instance() {
        let (app, _) = setup().await;
        let create = serde_json::json!({"alias":"saml","provider_id":"saml","enabled":true});
        let _ = app.clone().oneshot(Request::builder().method("POST").uri("/admin/realms/r/identity-provider/instances")
            .header("content-type","application/json").body(Body::from(create.to_string())).unwrap()).await.unwrap();
        let upd = serde_json::json!({"enabled":false});
        let resp = app.oneshot(Request::builder().method("PUT").uri("/admin/realms/r/identity-provider/instances/saml")
            .header("content-type","application/json").body(Body::from(upd.to_string())).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body(resp).await["enabled"], false);
    }

    // upstream: keycloak/keycloak IdentityProviderResourceTest.java:deleteRemovesInstance
    #[tokio::test]
    async fn delete_removes_instance() {
        let (app, state) = setup().await;
        let payload = serde_json::json!({"alias":"goner","provider_id":"oidc"});
        let _ = app.clone().oneshot(Request::builder().method("POST").uri("/admin/realms/r/identity-provider/instances")
            .header("content-type","application/json").body(Body::from(payload.to_string())).unwrap()).await.unwrap();
        let resp = app.oneshot(Request::builder().method("DELETE").uri("/admin/realms/r/identity-provider/instances/goner").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        assert_eq!(state.providers.count("r").await, 0);
    }

    // upstream: keycloak/keycloak IdentityProviderResourceTest.java:createInUnknownRealm404
    #[tokio::test]
    async fn create_in_unknown_realm_404() {
        let (app, _) = setup().await;
        let payload = serde_json::json!({"alias":"x","provider_id":"oidc"});
        let resp = app.oneshot(Request::builder().method("POST").uri("/admin/realms/ghost/identity-provider/instances")
            .header("content-type","application/json").body(Body::from(payload.to_string())).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
