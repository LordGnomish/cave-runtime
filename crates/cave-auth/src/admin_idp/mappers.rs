// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 CAVE Runtime contributors
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/services/resources/admin/IdentityProviderResource.java
//         (`@Path("mappers")` sub-resource — `getMappers`, `addMapper`,
//         `getMapperById`, `update`, `delete`).
//
// Identity-provider mappers tie attributes/roles/claims from an upstream
// IdP onto Keycloak user attributes. Each mapper has its own
// `identityProviderMapper` plugin id (e.g. `oidc-user-attribute-idp-mapper`,
// `hardcoded-role-idp-mapper`, `saml-role-idp-mapper`).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::sync::Arc;
use uuid::Uuid;

/// Keycloak `IdentityProviderMapperRepresentation`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IdentityProviderMapper {
    /// Server-assigned id (UUID). Optional on POST (server generates) but
    /// always present on GET/PUT.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    /// Mapper plugin id, e.g. `oidc-user-attribute-idp-mapper`.
    #[serde(rename = "identityProviderMapper")]
    pub identity_provider_mapper: String,
    /// Owning alias — Keycloak surfaces this so reverse lookup works.
    #[serde(default, rename = "identityProviderAlias", skip_serializing_if = "Option::is_none")]
    pub identity_provider_alias: Option<String>,
    #[serde(default)]
    pub config: Map<String, Value>,
}

#[derive(Debug, Default)]
pub struct IdentityProviderMapperStore {
    // (realm, idp_alias, mapper_id) -> mapper
    inner: DashMap<(String, String, String), IdentityProviderMapper>,
}

impl IdentityProviderMapperStore {
    pub fn new() -> Self {
        Self { inner: DashMap::new() }
    }

    pub fn list(&self, realm: &str, alias: &str) -> Vec<IdentityProviderMapper> {
        let mut out: Vec<_> = self
            .inner
            .iter()
            .filter(|kv| kv.key().0 == realm && kv.key().1 == alias)
            .map(|kv| kv.value().clone())
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    pub fn get(&self, realm: &str, alias: &str, id: &str) -> Option<IdentityProviderMapper> {
        self.inner
            .get(&(realm.into(), alias.into(), id.into()))
            .map(|v| v.clone())
    }

    /// Assigns a fresh UUID if the mapper lacks an id and stores it.
    /// Returns the persisted id.
    pub fn create(&self, realm: &str, alias: &str, mut m: IdentityProviderMapper) -> String {
        let id = m.id.clone().unwrap_or_else(|| Uuid::new_v4().to_string());
        m.id = Some(id.clone());
        m.identity_provider_alias.get_or_insert_with(|| alias.to_string());
        self.inner.insert((realm.into(), alias.into(), id.clone()), m);
        id
    }

    pub fn update(&self, realm: &str, alias: &str, id: &str, m: IdentityProviderMapper) -> bool {
        let key = (realm.to_string(), alias.to_string(), id.to_string());
        if !self.inner.contains_key(&key) {
            return false;
        }
        let mut m = m;
        m.id = Some(id.to_string());
        m.identity_provider_alias.get_or_insert_with(|| alias.to_string());
        self.inner.insert(key, m);
        true
    }

    pub fn delete(&self, realm: &str, alias: &str, id: &str) -> bool {
        self.inner
            .remove(&(realm.into(), alias.into(), id.into()))
            .is_some()
    }
}

#[derive(Clone)]
pub struct MapperState(pub Arc<IdentityProviderMapperStore>);

/// `GET /admin/realms/{realm}/identity-provider/instances/{alias}/mappers`
pub async fn list_mappers(
    State(store): State<Arc<IdentityProviderMapperStore>>,
    Path((realm, alias)): Path<(String, String)>,
) -> impl IntoResponse {
    (StatusCode::OK, Json(store.list(&realm, &alias)))
}

/// `POST /admin/realms/{realm}/identity-provider/instances/{alias}/mappers`
pub async fn create_mapper(
    State(store): State<Arc<IdentityProviderMapperStore>>,
    Path((realm, alias)): Path<(String, String)>,
    Json(m): Json<IdentityProviderMapper>,
) -> impl IntoResponse {
    if m.name.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "mapper name is required" })),
        )
            .into_response();
    }
    let id = store.create(&realm, &alias, m);
    let location =
        format!("/admin/realms/{realm}/identity-provider/instances/{alias}/mappers/{id}");
    (
        StatusCode::CREATED,
        [(axum::http::header::LOCATION, location)],
        Json(json!({ "id": id })),
    )
        .into_response()
}

/// `GET /admin/realms/{realm}/identity-provider/instances/{alias}/mappers/{id}`
pub async fn get_mapper(
    State(store): State<Arc<IdentityProviderMapperStore>>,
    Path((realm, alias, id)): Path<(String, String, String)>,
) -> impl IntoResponse {
    match store.get(&realm, &alias, &id) {
        Some(m) => (StatusCode::OK, Json(m)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "mapper not found", "id": id })),
        )
            .into_response(),
    }
}

/// `PUT /admin/realms/{realm}/identity-provider/instances/{alias}/mappers/{id}`
pub async fn update_mapper(
    State(store): State<Arc<IdentityProviderMapperStore>>,
    Path((realm, alias, id)): Path<(String, String, String)>,
    Json(m): Json<IdentityProviderMapper>,
) -> impl IntoResponse {
    if store.update(&realm, &alias, &id, m) {
        StatusCode::NO_CONTENT.into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "mapper not found", "id": id })),
        )
            .into_response()
    }
}

/// `DELETE /admin/realms/{realm}/identity-provider/instances/{alias}/mappers/{id}`
pub async fn delete_mapper(
    State(store): State<Arc<IdentityProviderMapperStore>>,
    Path((realm, alias, id)): Path<(String, String, String)>,
) -> impl IntoResponse {
    if store.delete(&realm, &alias, &id) {
        StatusCode::NO_CONTENT.into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "mapper not found", "id": id })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapper(name: &str) -> IdentityProviderMapper {
        IdentityProviderMapper {
            id: None,
            name: name.into(),
            identity_provider_mapper: "oidc-user-attribute-idp-mapper".into(),
            identity_provider_alias: None,
            config: Map::new(),
        }
    }

    #[test]
    fn create_assigns_uuid_and_alias() {
        let s = IdentityProviderMapperStore::new();
        let id = s.create("master", "google", mapper("email-mapper"));
        let got = s.get("master", "google", &id).unwrap();
        assert_eq!(got.identity_provider_alias.as_deref(), Some("google"));
        assert_eq!(got.id.as_deref(), Some(id.as_str()));
    }

    #[test]
    fn isolation_by_realm_and_alias() {
        let s = IdentityProviderMapperStore::new();
        s.create("r1", "google", mapper("m1"));
        s.create("r1", "github", mapper("m1"));
        s.create("r2", "google", mapper("m1"));
        assert_eq!(s.list("r1", "google").len(), 1);
        assert_eq!(s.list("r1", "github").len(), 1);
        assert_eq!(s.list("r2", "google").len(), 1);
        assert_eq!(s.list("r2", "github").len(), 0);
    }
}
