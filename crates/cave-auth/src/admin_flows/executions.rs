// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../admin/AuthenticationManagementResource.java#executions
//
//! Executions inside an authentication flow:
//!
//! - `GET    /admin/realms/{realm}/authentication/flows/{alias}/executions`
//! - `POST   /admin/realms/{realm}/authentication/flows/{alias}/executions`
//! - `PUT    /admin/realms/{realm}/authentication/flows/{alias}/executions/{id}`
//! - `DELETE /admin/realms/{realm}/authentication/flows/{alias}/executions/{id}`

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::AdminFlowsState;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum ExecutionRequirement {
    Required,
    Alternative,
    Optional,
    Disabled,
    Conditional,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthenticationExecution {
    pub id: String,
    pub provider_id: String,
    pub requirement: ExecutionRequirement,
    pub priority: i32,
    pub flow_alias: String,
    pub authenticator_flow: bool,
}

#[derive(Debug, Deserialize)]
pub struct CreateExecutionRequest {
    pub provider_id: String,
    pub requirement: Option<ExecutionRequirement>,
    pub priority: Option<i32>,
    pub authenticator_flow: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateExecutionRequest {
    pub requirement: Option<ExecutionRequirement>,
    pub priority: Option<i32>,
}

pub async fn list_executions(
    State(state): State<AdminFlowsState>,
    Path((realm, alias)): Path<(String, String)>,
) -> impl IntoResponse {
    if state.realms.get(&realm).await.is_none() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"realm_not_found"}))).into_response();
    }
    if state.flows.get(&realm, &alias).await.is_none() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"flow_not_found"}))).into_response();
    }
    let list = state.executions.list(&realm, &alias).await;
    (StatusCode::OK, Json(serde_json::to_value(list).unwrap())).into_response()
}

pub async fn create_execution(
    State(state): State<AdminFlowsState>,
    Path((realm, alias)): Path<(String, String)>,
    Json(req): Json<CreateExecutionRequest>,
) -> impl IntoResponse {
    if state.flows.get(&realm, &alias).await.is_none() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"flow_not_found"}))).into_response();
    }
    if req.provider_id.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"invalid_request"}))).into_response();
    }
    let exec = AuthenticationExecution {
        id: Uuid::new_v4().to_string(),
        provider_id: req.provider_id,
        requirement: req.requirement.unwrap_or(ExecutionRequirement::Required),
        priority: req.priority.unwrap_or(0),
        flow_alias: alias.clone(),
        authenticator_flow: req.authenticator_flow.unwrap_or(false),
    };
    match state.executions.create(&realm, &alias, exec).await {
        Ok(e) => (StatusCode::CREATED, Json(serde_json::to_value(e).unwrap())).into_response(),
        Err(_) => (StatusCode::CONFLICT, Json(serde_json::json!({"error":"conflict"}))).into_response(),
    }
}

pub async fn update_execution(
    State(state): State<AdminFlowsState>,
    Path((realm, alias, id)): Path<(String, String, String)>,
    Json(req): Json<UpdateExecutionRequest>,
) -> impl IntoResponse {
    let existing = state.executions.list(&realm, &alias).await.into_iter().find(|e| e.id == id);
    let mut existing = match existing {
        Some(e) => e,
        None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"not_found"}))).into_response(),
    };
    if let Some(v) = req.requirement { existing.requirement = v; }
    if let Some(v) = req.priority { existing.priority = v; }
    match state.executions.update(&realm, &alias, &id, existing).await {
        Ok(e) => (StatusCode::OK, Json(serde_json::to_value(e).unwrap())).into_response(),
        Err(_) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"not_found"}))).into_response(),
    }
}

pub async fn delete_execution(
    State(state): State<AdminFlowsState>,
    Path((realm, alias, id)): Path<(String, String, String)>,
) -> impl IntoResponse {
    match state.executions.delete(&realm, &alias, &id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"not_found"}))).into_response(),
    }
}

