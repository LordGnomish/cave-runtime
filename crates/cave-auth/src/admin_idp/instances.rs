// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 CAVE Runtime contributors
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/services/resources/admin/IdentityProvidersResource.java
//         services/src/main/java/org/keycloak/services/resources/admin/IdentityProviderResource.java
//
// Keycloak Admin REST API — Identity Provider instances.
//
// Routes ported (under `/admin/realms/{realm}/identity-provider/instances`):
//   GET    /                — list all IdPs in the realm
//   POST   /                — create new IdP
//   GET    /{alias}         — fetch single IdP by alias
//   PUT    /{alias}         — replace IdP config
//   DELETE /{alias}         — remove IdP
//
// Storage: in-memory (DashMap) keyed by `(realm, alias)`. K2 will replace
// the store with a JPA-backed implementation in a later commit (the
// `Backend` seam is the `IdentityProviderStore` struct itself — swapping
// it out is a contained refactor).

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::sync::Arc;

/// Keycloak `IdentityProviderRepresentation` — minimal subset that the
/// admin console actually writes. Extra fields are accepted via the
/// `config` map (Keycloak's own representation uses `Map<String, String>`
/// for arbitrary provider-specific config, e.g. `clientId`, `clientSecret`,
/// `tokenUrl`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IdentityProvider {
    /// Stable identifier used as the URL path segment and as the key in
    /// `KeycloakIdentityProviderMapperModel.identityProviderAlias`.
    pub alias: String,
    /// Provider plugin id — `oidc`, `saml`, `google`, `github`, …
    #[serde(rename = "providerId")]
    pub provider_id: String,
    #[serde(default)]
    pub enabled: bool,
    /// Whether `account.management` allows the user to unlink this IdP.
    #[serde(default, rename = "storeToken")]
    pub store_token: bool,
    /// Free-form provider config (`clientId`, `clientSecret`, `tokenUrl`,
    /// `authorizationUrl`, `useJwksUrl`, …). Mirrors Keycloak's String→String
    /// representation; values are JSON-stringified by the admin console.
    #[serde(default)]
    pub config: Map<String, Value>,
    /// Display name (optional) — used by the login page.
    #[serde(
        default,
        rename = "displayName",
        skip_serializing_if = "Option::is_none"
    )]
    pub display_name: Option<String>,
}

/// In-memory store keyed by `(realm, alias)`. Concurrent-safe.
#[derive(Debug, Default)]
pub struct IdentityProviderStore {
    inner: DashMap<(String, String), IdentityProvider>,
}

impl IdentityProviderStore {
    pub fn new() -> Self {
        Self {
            inner: DashMap::new(),
        }
    }

    /// List all IdPs in a realm, sorted by alias (deterministic for tests).
    pub fn list(&self, realm: &str) -> Vec<IdentityProvider> {
        let mut out: Vec<_> = self
            .inner
            .iter()
            .filter(|kv| kv.key().0 == realm)
            .map(|kv| kv.value().clone())
            .collect();
        out.sort_by(|a, b| a.alias.cmp(&b.alias));
        out
    }

    pub fn get(&self, realm: &str, alias: &str) -> Option<IdentityProvider> {
        self.inner
            .get(&(realm.to_string(), alias.to_string()))
            .map(|v| v.clone())
    }

    /// Insert or replace. Returns `true` if a previous entry existed.
    pub fn put(&self, realm: &str, idp: IdentityProvider) -> bool {
        let key = (realm.to_string(), idp.alias.clone());
        self.inner.insert(key, idp).is_some()
    }

    pub fn delete(&self, realm: &str, alias: &str) -> bool {
        self.inner
            .remove(&(realm.to_string(), alias.to_string()))
            .is_some()
    }
}

/// `GET /admin/realms/{realm}/identity-provider/instances`
pub async fn list_instances(
    State(store): State<Arc<IdentityProviderStore>>,
    Path(realm): Path<String>,
) -> impl IntoResponse {
    let idps = store.list(&realm);
    (StatusCode::OK, Json(idps))
}

