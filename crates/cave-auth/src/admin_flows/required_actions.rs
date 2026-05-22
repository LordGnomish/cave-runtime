// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 CAVE Runtime contributors
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/services/resources/admin/AuthenticationManagementResource.java
//         (`@Path("required-actions")` sub-resource — `getRequiredActions`,
//         `updateRequiredAction`, `removeRequiredAction`).
//
// Required actions are global per-realm settings: `UPDATE_PASSWORD`,
// `CONFIGURE_TOTP`, `VERIFY_EMAIL`, … The admin UI lets you toggle
// `enabled`, `defaultAction`, `priority`. Plugin id is the alias.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RequiredActionProvider {
    pub alias: String,
    pub name: String,
    #[serde(rename = "providerId")]
    pub provider_id: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, rename = "defaultAction")]
    pub default_action: bool,
    #[serde(default)]
    pub priority: i32,
}

#[derive(Debug, Default)]
pub struct RequiredActionStore {
    inner: DashMap<(String, String), RequiredActionProvider>,
}

impl RequiredActionStore {
    pub fn new() -> Self {
        Self {
            inner: DashMap::new(),
        }
    }

    pub fn list(&self, realm: &str) -> Vec<RequiredActionProvider> {
        let mut out: Vec<_> = self
            .inner
            .iter()
            .filter(|kv| kv.key().0 == realm)
            .map(|kv| kv.value().clone())
            .collect();
        out.sort_by_key(|r| r.priority);
        out
    }

    pub fn get(&self, realm: &str, alias: &str) -> Option<RequiredActionProvider> {
        self.inner
            .get(&(realm.into(), alias.into()))
            .map(|v| v.clone())
    }

    /// Upsert.
    pub fn put(&self, realm: &str, r: RequiredActionProvider) {
        self.inner.insert((realm.into(), r.alias.clone()), r);
    }

    pub fn delete(&self, realm: &str, alias: &str) -> bool {
        self.inner.remove(&(realm.into(), alias.into())).is_some()
    }
}

/// `GET /admin/realms/{realm}/authentication/required-actions`
pub async fn list_required_actions(
    State(store): State<Arc<RequiredActionStore>>,
    Path(realm): Path<String>,
) -> impl IntoResponse {
    (StatusCode::OK, Json(store.list(&realm)))
}

/// `GET /admin/realms/{realm}/authentication/required-actions/{alias}`
pub async fn get_required_action(
    State(store): State<Arc<RequiredActionStore>>,
    Path((realm, alias)): Path<(String, String)>,
) -> impl IntoResponse {
    match store.get(&realm, &alias) {
        Some(r) => (StatusCode::OK, Json(r)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "required action not found", "alias": alias })),
        )
            .into_response(),
    }
}

/// `PUT /admin/realms/{realm}/authentication/required-actions/{alias}` —
/// upsert. Keycloak returns 204.
pub async fn update_required_action(
    State(store): State<Arc<RequiredActionStore>>,
    Path((realm, alias)): Path<(String, String)>,
    Json(mut r): Json<RequiredActionProvider>,
) -> impl IntoResponse {
    if r.alias != alias {
        // Tolerant: prefer the path alias (admin UI sometimes omits the
        // body alias on PUT, leaving it empty).
        r.alias = alias;
    }
    store.put(&realm, r);
    StatusCode::NO_CONTENT.into_response()
}

/// `DELETE /admin/realms/{realm}/authentication/required-actions/{alias}`
pub async fn delete_required_action(
    State(store): State<Arc<RequiredActionStore>>,
    Path((realm, alias)): Path<(String, String)>,
) -> impl IntoResponse {
    if store.delete(&realm, &alias) {
        StatusCode::NO_CONTENT.into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "required action not found", "alias": alias })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ra(alias: &str, priority: i32) -> RequiredActionProvider {
        RequiredActionProvider {
            alias: alias.into(),
            name: alias.replace('_', " "),
            provider_id: alias.into(),
            enabled: true,
            default_action: false,
            priority,
        }
    }

    #[test]
    fn list_sorted_by_priority() {
        let s = RequiredActionStore::new();
        s.put("master", ra("UPDATE_PASSWORD", 30));
        s.put("master", ra("VERIFY_EMAIL", 10));
        let out = s.list("master");
        assert_eq!(out[0].alias, "VERIFY_EMAIL");
        assert_eq!(out[1].alias, "UPDATE_PASSWORD");
    }

    #[test]
    fn put_upserts() {
        let s = RequiredActionStore::new();
        s.put("master", ra("CONFIGURE_TOTP", 10));
        let mut r = ra("CONFIGURE_TOTP", 99);
        r.enabled = false;
        s.put("master", r);
        let got = s.get("master", "CONFIGURE_TOTP").unwrap();
        assert_eq!(got.priority, 99);
        assert!(!got.enabled);
    }
}