pub fn router(state: AdminFlowsState) -> Router {
    Router::new()
        .route(
            "/admin/realms/{realm}/authentication/flows/{alias}/executions",
            get(list_executions).post(create_execution),
        )
        .route(
            "/admin/realms/{realm}/authentication/flows/{alias}/executions/{id}",
            axum::routing::put(update_execution).delete(delete_execution),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin_flows::flows::AuthenticationFlow;
    use crate::keycloak::realm::{RealmRequest, RealmStore};
    use axum::{body::Body, http::Request};
    use serde_json::Value;
    use tower::ServiceExt;

    async fn setup() -> (Router, AdminFlowsState) {
        let realms = RealmStore::new();
        realms.create(RealmRequest { id: "r".into(), display_name: None, enabled: None, ssl_required: None, registration_allowed: None, login_with_email_allowed: None, duplicate_emails_allowed: None, access_token_lifespan: None, sso_session_idle_timeout: None }).await.unwrap();
        let state = AdminFlowsState::new(realms);
        state.flows.create("r", AuthenticationFlow {
            alias: "browser".into(), description: None, provider_id: "basic-flow".into(),
            top_level: true, built_in: false,
        }).await.unwrap();
        let app = router(state.clone());
        (app, state)
    }

    async fn body(resp: axum::response::Response) -> Value {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    }

    // upstream: keycloak/keycloak AuthenticationManagementResourceTest.java:listExecutionsEmpty
    #[tokio::test]
    async fn list_executions_empty() {
        let (app, _) = setup().await;
        let resp = app.oneshot(Request::builder().method("GET").uri("/admin/realms/r/authentication/flows/browser/executions").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(body(resp).await.as_array().unwrap().is_empty());
    }

    // upstream: keycloak/keycloak AuthenticationManagementResourceTest.java:createExecution
    #[tokio::test]
    async fn create_execution() {
        let (app, _) = setup().await;
        let payload = serde_json::json!({"provider_id":"auth-username-password-form","requirement":"REQUIRED","priority":10});
        let resp = app.oneshot(Request::builder().method("POST").uri("/admin/realms/r/authentication/flows/browser/executions")
            .header("content-type","application/json").body(Body::from(payload.to_string())).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let b = body(resp).await;
        assert_eq!(b["requirement"], "REQUIRED");
        assert_eq!(b["priority"], 10);
    }

    // upstream: keycloak/keycloak AuthenticationManagementResourceTest.java:listSortedByPriority
    #[tokio::test]
    async fn list_sorted_by_priority() {
        let (app, _) = setup().await;
        for (pid, prio) in [("a", 30), ("b", 10), ("c", 20)] {
            let payload = serde_json::json!({"provider_id": pid, "priority": prio});
            let _ = app.clone().oneshot(Request::builder().method("POST").uri("/admin/realms/r/authentication/flows/browser/executions")
                .header("content-type","application/json").body(Body::from(payload.to_string())).unwrap()).await.unwrap();
        }
        let resp = app.oneshot(Request::builder().method("GET").uri("/admin/realms/r/authentication/flows/browser/executions").body(Body::empty()).unwrap()).await.unwrap();
        let b = body(resp).await;
        let arr = b.as_array().unwrap();
        assert_eq!(arr[0]["priority"], 10);
        assert_eq!(arr[1]["priority"], 20);
        assert_eq!(arr[2]["priority"], 30);
    }

    // upstream: keycloak/keycloak AuthenticationManagementResourceTest.java:updateExecutionRequirement
    #[tokio::test]
    async fn update_execution_requirement() {
        let (app, _) = setup().await;
        let payload = serde_json::json!({"provider_id":"x","requirement":"REQUIRED","priority":5});
        let create_resp = app.clone().oneshot(Request::builder().method("POST").uri("/admin/realms/r/authentication/flows/browser/executions")
            .header("content-type","application/json").body(Body::from(payload.to_string())).unwrap()).await.unwrap();
        let bytes = axum::body::to_bytes(create_resp.into_body(), usize::MAX).await.unwrap();
        let exec: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let id = exec["id"].as_str().unwrap();
        let upd = serde_json::json!({"requirement":"ALTERNATIVE"});
        let uri = format!("/admin/realms/r/authentication/flows/browser/executions/{}", id);
        let resp = app.oneshot(Request::builder().method("PUT").uri(uri)
            .header("content-type","application/json").body(Body::from(upd.to_string())).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body(resp).await["requirement"], "ALTERNATIVE");
    }

    // upstream: keycloak/keycloak AuthenticationManagementResourceTest.java:executionInUnknownFlow404
    #[tokio::test]
    async fn execution_in_unknown_flow_404() {
        let (app, _) = setup().await;
        let payload = serde_json::json!({"provider_id":"x"});
        let resp = app.oneshot(Request::builder().method("POST").uri("/admin/realms/r/authentication/flows/ghost/executions")
            .header("content-type","application/json").body(Body::from(payload.to_string())).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