/// `POST /admin/realms/{realm}/identity-provider/instances`
///
/// Keycloak returns `201 Created` with a `Location` header pointing at the
/// newly created resource. On alias collision Keycloak returns `409
/// Conflict` (see `IdentityProvidersResource.create`).
pub async fn create_instance(
    State(store): State<Arc<IdentityProviderStore>>,
    Path(realm): Path<String>,
    Json(idp): Json<IdentityProvider>,
) -> impl IntoResponse {
    if idp.alias.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "alias is required" })),
        )
            .into_response();
    }
    if store.get(&realm, &idp.alias).is_some() {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "error": "alias already exists", "alias": idp.alias })),
        )
            .into_response();
    }
    let location = format!(
        "/admin/realms/{realm}/identity-provider/instances/{}",
        idp.alias
    );
    store.put(&realm, idp);
    (
        StatusCode::CREATED,
        [(axum::http::header::LOCATION, location)],
        Json(json!({})),
    )
        .into_response()
}

/// `GET /admin/realms/{realm}/identity-provider/instances/{alias}`
pub async fn get_instance(
    State(store): State<Arc<IdentityProviderStore>>,
    Path((realm, alias)): Path<(String, String)>,
) -> impl IntoResponse {
    match store.get(&realm, &alias) {
        Some(idp) => (StatusCode::OK, Json(serde_json::to_value(&idp).unwrap())).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "identity provider not found", "alias": alias })),
        )
            .into_response(),
    }
}

/// `PUT /admin/realms/{realm}/identity-provider/instances/{alias}`
///
/// Keycloak insists the path alias matches the body alias. Mismatch →
/// `400 Bad Request`.
pub async fn update_instance(
    State(store): State<Arc<IdentityProviderStore>>,
    Path((realm, alias)): Path<(String, String)>,
    Json(idp): Json<IdentityProvider>,
) -> impl IntoResponse {
    if idp.alias != alias {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "alias mismatch between path and body" })),
        )
            .into_response();
    }
    if store.get(&realm, &alias).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "identity provider not found", "alias": alias })),
        )
            .into_response();
    }
    store.put(&realm, idp);
    StatusCode::NO_CONTENT.into_response()
}

/// `DELETE /admin/realms/{realm}/identity-provider/instances/{alias}`
pub async fn delete_instance(
    State(store): State<Arc<IdentityProviderStore>>,
    Path((realm, alias)): Path<(String, String)>,
) -> impl IntoResponse {
    if store.delete(&realm, &alias) {
        StatusCode::NO_CONTENT.into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "identity provider not found", "alias": alias })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(alias: &str) -> IdentityProvider {
        IdentityProvider {
            alias: alias.into(),
            provider_id: "oidc".into(),
            enabled: true,
            store_token: false,
            config: Map::new(),
            display_name: Some(format!("{alias} login")),
        }
    }

    #[test]
    fn list_returns_sorted_by_alias() {
        let s = IdentityProviderStore::new();
        s.put("master", sample("zulu"));
        s.put("master", sample("alpha"));
        let out = s.list("master");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].alias, "alpha");
        assert_eq!(out[1].alias, "zulu");
    }

    #[test]
    fn realm_isolation() {
        let s = IdentityProviderStore::new();
        s.put("realm-a", sample("google"));
        s.put("realm-b", sample("github"));
        assert_eq!(s.list("realm-a").len(), 1);
        assert_eq!(s.list("realm-b").len(), 1);
        assert_eq!(s.list("realm-c").len(), 0);
    }

    #[test]
    fn put_returns_true_on_overwrite() {
        let s = IdentityProviderStore::new();
        assert!(!s.put("r", sample("a")));
        assert!(s.put("r", sample("a")));
    }

    #[test]
    fn delete_returns_false_when_missing() {
        let s = IdentityProviderStore::new();
        assert!(!s.delete("r", "nope"));
    }
}
