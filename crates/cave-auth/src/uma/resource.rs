// SPDX-License-Identifier: AGPL-3.0-or-later
//
// UMA 2.0 Resource Registration — UMA-FedAuthz §2.
//
// Endpoint: `POST /authz/protection/resource_set` (and `GET /…/{id}`,
// `PUT /…/{id}`, `DELETE /…/{id}`).
//
// Upstream: keycloak/keycloak  b825ba97b489d715f7ca1984c19bd95afb355a38
//   services/src/main/java/org/keycloak/authorization/protection/resource/ResourceService.java

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use super::UmaError;

/// UMA-FedAuthz §2.1 — resource description registered by the resource
/// server with the authorization server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceSet {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_: Option<String>,
    #[serde(default)]
    pub resource_scopes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_uri: Option<String>,
    /// Owner is added by the AS (=resource owner subject). Resource servers
    /// may declare `owner_managed_access` to opt the resource into UMA.
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub owner_managed_access: bool,
}

/// In-memory resource-set store. Scoped per realm.
#[derive(Clone, Default)]
pub struct ResourceStore {
    inner: Arc<Mutex<HashMap<(String, String), ResourceSet>>>, // (realm, id) -> set
}

impl ResourceStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &self,
        realm: &str,
        mut rs: ResourceSet,
        owner_sub: Option<String>,
    ) -> Result<ResourceSet, UmaError> {
        if rs.name.trim().is_empty() {
            return Err(UmaError::InvalidRequest("name required"));
        }
        let id = Uuid::new_v4().to_string();
        rs.id = Some(id.clone());
        if rs.owner.is_none() {
            rs.owner = owner_sub;
        }
        self.inner.lock().unwrap().insert((realm.to_string(), id), rs.clone());
        Ok(rs)
    }

    pub fn get(&self, realm: &str, id: &str) -> Option<ResourceSet> {
        self.inner.lock().unwrap().get(&(realm.to_string(), id.to_string())).cloned()
    }

    pub fn update(&self, realm: &str, id: &str, mut rs: ResourceSet) -> Result<ResourceSet, UmaError> {
        let mut g = self.inner.lock().unwrap();
        if !g.contains_key(&(realm.to_string(), id.to_string())) {
            return Err(UmaError::NotFound);
        }
        rs.id = Some(id.to_string());
        g.insert((realm.to_string(), id.to_string()), rs.clone());
        Ok(rs)
    }

    pub fn delete(&self, realm: &str, id: &str) -> Result<(), UmaError> {
        let mut g = self.inner.lock().unwrap();
        if g.remove(&(realm.to_string(), id.to_string())).is_none() {
            return Err(UmaError::NotFound);
        }
        Ok(())
    }

    pub fn list(&self, realm: &str) -> Vec<ResourceSet> {
        self.inner
            .lock()
            .unwrap()
            .iter()
            .filter(|((r, _), _)| r == realm)
            .map(|(_, v)| v.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ResourceSet {
        ResourceSet {
            id: None,
            name: "Photo Album".into(),
            uri: Some("/album/2026/spring".into()),
            type_: Some("photo-album".into()),
            resource_scopes: vec!["view".into(), "edit".into()],
            icon_uri: None,
            owner: None,
            owner_managed_access: true,
        }
    }

    // upstream: uma-fedauthz §2.2.1 — register returns the assigned _id.
    #[test]
    fn register_assigns_id() {
        let store = ResourceStore::new();
        let out = store.register("r1", sample(), Some("alice".into())).unwrap();
        assert!(out.id.is_some());
        assert_eq!(out.owner.as_deref(), Some("alice"));
    }

    // upstream: uma-fedauthz §2.2.1 — name is required.
    #[test]
    fn register_requires_name() {
        let store = ResourceStore::new();
        let mut bad = sample();
        bad.name = "".into();
        let err = store.register("r1", bad, None).unwrap_err();
        assert!(matches!(err, UmaError::InvalidRequest(_)));
    }

    // upstream: uma-fedauthz §2.2.4 — get / update / delete round-trip.
    #[test]
    fn crud_round_trip() {
        let store = ResourceStore::new();
        let out = store.register("r1", sample(), Some("alice".into())).unwrap();
        let id = out.id.clone().unwrap();
        assert_eq!(store.get("r1", &id).unwrap().name, "Photo Album");
        let mut upd = out.clone();
        upd.name = "Renamed Album".into();
        store.update("r1", &id, upd).unwrap();
        assert_eq!(store.get("r1", &id).unwrap().name, "Renamed Album");
        store.delete("r1", &id).unwrap();
        assert!(store.get("r1", &id).is_none());
    }

    // upstream: uma-fedauthz §2.2.4 — DELETE on unknown id = 404.
    #[test]
    fn delete_missing_errors() {
        let store = ResourceStore::new();
        let err = store.delete("r1", "no-such").unwrap_err();
        assert_eq!(err, UmaError::NotFound);
    }

    // upstream: uma-fedauthz §2.2.5 — list scoped to realm only.
    #[test]
    fn list_isolated_per_realm() {
        let store = ResourceStore::new();
        store.register("r1", sample(), None).unwrap();
        store.register("r1", sample(), None).unwrap();
        store.register("r2", sample(), None).unwrap();
        assert_eq!(store.list("r1").len(), 2);
        assert_eq!(store.list("r2").len(), 1);
    }
}
