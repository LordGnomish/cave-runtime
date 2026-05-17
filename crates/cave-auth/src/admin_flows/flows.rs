// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../admin/AuthenticationManagementResource.java#flows
//
//! Authentication-flow CRUD:
//!
//! - `GET    /admin/realms/{realm}/authentication/flows`
//! - `POST   /admin/realms/{realm}/authentication/flows`
//! - `GET    /admin/realms/{realm}/authentication/flows/{alias}`
//! - `PUT    /admin/realms/{realm}/authentication/flows/{alias}`
//! - `DELETE /admin/realms/{realm}/authentication/flows/{alias}`

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use super::AdminFlowsState;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthenticationFlow {
    pub alias: String,
    pub description: Option<String>,
    pub provider_id: String,
    pub top_level: bool,
    pub built_in: bool,
}

#[derive(Debug, Deserialize)]
pub struct CreateFlowRequest {
    pub alias: String,
    pub description: Option<String>,
    pub provider_id: Option<String>,
    pub top_level: Option<bool>,
    pub built_in: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateFlowRequest {
    pub description: Option<String>,
    pub top_level: Option<bool>,
}

pub async fn list_flows(
    State(state): State<AdminFlowsState>,
    Path(realm): Path<String>,
) -> impl IntoResponse {
    if state.realms.get(&realm).await.is_none() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"realm_not_found"}))).into_response();
    }
    let list = state.flows.list(&realm).await;
    (StatusCode::OK, Json(serde_json::to_value(list).unwrap())).into_response()
}

pub async fn create_flow(
    State(state): State<AdminFlowsState>,
    Path(realm): Path<String>,
    Json(req): Json<CreateFlowRequest>,
) -> impl IntoResponse {
    if state.realms.get(&realm).await.is_none() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"realm_not_found"}))).into_response();
    }
    if req.alias.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"invalid_request"}))).into_response();
    }
    let flow = AuthenticationFlow {
        alias: req.alias,
        description: req.description,
        provider_id: req.provider_id.unwrap_or_else(|| "basic-flow".into()),
        top_level: req.top_level.unwrap_or(true),
        built_in: req.built_in.unwrap_or(false),
    };
    match state.flows.create(&realm, flow).await {
        Ok(f) => (StatusCode::CREATED, Json(serde_json::to_value(f).unwrap())).into_response(),
        Err("conflict") => (StatusCode::CONFLICT, Json(serde_json::json!({"error":"alias_exists"}))).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error":"server_error"}))).into_response(),
    }
}

pub async fn get_flow(
    State(state): State<AdminFlowsState>,
    Path((realm, alias)): Path<(String, String)>,
) -> impl IntoResponse {
    match state.flows.get(&realm, &alias).await {
        Some(f) => (StatusCode::OK, Json(serde_json::to_value(f).unwrap())).into_response(),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"not_found"}))).into_response(),
    }
}

pub async fn update_flow(
    State(state): State<AdminFlowsState>,
    Path((realm, alias)): Path<(String, String)>,
    Json(req): Json<UpdateFlowRequest>,
) -> impl IntoResponse {
    let mut existing = match state.flows.get(&realm, &alias).await {
        Some(f) => f,
        None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"not_found"}))).into_response(),
    };
    if existing.built_in {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"built_in_flow"}))).into_response();
    }
    if let Some(v) = req.description { existing.description = Some(v); }
    if let Some(v) = req.top_level { existing.top_level = v; }
    match state.flows.update(&realm, &alias, existing).await {
        Ok(f) => (StatusCode::OK, Json(serde_json::to_value(f).unwrap())).into_response(),
        Err(_) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"not_found"}))).into_response(),
    }
}

pub async fn delete_flow(
    State(state): State<AdminFlowsState>,
    Path((realm, alias)): Path<(String, String)>,
) -> impl IntoResponse {
    let f = match state.flows.get(&realm, &alias).await {
        Some(f) => f,
        None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"not_found"}))).into_response(),
    };
    if f.built_in {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"built_in_flow"}))).into_response();
    }
    state.flows.delete(&realm, &alias).await.ok();
    StatusCode::NO_CONTENT.into_response()
}

