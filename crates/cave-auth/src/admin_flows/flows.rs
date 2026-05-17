// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 CAVE Runtime contributors
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/services/resources/admin/AuthenticationManagementResource.java
//         (`@Path("flows")` sub-resource — `getFlows`, `createFlow`,
//         `getFlow`, `updateFlow`, `deleteFlow`).
//
// An `AuthenticationFlow` is a tree of `AuthenticationExecution`s.
// `topLevel = true` marks the entry-point flows (`browser`, `direct grant`,
// `registration`, …). Built-in flows have `builtIn = true` and cannot be
// deleted via this endpoint (Keycloak returns 400). We expose the same
// validation surface.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuthenticationFlow {
    /// Server-assigned UUID. None on create POST, always Some on
    /// GET/PUT/DELETE responses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub alias: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `basic-flow` or `form-flow`.
    #[serde(default = "default_provider_id", rename = "providerId")]
    pub provider_id: String,
    /// True iff this is an entry-point flow.
    #[serde(default, rename = "topLevel")]
    pub top_level: bool,
    /// True iff this flow ships with Keycloak — it can be cloned but not
    /// deleted.
    #[serde(default, rename = "builtIn")]
    pub built_in: bool,
}

fn default_provider_id() -> String {
    "basic-flow".into()
}

#[derive(Debug, Default)]
pub struct AuthenticationFlowStore {
    // (realm, flow_id) -> flow
    inner: DashMap<(String, String), AuthenticationFlow>,
}

impl AuthenticationFlowStore {
    pub fn new() -> Self {
        Self { inner: DashMap::new() }
    }

    pub fn list(&self, realm: &str) -> Vec<AuthenticationFlow> {
        let mut out: Vec<_> = self
            .inner
            .iter()
            .filter(|kv| kv.key().0 == realm)
            .map(|kv| kv.value().clone())
            .collect();
        out.sort_by(|a, b| a.alias.cmp(&b.alias));
        out
    }

    pub fn get(&self, realm: &str, id: &str) -> Option<AuthenticationFlow> {
        self.inner.get(&(realm.into(), id.into())).map(|v| v.clone())
    }

    /// Look up by alias (used by the `executions` sub-resource which
    /// addresses flows by their alias not their id, matching Keycloak).
    pub fn find_by_alias(&self, realm: &str, alias: &str) -> Option<AuthenticationFlow> {
        self.inner
            .iter()
            .find(|kv| kv.key().0 == realm && kv.value().alias == alias)
            .map(|kv| kv.value().clone())
    }

    /// Returns assigned id.
    pub fn create(&self, realm: &str, mut f: AuthenticationFlow) -> String {
        let id = f.id.clone().unwrap_or_else(|| Uuid::new_v4().to_string());
        f.id = Some(id.clone());
        self.inner.insert((realm.into(), id.clone()), f);
        id
    }

    pub fn update(&self, realm: &str, id: &str, mut f: AuthenticationFlow) -> bool {
        let key = (realm.to_string(), id.to_string());
        if !self.inner.contains_key(&key) {
            return false;
        }
        f.id = Some(id.to_string());
        self.inner.insert(key, f);
        true
    }

    pub fn delete(&self, realm: &str, id: &str) -> bool {
        self.inner.remove(&(realm.into(), id.into())).is_some()
    }
}

/// `GET /admin/realms/{realm}/authentication/flows`
pub async fn list_flows(
    State(store): State<Arc<AuthenticationFlowStore>>,
    Path(realm): Path<String>,
) -> impl IntoResponse {
    (StatusCode::OK, Json(store.list(&realm)))
}

/// `POST /admin/realms/{realm}/authentication/flows`
pub async fn create_flow(
    State(store): State<Arc<AuthenticationFlowStore>>,
    Path(realm): Path<String>,
    Json(f): Json<AuthenticationFlow>,
) -> impl IntoResponse {
    if f.alias.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "alias is required" })),
        )
            .into_response();
    }
    if store.find_by_alias(&realm, &f.alias).is_some() {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "error": "flow alias already exists", "alias": f.alias })),
        )
            .into_response();
    }
    let id = store.create(&realm, f);
    let location = format!("/admin/realms/{realm}/authentication/flows/{id}");
    (
        StatusCode::CREATED,
        [(axum::http::header::LOCATION, location)],
        Json(json!({ "id": id })),
    )
        .into_response()
}

/// `GET /admin/realms/{realm}/authentication/flows/{id}`
pub async fn get_flow(
    State(store): State<Arc<AuthenticationFlowStore>>,
    Path((realm, id)): Path<(String, String)>,
) -> impl IntoResponse {
    match store.get(&realm, &id) {
        Some(f) => (StatusCode::OK, Json(f)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "flow not found", "id": id })),
        )
            .into_response(),
    }
}

/// `PUT /admin/realms/{realm}/authentication/flows/{id}`
pub async fn update_flow(
    State(store): State<Arc<AuthenticationFlowStore>>,
    Path((realm, id)): Path<(String, String)>,
    Json(f): Json<AuthenticationFlow>,
) -> impl IntoResponse {
    let existing = match store.get(&realm, &id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "flow not found", "id": id })),
            )
                .into_response();
        }
    };
    // Keycloak refuses to mutate the `builtIn` bit on a built-in flow.
    if existing.built_in && !f.built_in {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "cannot un-mark a built-in flow", "id": id })),
        )
            .into_response();
    }
    store.update(&realm, &id, f);
    StatusCode::NO_CONTENT.into_response()
}

/// `DELETE /admin/realms/{realm}/authentication/flows/{id}` — built-in
/// flows reject with 400 (matches Keycloak `removeFlow`).
pub async fn delete_flow(
    State(store): State<Arc<AuthenticationFlowStore>>,
    Path((realm, id)): Path<(String, String)>,
) -> impl IntoResponse {
    match store.get(&realm, &id) {
        Some(f) if f.built_in => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "cannot delete built-in flow", "id": id })),
        )
            .into_response(),
        Some(_) => {
            store.delete(&realm, &id);
            StatusCode::NO_CONTENT.into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "flow not found", "id": id })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(alias: &str) -> AuthenticationFlow {
        AuthenticationFlow {
            id: None,
            alias: alias.into(),
            description: Some("test".into()),
            provider_id: "basic-flow".into(),
            top_level: true,
            built_in: false,
        }
    }

    #[test]
    fn create_assigns_uuid() {
        let s = AuthenticationFlowStore::new();
        let id = s.create("master", sample("browser-custom"));
        let f = s.get("master", &id).unwrap();
        assert_eq!(f.id.as_deref(), Some(id.as_str()));
    }

    #[test]
    fn find_by_alias() {
        let s = AuthenticationFlowStore::new();
        let id = s.create("master", sample("registration-custom"));
        let f = s.find_by_alias("master", "registration-custom").unwrap();
        assert_eq!(f.id.as_deref(), Some(id.as_str()));
        assert!(s.find_by_alias("master", "nope").is_none());
    }

    #[test]
    fn update_missing_returns_false() {
        let s = AuthenticationFlowStore::new();
        assert!(!s.update("master", "no-id", sample("x")));
    }
}
