// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../admin/IdentityProviderResource.java#mappers
//
//! Identity-provider mappers — per-instance attribute / role mappers.
//!
//! - `GET    /admin/realms/{realm}/identity-provider/instances/{alias}/mappers`
//! - `POST   /admin/realms/{realm}/identity-provider/instances/{alias}/mappers`
//! - `GET    /admin/realms/{realm}/identity-provider/instances/{alias}/mappers/{id}`
//! - `DELETE /admin/realms/{realm}/identity-provider/instances/{alias}/mappers/{id}`

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use super::AdminIdpState;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IdentityProviderMapper {
    pub id: String,
    pub name: String,
    pub identity_provider_mapper: String,
    #[serde(default)]
    pub config: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateMapperRequest {
    pub name: String,
    pub identity_provider_mapper: String,
    #[serde(default)]
    pub config: HashMap<String, String>,
}

pub async fn list_mappers(
    State(state): State<AdminIdpState>,
    Path((realm, alias)): Path<(String, String)>,
) -> impl IntoResponse {
    if state.providers.get(&realm, &alias).await.is_none() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"idp_not_found"}))).into_response();
    }
    let list = state.mappers.list(&realm, &alias).await;
    (StatusCode::OK, Json(serde_json::to_value(list).unwrap())).into_response()
}

pub async fn create_mapper(
    State(state): State<AdminIdpState>,
    Path((realm, alias)): Path<(String, String)>,
    Json(req): Json<CreateMapperRequest>,
) -> impl IntoResponse {
    if state.providers.get(&realm, &alias).await.is_none() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"idp_not_found"}))).into_response();
    }
    let mapper = IdentityProviderMapper {
        id: Uuid::new_v4().to_string(),
        name: req.name,
        identity_provider_mapper: req.identity_provider_mapper,
        config: req.config,
    };
    match state.mappers.create(&realm, &alias, mapper).await {
        Ok(m) => (StatusCode::CREATED, Json(serde_json::to_value(m).unwrap())).into_response(),
        Err(_) => (StatusCode::CONFLICT, Json(serde_json::json!({"error":"conflict"}))).into_response(),
    }
}

pub async fn get_mapper(
    State(state): State<AdminIdpState>,
    Path((realm, alias, id)): Path<(String, String, String)>,
) -> impl IntoResponse {
    match state.mappers.get(&realm, &alias, &id).await {
        Some(m) => (StatusCode::OK, Json(serde_json::to_value(m).unwrap())).into_response(),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"not_found"}))).into_response(),
    }
}

pub async fn delete_mapper(
    State(state): State<AdminIdpState>,
    Path((realm, alias, id)): Path<(String, String, String)>,
) -> impl IntoResponse {
    match state.mappers.delete(&realm, &alias, &id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"not_found"}))).into_response(),
    }
}

pub fn router(state: AdminIdpState) -> Router {
    Router::new()
        .route(
            "/admin/realms/{realm}/identity-provider/instances/{alias}/mappers",
            get(list_mappers).post(create_mapper),
        )
        .route(
            "/admin/realms/{realm}/identity-provider/instances/{alias}/mappers/{id}",
            get(get_mapper).delete(delete_mapper),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin_idp::instances::IdentityProvider;
    use crate::keycloak::realm::{RealmRequest, RealmStore};
    use axum::{body::Body, http::Request};
    use chrono::Utc;
    use tower::ServiceExt;

    async fn setup() -> (Router, AdminIdpState) {
        let realms = RealmStore::new();
        realms.create(RealmRequest { id: "r".into(), display_name: None, enabled: None, ssl_required: None, registration_allowed: None, login_with_email_allowed: None, duplicate_emails_allowed: None, access_token_lifespan: None, sso_session_idle_timeout: None }).await.unwrap();
        let state = AdminIdpState::new(realms);
        state.providers.create("r", IdentityProvider {
            alias: "github".into(), display_name: None, provider_id: "github".into(),
            enabled: true, config: Default::default(), created_at: Utc::now(),
        }).await.unwrap();
        let app = router(state.clone());
        (app, state)
    }

    // upstream: keycloak/keycloak IdentityProviderResourceTest.java:listMappersOnFreshInstance
    #[tokio::test]
    async fn list_mappers_empty() {
        let (app, _) = setup().await;
        let resp = app.oneshot(Request::builder().method("GET").uri("/admin/realms/r/identity-provider/instances/github/mappers").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // upstream: keycloak/keycloak IdentityProviderResourceTest.java:createMapperReturnsCreated
    #[tokio::test]
    async fn create_mapper_returns_created() {
        let (app, _) = setup().await;
        let payload = serde_json::json!({"name":"email-attr","identity_provider_mapper":"oidc-user-attribute-idp-mapper","config":{}});
        let resp = app.oneshot(Request::builder().method("POST").uri("/admin/realms/r/identity-provider/instances/github/mappers")
            .header("content-type","application/json").body(Body::from(payload.to_string())).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    // upstream: keycloak/keycloak IdentityProviderResourceTest.java:createMapperUnknownIdp404
    #[tokio::test]
    async fn create_mapper_unknown_idp_404() {
        let (app, _) = setup().await;
        let payload = serde_json::json!({"name":"x","identity_provider_mapper":"oidc-user-attribute-idp-mapper"});
        let resp = app.oneshot(Request::builder().method("POST").uri("/admin/realms/r/identity-provider/instances/ghost/mappers")
            .header("content-type","application/json").body(Body::from(payload.to_string())).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // upstream: keycloak/keycloak IdentityProviderResourceTest.java:deleteMapperFlow
    #[tokio::test]
    async fn create_then_delete_mapper() {
        let (app, _) = setup().await;
        let payload = serde_json::json!({"name":"role-map","identity_provider_mapper":"oidc-role-idp-mapper"});
        let create = app.clone().oneshot(Request::builder().method("POST").uri("/admin/realms/r/identity-provider/instances/github/mappers")
            .header("content-type","application/json").body(Body::from(payload.to_string())).unwrap()).await.unwrap();
        let bytes = axum::body::to_bytes(create.into_body(), usize::MAX).await.unwrap();
        let m: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let id = m["id"].as_str().unwrap();
        let uri = format!("/admin/realms/r/identity-provider/instances/github/mappers/{}", id);
        let resp = app.oneshot(Request::builder().method("DELETE").uri(&uri).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }
}
