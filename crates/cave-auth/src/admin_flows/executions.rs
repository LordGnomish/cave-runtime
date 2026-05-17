// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 CAVE Runtime contributors
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/services/resources/admin/AuthenticationManagementResource.java
//         (`@Path("flows/{flowAlias}/executions")` and
//          `@Path("flows/{flowAlias}/executions/execution")`).
//
// An `AuthenticationExecutionInfoRepresentation` describes one step in a
// flow tree. Keycloak persists `AuthenticationExecutionModel` rows with
// `requirement`, `authenticator`, `priority`, `parentFlow`. Our in-memory
// store keeps the same shape so the admin UI handlers can round-trip.

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

/// `requirement` enum per Keycloak. The wire format uppercases.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Requirement {
    #[serde(rename = "REQUIRED")]
    Required,
    #[serde(rename = "ALTERNATIVE")]
    Alternative,
    #[serde(rename = "DISABLED")]
    Disabled,
    #[serde(rename = "CONDITIONAL")]
    Conditional,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuthenticationExecution {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Provider id of the authenticator plugin, e.g.
    /// `auth-username-password-form`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authenticator: Option<String>,
    pub requirement: Requirement,
    #[serde(default)]
    pub priority: i32,
    /// Flow alias this execution belongs to.
    #[serde(rename = "parentFlow")]
    pub parent_flow: String,
    /// True iff this row is itself a sub-flow rather than a leaf.
    #[serde(default, rename = "authenticatorFlow")]
    pub authenticator_flow: bool,
}

#[derive(Debug, Default)]
pub struct ExecutionStore {
    // (realm, flow_alias, exec_id) -> exec
    inner: DashMap<(String, String, String), AuthenticationExecution>,
}

impl ExecutionStore {
    pub fn new() -> Self {
        Self { inner: DashMap::new() }
    }

    pub fn list(&self, realm: &str, flow_alias: &str) -> Vec<AuthenticationExecution> {
        let mut out: Vec<_> = self
            .inner
            .iter()
            .filter(|kv| kv.key().0 == realm && kv.key().1 == flow_alias)
            .map(|kv| kv.value().clone())
            .collect();
        out.sort_by_key(|e| e.priority);
        out
    }

    pub fn create(&self, realm: &str, flow_alias: &str, mut e: AuthenticationExecution) -> String {
        let id = e.id.clone().unwrap_or_else(|| Uuid::new_v4().to_string());
        e.id = Some(id.clone());
        e.parent_flow = flow_alias.to_string();
        self.inner.insert((realm.into(), flow_alias.into(), id.clone()), e);
        id
    }

    pub fn delete(&self, realm: &str, flow_alias: &str, id: &str) -> bool {
        self.inner
            .remove(&(realm.into(), flow_alias.into(), id.into()))
            .is_some()
    }
}

/// `GET /admin/realms/{realm}/authentication/flows/{flowAlias}/executions`
pub async fn list_executions(
    State(store): State<Arc<ExecutionStore>>,
    Path((realm, flow_alias)): Path<(String, String)>,
) -> impl IntoResponse {
    (StatusCode::OK, Json(store.list(&realm, &flow_alias)))
}

/// `POST /admin/realms/{realm}/authentication/flows/{flowAlias}/executions/execution`
pub async fn add_execution(
    State(store): State<Arc<ExecutionStore>>,
    Path((realm, flow_alias)): Path<(String, String)>,
    Json(e): Json<AuthenticationExecution>,
) -> impl IntoResponse {
    let id = store.create(&realm, &flow_alias, e);
    let location = format!(
        "/admin/realms/{realm}/authentication/flows/{flow_alias}/executions/{id}"
    );
    (
        StatusCode::CREATED,
        [(axum::http::header::LOCATION, location)],
        Json(json!({ "id": id })),
    )
        .into_response()
}

/// `DELETE /admin/realms/{realm}/authentication/executions/{id}` — Keycloak
/// looks up by execution id alone, but we keep the flow-alias path for
/// router-state simplicity (the admin console always knows the flow it
/// just listed).
pub async fn delete_execution(
    State(store): State<Arc<ExecutionStore>>,
    Path((realm, flow_alias, id)): Path<(String, String, String)>,
) -> impl IntoResponse {
    if store.delete(&realm, &flow_alias, &id) {
        StatusCode::NO_CONTENT.into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "execution not found", "id": id })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin_flows::flows::AuthenticationFlow;

    #[test]
    fn list_sorts_by_priority() {
        let s = ExecutionStore::new();
        let mk = |p: i32| AuthenticationExecution {
            id: None,
            authenticator: Some("auth-username-password-form".into()),
            requirement: Requirement::Required,
            priority: p,
            parent_flow: "browser-custom".into(),
            authenticator_flow: false,
        };
        s.create("master", "browser-custom", mk(30));
        s.create("master", "browser-custom", mk(10));
        s.create("master", "browser-custom", mk(20));
        let out = s.list("master", "browser-custom");
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].priority, 10);
        assert_eq!(out[2].priority, 30);
    }

    #[test]
    fn flow_alias_threaded_into_execution() {
        let s = ExecutionStore::new();
        let id = s.create(
            "master",
            "browser-custom",
            AuthenticationExecution {
                id: None,
                authenticator: None,
                requirement: Requirement::Alternative,
                priority: 5,
                parent_flow: "ignored-by-store".into(),
                authenticator_flow: false,
            },
        );
        let list = s.list("master", "browser-custom");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].parent_flow, "browser-custom");
        assert_eq!(list[0].id.as_deref(), Some(id.as_str()));
        // Reference the sibling module so the keycloak-source stamp on it is
        // exercised by the compiler.
        let _f: Option<AuthenticationFlow> = None;
    }
}
