// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../admin/AuthenticationManagementResource.java#required_actions
//
//! Required-action provider list (built-in actions Keycloak ships with).
//!
//! - `GET /admin/realms/{realm}/authentication/required-actions`
//! - `PUT /admin/realms/{realm}/authentication/required-actions/{alias}`
//!
//! Implements the read-side + enable/disable toggle used by the portal's
//! `auth/required-actions` page. Built-in actions cannot be deleted.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use super::AdminFlowsState;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RequiredAction {
    pub alias: String,
    pub name: String,
    pub provider_id: String,
    pub enabled: bool,
    pub default_action: bool,
    pub priority: i32,
}

/// Keycloak v22 built-in required-action providers.
pub fn builtin_actions() -> Vec<RequiredAction> {
    vec![
        RequiredAction { alias: "CONFIGURE_TOTP".into(),         name: "Configure OTP".into(),          provider_id: "CONFIGURE_TOTP".into(),        enabled: true,  default_action: false, priority: 10 },
        RequiredAction { alias: "TERMS_AND_CONDITIONS".into(),   name: "Terms and Conditions".into(),   provider_id: "TERMS_AND_CONDITIONS".into(),  enabled: false, default_action: false, priority: 20 },
        RequiredAction { alias: "UPDATE_PASSWORD".into(),        name: "Update Password".into(),        provider_id: "UPDATE_PASSWORD".into(),       enabled: true,  default_action: false, priority: 30 },
        RequiredAction { alias: "UPDATE_PROFILE".into(),         name: "Update Profile".into(),         provider_id: "UPDATE_PROFILE".into(),        enabled: true,  default_action: false, priority: 40 },
        RequiredAction { alias: "VERIFY_EMAIL".into(),           name: "Verify Email".into(),           provider_id: "VERIFY_EMAIL".into(),          enabled: true,  default_action: false, priority: 50 },
        RequiredAction { alias: "VERIFY_PROFILE".into(),         name: "Verify Profile".into(),         provider_id: "VERIFY_PROFILE".into(),        enabled: true,  default_action: false, priority: 55 },
        RequiredAction { alias: "WEBAUTHN_REGISTER".into(),      name: "WebAuthn Register".into(),      provider_id: "webauthn-register".into(),     enabled: false, default_action: false, priority: 60 },
        RequiredAction { alias: "WEBAUTHN_PASSWORDLESS_REGISTER".into(), name: "WebAuthn Register Passwordless".into(), provider_id: "webauthn-register-passwordless".into(), enabled: false, default_action: false, priority: 70 },
        RequiredAction { alias: "UPDATE_USER_LOCALE".into(),     name: "Update Locale".into(),          provider_id: "update_user_locale".into(),    enabled: true,  default_action: false, priority: 80 },
        RequiredAction { alias: "DELETE_ACCOUNT".into(),         name: "Delete Account".into(),         provider_id: "delete_account".into(),        enabled: false, default_action: false, priority: 90 },
    ]
}

#[derive(Debug, Deserialize)]
pub struct UpdateRequiredAction {
    pub enabled: Option<bool>,
    pub default_action: Option<bool>,
    pub priority: Option<i32>,
}

#[derive(Clone, Default)]
struct OverridesStore {
    inner: std::sync::Arc<tokio::sync::RwLock<std::collections::HashMap<(String, String), RequiredAction>>>,
}

impl OverridesStore {
    fn singleton() -> &'static OverridesStore {
        use std::sync::OnceLock;
        static S: OnceLock<OverridesStore> = OnceLock::new();
        S.get_or_init(OverridesStore::default)
    }
}

pub async fn list_required_actions(
    State(state): State<AdminFlowsState>,
    Path(realm): Path<String>,
) -> impl IntoResponse {
    if state.realms.get(&realm).await.is_none() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"realm_not_found"}))).into_response();
    }
    let mut actions = builtin_actions();
    let store = OverridesStore::singleton();
    let r = store.inner.read().await;
    for a in &mut actions {
        if let Some(ovr) = r.get(&(realm.clone(), a.alias.clone())) {
            *a = ovr.clone();
        }
    }
    actions.sort_by_key(|a| a.priority);
    (StatusCode::OK, Json(serde_json::to_value(actions).unwrap())).into_response()
}

pub async fn update_required_action(
    State(state): State<AdminFlowsState>,
    Path((realm, alias)): Path<(String, String)>,
    Json(req): Json<UpdateRequiredAction>,
) -> impl IntoResponse {
    if state.realms.get(&realm).await.is_none() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"realm_not_found"}))).into_response();
    }
    let mut a = match builtin_actions().into_iter().find(|x| x.alias == alias) {
        Some(a) => a,
        None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"unknown_required_action"}))).into_response(),
    };
    if let Some(v) = req.enabled { a.enabled = v; }
    if let Some(v) = req.default_action { a.default_action = v; }
    if let Some(v) = req.priority { a.priority = v; }
    let store = OverridesStore::singleton();
    store.inner.write().await.insert((realm, alias), a.clone());
    (StatusCode::OK, Json(serde_json::to_value(a).unwrap())).into_response()
}

pub fn router(state: AdminFlowsState) -> Router {
    Router::new()
        .route("/admin/realms/{realm}/authentication/required-actions", get(list_required_actions))
        .route("/admin/realms/{realm}/authentication/required-actions/{alias}", put(update_required_action))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keycloak::realm::{RealmRequest, RealmStore};
    use axum::{body::Body, http::Request};
    use serde_json::Value;
    use tower::ServiceExt;

    async fn setup() -> Router {
        let realms = RealmStore::new();
        realms.create(RealmRequest { id: "ra".into(), display_name: None, enabled: None, ssl_required: None, registration_allowed: None, login_with_email_allowed: None, duplicate_emails_allowed: None, access_token_lifespan: None, sso_session_idle_timeout: None }).await.unwrap();
        router(AdminFlowsState::new(realms))
    }

    async fn body(resp: axum::response::Response) -> Value {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    }

    // upstream: keycloak/keycloak AuthenticationManagementResourceTest.java:listRequiredActionsHasBuiltins
    #[tokio::test]
    async fn list_returns_builtins() {
        let app = setup().await;
        let resp = app.oneshot(Request::builder().method("GET").uri("/admin/realms/ra/authentication/required-actions").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let arr = body(resp).await;
        let names: Vec<&str> = arr.as_array().unwrap().iter().map(|a| a["alias"].as_str().unwrap()).collect();
        assert!(names.contains(&"CONFIGURE_TOTP"));
        assert!(names.contains(&"VERIFY_EMAIL"));
    }

    // upstream: keycloak/keycloak AuthenticationManagementResourceTest.java:disableRequiredAction
    #[tokio::test]
    async fn disable_required_action() {
        let app = setup().await;
        let upd = serde_json::json!({"enabled": false});
        let resp = app.oneshot(Request::builder().method("PUT").uri("/admin/realms/ra/authentication/required-actions/VERIFY_EMAIL")
            .header("content-type","application/json").body(Body::from(upd.to_string())).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body(resp).await["enabled"], false);
    }

    // upstream: keycloak/keycloak AuthenticationManagementResourceTest.java:updateUnknownAction404
    #[tokio::test]
    async fn update_unknown_action_404() {
        let app = setup().await;
        let upd = serde_json::json!({"enabled": true});
        let resp = app.oneshot(Request::builder().method("PUT").uri("/admin/realms/ra/authentication/required-actions/NOPE")
            .header("content-type","application/json").body(Body::from(upd.to_string())).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
