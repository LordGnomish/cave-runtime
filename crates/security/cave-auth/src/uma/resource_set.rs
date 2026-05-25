// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../authorization/admin/ResourceSetService.java + Kantara UMA-FedAuthz §2
//
//! UMA 2.0 Resource Registration endpoint (`/uma2/resource_set`).
//!
//! POST   /uma2/resource_set        → create
//! PUT    /uma2/resource_set/{id}   → update
//! DELETE /uma2/resource_set/{id}   → delete
//! GET    /uma2/resource_set/{id}   → fetch
//! GET    /uma2/resource_set        → list ids

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// A resource set as defined in UMA 2.0 Federated Authz §2.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ResourceSet {
    /// Resource set id, server-issued.
    #[serde(rename = "_id", default)]
    pub id: String,
    /// Resource owner subject identifier.
    pub resource_owner: String,
    /// Human readable name, MUST be unique within the resource owner's scope.
    pub name: String,
    /// MIME type or display hint.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub r#type: Option<String>,
    /// Scopes available at this resource. Each is a string per the AS schema.
    #[serde(default)]
    pub resource_scopes: Vec<String>,
    /// Icon URI.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub icon_uri: Option<String>,
    /// Free-form display name.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ResourceSetError {
    #[error("resource set name is required")]
    NameRequired,
    #[error("resource set scopes must be non-empty per UMA-FedAuthz §2")]
    ScopesRequired,
    #[error("resource owner missing")]
    OwnerRequired,
    #[error("name {0:?} already registered for owner {1:?}")]
    DuplicateName(String, String),
    #[error("resource set {0:?} not found")]
    NotFound(String),
}

/// In-memory registry of resource sets — the production crate stores these
/// in the same backend as JPA Resource entities upstream.
pub struct ResourceSetStore {
    inner: Mutex<HashMap<String, ResourceSet>>,
}

impl Default for ResourceSetStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ResourceSetStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Validates and registers a new resource set. The server allocates the `id`.
    pub fn create(&self, mut rs: ResourceSet) -> Result<ResourceSet, ResourceSetError> {
        if rs.name.is_empty() {
            return Err(ResourceSetError::NameRequired);
        }
        if rs.resource_scopes.is_empty() {
            return Err(ResourceSetError::ScopesRequired);
        }
        if rs.resource_owner.is_empty() {
            return Err(ResourceSetError::OwnerRequired);
        }
        let mut guard = self.inner.lock().unwrap();
        if guard
            .values()
            .any(|x| x.name == rs.name && x.resource_owner == rs.resource_owner)
        {
            return Err(ResourceSetError::DuplicateName(
                rs.name.clone(),
                rs.resource_owner,
            ));
        }
        if rs.id.is_empty() {
            rs.id = Uuid::new_v4().to_string();
        }
        guard.insert(rs.id.clone(), rs.clone());
        Ok(rs)
    }

    pub fn update(&self, id: &str, rs: ResourceSet) -> Result<ResourceSet, ResourceSetError> {
        let mut guard = self.inner.lock().unwrap();
        if !guard.contains_key(id) {
            return Err(ResourceSetError::NotFound(id.to_string()));
        }
        let mut new = rs;
        new.id = id.to_string();
        guard.insert(id.to_string(), new.clone());
        Ok(new)
    }

    pub fn delete(&self, id: &str) -> Result<(), ResourceSetError> {
        let mut guard = self.inner.lock().unwrap();
        guard
            .remove(id)
            .map(|_| ())
            .ok_or_else(|| ResourceSetError::NotFound(id.to_string()))
    }

    pub fn get(&self, id: &str) -> Result<ResourceSet, ResourceSetError> {
        let guard = self.inner.lock().unwrap();
        guard
            .get(id)
            .cloned()
            .ok_or_else(|| ResourceSetError::NotFound(id.to_string()))
    }

    pub fn list(&self) -> Vec<String> {
        let guard = self.inner.lock().unwrap();
        guard.keys().cloned().collect()
    }

    pub fn list_for_owner(&self, owner: &str) -> Vec<ResourceSet> {
        let guard = self.inner.lock().unwrap();
        guard
            .values()
            .filter(|x| x.resource_owner == owner)
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ResourceSet {
        ResourceSet {
            id: String::new(),
            resource_owner: "alice".into(),
            name: "photos".into(),
            r#type: Some("https://example/img".into()),
            resource_scopes: vec!["view".into(), "edit".into()],
            icon_uri: None,
            display_name: Some("Alice's photos".into()),
        }
    }

    #[test]
    fn create_assigns_id() {
        let store = ResourceSetStore::new();
        let rs = store.create(sample()).unwrap();
        assert!(!rs.id.is_empty(), "id was not assigned");
    }

    #[test]
    fn create_persists_to_get() {
        let store = ResourceSetStore::new();
        let rs = store.create(sample()).unwrap();
        let fetched = store.get(&rs.id).unwrap();
        assert_eq!(fetched.name, "photos");
        assert_eq!(fetched.resource_scopes, vec!["view", "edit"]);
    }

    #[test]
    fn create_rejects_empty_name() {
        let store = ResourceSetStore::new();
        let mut bad = sample();
        bad.name = String::new();
        assert_eq!(store.create(bad), Err(ResourceSetError::NameRequired));
    }

    #[test]
    fn create_rejects_empty_scopes() {
        let store = ResourceSetStore::new();
        let mut bad = sample();
        bad.resource_scopes.clear();
        assert_eq!(store.create(bad), Err(ResourceSetError::ScopesRequired));
    }

    #[test]
    fn create_rejects_duplicate_name_per_owner() {
        let store = ResourceSetStore::new();
        store.create(sample()).unwrap();
        let err = store.create(sample()).unwrap_err();
        assert!(matches!(err, ResourceSetError::DuplicateName(_, _)));
    }

    #[test]
    fn create_allows_same_name_for_different_owner() {
        let store = ResourceSetStore::new();
        store.create(sample()).unwrap();
        let mut other = sample();
        other.resource_owner = "bob".into();
        store.create(other).expect("different owner is fine");
    }

    #[test]
    fn update_replaces_record() {
        let store = ResourceSetStore::new();
        let rs = store.create(sample()).unwrap();
        let mut patched = rs.clone();
        patched.display_name = Some("renamed".into());
        let out = store.update(&rs.id, patched).unwrap();
        assert_eq!(out.display_name.as_deref(), Some("renamed"));
    }

    #[test]
    fn update_unknown_id_fails() {
        let store = ResourceSetStore::new();
        let err = store.update("nope", sample()).unwrap_err();
        assert!(matches!(err, ResourceSetError::NotFound(_)));
    }

    #[test]
    fn delete_removes() {
        let store = ResourceSetStore::new();
        let rs = store.create(sample()).unwrap();
        store.delete(&rs.id).unwrap();
        assert!(matches!(
            store.get(&rs.id),
            Err(ResourceSetError::NotFound(_))
        ));
    }

    #[test]
    fn list_for_owner_filters() {
        let store = ResourceSetStore::new();
        store.create(sample()).unwrap();
        let mut other = sample();
        other.resource_owner = "bob".into();
        other.name = "docs".into();
        store.create(other).unwrap();
        assert_eq!(store.list_for_owner("alice").len(), 1);
        assert_eq!(store.list_for_owner("bob").len(), 1);
    }

    #[test]
    fn list_returns_all_ids() {
        let store = ResourceSetStore::new();
        store.create(sample()).unwrap();
        let mut other = sample();
        other.name = "docs".into();
        store.create(other).unwrap();
        assert_eq!(store.list().len(), 2);
    }
}
