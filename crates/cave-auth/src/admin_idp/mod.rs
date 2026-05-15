// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../admin/IdentityProviderResource.java
//
//! `/admin/realms/{realm}/identity-provider/` REST surface.
//!
//! Sub-modules:
//! - [`instances`] — CRUD for identity-provider instances.
//! - [`mappers`] — CRUD for identity-provider mappers (per-instance).

pub mod instances;
pub mod mappers;

#[cfg(test)]
pub mod tests;

use axum::Router;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::HashMap;

use instances::IdentityProvider;
use mappers::IdentityProviderMapper;

#[derive(Clone, Default)]
pub struct IdentityProviderStore {
    // realm -> alias -> IdentityProvider
    inner: Arc<RwLock<HashMap<String, HashMap<String, IdentityProvider>>>>,
}

impl IdentityProviderStore {
    pub fn new() -> Self { Self::default() }

    pub async fn list(&self, realm: &str) -> Vec<IdentityProvider> {
        self.inner.read().await.get(realm).map(|m| m.values().cloned().collect()).unwrap_or_default()
    }
    pub async fn get(&self, realm: &str, alias: &str) -> Option<IdentityProvider> {
        self.inner.read().await.get(realm).and_then(|m| m.get(alias).cloned())
    }
    pub async fn create(&self, realm: &str, idp: IdentityProvider) -> Result<IdentityProvider, &'static str> {
        let mut w = self.inner.write().await;
        let r = w.entry(realm.to_string()).or_default();
        if r.contains_key(&idp.alias) { return Err("conflict"); }
        r.insert(idp.alias.clone(), idp.clone());
        Ok(idp)
    }
    pub async fn update(&self, realm: &str, alias: &str, mut idp: IdentityProvider) -> Result<IdentityProvider, &'static str> {
        let mut w = self.inner.write().await;
        let r = w.get_mut(realm).ok_or("not_found")?;
        if !r.contains_key(alias) { return Err("not_found"); }
        idp.alias = alias.to_string();
        r.insert(alias.to_string(), idp.clone());
        Ok(idp)
    }
    pub async fn delete(&self, realm: &str, alias: &str) -> Result<(), &'static str> {
        let mut w = self.inner.write().await;
        let r = w.get_mut(realm).ok_or("not_found")?;
        r.remove(alias).ok_or("not_found")?;
        Ok(())
    }
    pub async fn count(&self, realm: &str) -> usize {
        self.inner.read().await.get(realm).map(|m| m.len()).unwrap_or(0)
    }
}

#[derive(Clone, Default)]
pub struct IdentityProviderMapperStore {
    // realm -> alias -> mapperId -> mapper
    inner: Arc<RwLock<HashMap<String, HashMap<String, HashMap<String, IdentityProviderMapper>>>>>,
}

impl IdentityProviderMapperStore {
    pub fn new() -> Self { Self::default() }

    pub async fn list(&self, realm: &str, alias: &str) -> Vec<IdentityProviderMapper> {
        self.inner.read().await.get(realm)
            .and_then(|m| m.get(alias))
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default()
    }
    pub async fn create(&self, realm: &str, alias: &str, mapper: IdentityProviderMapper) -> Result<IdentityProviderMapper, &'static str> {
        let mut w = self.inner.write().await;
        let entry = w.entry(realm.to_string()).or_default().entry(alias.to_string()).or_default();
        if entry.contains_key(&mapper.id) { return Err("conflict"); }
        entry.insert(mapper.id.clone(), mapper.clone());
        Ok(mapper)
    }
    pub async fn get(&self, realm: &str, alias: &str, mapper_id: &str) -> Option<IdentityProviderMapper> {
        self.inner.read().await.get(realm)?.get(alias)?.get(mapper_id).cloned()
    }
    pub async fn delete(&self, realm: &str, alias: &str, mapper_id: &str) -> Result<(), &'static str> {
        let mut w = self.inner.write().await;
        let r = w.get_mut(realm).and_then(|m| m.get_mut(alias)).ok_or("not_found")?;
        r.remove(mapper_id).ok_or("not_found")?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct AdminIdpState {
    pub realms: crate::keycloak::realm::RealmStore,
    pub providers: IdentityProviderStore,
    pub mappers: IdentityProviderMapperStore,
}

impl AdminIdpState {
    pub fn new(realms: crate::keycloak::realm::RealmStore) -> Self {
        Self { realms, providers: IdentityProviderStore::new(), mappers: IdentityProviderMapperStore::new() }
    }
}

pub fn admin_idp_router(state: AdminIdpState) -> Router {
    Router::new()
        .merge(instances::router(state.clone()))
        .merge(mappers::router(state))
}
