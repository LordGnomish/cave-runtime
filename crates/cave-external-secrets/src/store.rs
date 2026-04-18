//! SecretStore and ClusterSecretStore registry.

use crate::error::{EsoError, EsoResult};
use crate::models::*;
use chrono::Utc;
use dashmap::DashMap;
use tracing::info;
use uuid::Uuid;

pub struct SecretStoreRegistry {
    stores: DashMap<String, SecretStore>,
}

impl SecretStoreRegistry {
    pub fn new() -> Self {
        Self { stores: DashMap::new() }
    }

    fn key(namespace: Option<&str>, name: &str) -> String {
        match namespace {
            Some(ns) => format!("{ns}/{name}"),
            None => format!("cluster/{name}"),
        }
    }

    pub fn create(&self, req: CreateSecretStoreRequest) -> EsoResult<SecretStore> {
        let key = Self::key(req.namespace.as_deref(), &req.name);
        if self.stores.contains_key(&key) {
            return Err(EsoError::AlreadyExists(key));
        }
        let scope = req.scope.unwrap_or(SecretStoreScope::Namespaced);
        let store = SecretStore {
            id: Uuid::new_v4(),
            name: req.name.clone(),
            namespace: req.namespace.clone(),
            scope,
            provider: req.provider,
            provider_config: req.provider_config,
            refresh_interval_secs: req.refresh_interval_secs.unwrap_or(3600),
            status: SecretStoreStatus::Valid,
            status_message: None,
            created_at: Utc::now(),
        };
        self.stores.insert(key, store.clone());
        info!(name = %req.name, "SecretStore created");
        Ok(store)
    }

    pub fn get(&self, namespace: Option<&str>, name: &str) -> EsoResult<SecretStore> {
        let key = Self::key(namespace, name);
        self.stores.get(&key).map(|r| r.clone()).ok_or_else(|| EsoError::SecretStoreNotFound(key))
    }

    pub fn list(&self, namespace: Option<&str>) -> Vec<SecretStore> {
        self.stores.iter()
            .filter(|r| match namespace {
                Some(ns) => r.value().namespace.as_deref() == Some(ns),
                None => r.value().scope == SecretStoreScope::Cluster,
            })
            .map(|r| r.value().clone())
            .collect()
    }

    pub fn delete(&self, namespace: Option<&str>, name: &str) -> EsoResult<()> {
        let key = Self::key(namespace, name);
        self.stores.remove(&key).ok_or_else(|| EsoError::SecretStoreNotFound(key))?;
        Ok(())
    }
}

impl Default for SecretStoreRegistry {
    fn default() -> Self { Self::new() }
}