pub fn router(state: AdminFlowsState) -> Router {
    Router::new()
        .route("/admin/realms/{realm}/authentication/flows", get(list_flows).post(create_flow))
        .route(
            "/admin/realms/{realm}/authentication/flows/{alias}",
            get(get_flow).put(update_flow).delete(delete_flow),
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

    async fn setup() -> (Router, AdminFlowsState) {
        let realms = RealmStore::new();
        realms.create(RealmRequest { id: "r".into(), display_name: None, enabled: None, ssl_required: None, registration_allowed: None, login_with_email_allowed: None, duplicate_emails_allowed: None, access_token_lifespan: None, sso_session_idle_timeout: None }).await.unwrap();
        let state = AdminFlowsState::new(realms);
        let app = router(state.clone());
        (app, state)
    }

    async fn body(resp: axum::response::Response) -> Value {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    }

    // upstream: keycloak/keycloak AuthenticationManagementResourceTest.java:listEmpty
    #[tokio::test]
    async fn list_flows_initially_empty() {
        let (app, _) = setup().await;
        let resp = app.oneshot(Request::builder().method("GET").uri("/admin/realms/r/authentication/flows").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(body(resp).await.as_array().unwrap().is_empty());
    }

    // upstream: keycloak/keycloak AuthenticationManagementResourceTest.java:createFlow
    #[tokio::test]
    async fn create_flow() {
        let (app, _) = setup().await;
        let payload = serde_json::json!({"alias":"browser","description":"browser flow"});
        let resp = app.oneshot(Request::builder().method("POST").uri("/admin/realms/r/authentication/flows")
            .header("content-type","application/json").body(Body::from(payload.to_string())).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let b = body(resp).await;
        assert_eq!(b["alias"], "browser");
        assert_eq!(b["top_level"], true);
    }

    // upstream: keycloak/keycloak AuthenticationManagementResourceTest.java:duplicateFlowReturns409
    #[tokio::test]
    async fn duplicate_flow_returns_conflict() {
        let (app, _) = setup().await;
        let payload = serde_json::json!({"alias":"dup"});
        let _ = app.clone().oneshot(Request::builder().method("POST").uri("/admin/realms/r/authentication/flows")
            .header("content-type","application/json").body(Body::from(payload.to_string())).unwrap()).await.unwrap();
        let resp = app.oneshot(Request::builder().method("POST").uri("/admin/realms/r/authentication/flows")
            .header("content-type","application/json").body(Body::from(payload.to_string())).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    // upstream: keycloak/keycloak AuthenticationManagementResourceTest.java:updateFlow
    #[tokio::test]
    async fn update_flow_description() {
        let (app, _) = setup().await;
        let create = serde_json::json!({"alias":"f1","description":"first"});
        let _ = app.clone().oneshot(Request::builder().method("POST").uri("/admin/realms/r/authentication/flows")
            .header("content-type","application/json").body(Body::from(create.to_string())).unwrap()).await.unwrap();
        let upd = serde_json::json!({"description":"second"});
        let resp = app.oneshot(Request::builder().method("PUT").uri("/admin/realms/r/authentication/flows/f1")
            .header("content-type","application/json").body(Body::from(upd.to_string())).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body(resp).await["description"], "second");
    }

    // upstream: keycloak/keycloak AuthenticationManagementResourceTest.java:deleteFlow
    #[tokio::test]
    async fn delete_flow() {
        let (app, _) = setup().await;
        let create = serde_json::json!({"alias":"f2"});
        let _ = app.clone().oneshot(Request::builder().method("POST").uri("/admin/realms/r/authentication/flows")
            .header("content-type","application/json").body(Body::from(create.to_string())).unwrap()).await.unwrap();
        let resp = app.oneshot(Request::builder().method("DELETE").uri("/admin/realms/r/authentication/flows/f2").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    // upstream: keycloak/keycloak AuthenticationManagementResourceTest.java:builtInFlowsAreLocked
    #[tokio::test]
    async fn built_in_flows_cannot_be_deleted() {
        let (app, state) = setup().await;
        state.flows.create("r", AuthenticationFlow {
            alias: "browser-builtin".into(), description: None,
            provider_id: "basic-flow".into(), top_level: true, built_in: true,
        }).await.unwrap();
        let resp = app.oneshot(Request::builder().method("DELETE").uri("/admin/realms/r/authentication/flows/browser-builtin").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // upstream: keycloak/keycloak AuthenticationManagementResourceTest.java:unknownRealm404
    #[tokio::test]
    async fn unknown_realm_returns_404() {
        let (app, _) = setup().await;
        let resp = app.oneshot(Request::builder().method("GET").uri("/admin/realms/ghost/authentication/flows").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
