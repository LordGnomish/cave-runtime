//! ExternalSecret and PushSecret stores + simulated sync.

use crate::error::{EsoError, EsoResult};
use crate::models::*;
use chrono::Utc;
use dashmap::DashMap;
use uuid::Uuid;

pub struct ExternalSecretStore {
    secrets: DashMap<String, ExternalSecret>,
}

impl ExternalSecretStore {
    pub fn new() -> Self {
        Self { secrets: DashMap::new() }
    }

    fn key(ns: &str, name: &str) -> String { format!("{ns}/{name}") }

    pub fn create(&self, req: CreateExternalSecretRequest) -> EsoResult<ExternalSecret> {
        let key = Self::key(&req.namespace, &req.name);
        if self.secrets.contains_key(&key) {
            return Err(EsoError::AlreadyExists(key));
        }
        let es = ExternalSecret {
            id: Uuid::new_v4(),
            name: req.name,
            namespace: req.namespace,
            secret_store_ref: req.secret_store_ref,
            target: req.target,
            data: req.data,
            data_from: req.data_from.unwrap_or_default(),
            refresh_interval_secs: req.refresh_interval_secs.unwrap_or(3600),
            status: SyncStatus::Unknown,
            last_synced_at: None,
            synced_version: None,
            created_at: Utc::now(),
        };
        self.secrets.insert(key, es.clone());
        Ok(es)
    }

    pub fn get(&self, ns: &str, name: &str) -> EsoResult<ExternalSecret> {
        let key = Self::key(ns, name);
        self.secrets.get(&key).map(|r| r.clone()).ok_or_else(|| EsoError::ExternalSecretNotFound(key))
    }

    pub fn list(&self, ns: &str) -> Vec<ExternalSecret> {
        self.secrets.iter().filter(|r| r.value().namespace == ns).map(|r| r.value().clone()).collect()
    }

    pub fn delete(&self, ns: &str, name: &str) -> EsoResult<()> {
        let key = Self::key(ns, name);
        self.secrets.remove(&key).ok_or_else(|| EsoError::ExternalSecretNotFound(key))?;
        Ok(())
    }

    pub fn simulate_sync(&self, ns: &str, name: &str) -> EsoResult<SyncResult> {
        let key = Self::key(ns, name);
        let mut es = self.secrets.get(&key).map(|r| r.clone())
            .ok_or_else(|| EsoError::ExternalSecretNotFound(key.clone()))?;
        let keys_synced: Vec<String> = es.data.iter().map(|d| d.secret_key.clone()).collect();
        es.status = SyncStatus::Ready;
        es.last_synced_at = Some(Utc::now());
        es.synced_version = Some(format!("v{}", Utc::now().timestamp()));
        let result = SyncResult {
            secret_name: es.target.name.clone(),
            namespace: ns.to_owned(),
            keys_synced: keys_synced.clone(),
            synced_at: Utc::now(),
            version: es.synced_version.clone().unwrap_or_default(),
        };
        self.secrets.insert(key, es);
        Ok(result)
    }
}

impl Default for ExternalSecretStore {
    fn default() -> Self { Self::new() }
}

pub struct PushSecretStore {
    secrets: DashMap<String, PushSecret>,
}

impl PushSecretStore {
    pub fn new() -> Self {
        Self { secrets: DashMap::new() }
    }

    fn key(ns: &str, name: &str) -> String { format!("{ns}/{name}") }

    pub fn create(&self, req: CreatePushSecretRequest) -> EsoResult<PushSecret> {
        let key = Self::key(&req.namespace, &req.name);
        if self.secrets.contains_key(&key) {
            return Err(EsoError::AlreadyExists(key));
        }
        let ps = PushSecret {
            id: Uuid::new_v4(),
            name: req.name,
            namespace: req.namespace,
            secret_store_refs: req.secret_store_refs,
            selector: req.selector,
            data: req.data,
            status: SyncStatus::Unknown,
            created_at: Utc::now(),
        };
        self.secrets.insert(key, ps.clone());
        Ok(ps)
    }

    pub fn get(&self, ns: &str, name: &str) -> EsoResult<PushSecret> {
        let key = Self::key(ns, name);
        self.secrets.get(&key).map(|r| r.clone()).ok_or_else(|| EsoError::PushSecretNotFound(key))
    }

    pub fn list(&self, ns: &str) -> Vec<PushSecret> {
        self.secrets.iter().filter(|r| r.value().namespace == ns).map(|r| r.value().clone()).collect()
    }

    pub fn delete(&self, ns: &str, name: &str) -> EsoResult<()> {
        let key = Self::key(ns, name);
        self.secrets.remove(&key).ok_or_else(|| EsoError::PushSecretNotFound(key))?;
        Ok(())
    }
}

impl Default for PushSecretStore {
    fn default() -> Self { Self::new() }
}
