//! Provider configuration store.

use crate::error::{EsoError, EsoResult};
use crate::models::ProviderType;
use dashmap::DashMap;
use uuid::Uuid;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProviderConfig {
    pub id: Uuid,
    pub name: String,
    pub provider_type: ProviderType,
    pub config: serde_json::Value,
    pub region: Option<String>,
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CreateProviderConfigRequest {
    pub name: String,
    pub provider_type: ProviderType,
    pub config: serde_json::Value,
    pub region: Option<String>,
    pub endpoint: Option<String>,
}

pub struct ProviderConfigStore {
    configs: DashMap<String, ProviderConfig>,
}

impl ProviderConfigStore {
    pub fn new() -> Self {
        Self { configs: DashMap::new() }
    }

    pub fn create(&self, req: CreateProviderConfigRequest) -> EsoResult<ProviderConfig> {
        if self.configs.contains_key(&req.name) {
            return Err(EsoError::AlreadyExists(req.name));
        }
        let cfg = ProviderConfig {
            id: Uuid::new_v4(),
            name: req.name.clone(),
            provider_type: req.provider_type,
            config: req.config,
            region: req.region,
            endpoint: req.endpoint,
        };
        self.configs.insert(req.name, cfg.clone());
        Ok(cfg)
    }

    pub fn get(&self, name: &str) -> EsoResult<ProviderConfig> {
        self.configs.get(name).map(|r| r.clone()).ok_or_else(|| EsoError::ProviderConfigNotFound(name.to_owned()))
    }

    pub fn list(&self) -> Vec<ProviderConfig> {
        self.configs.iter().map(|r| r.value().clone()).collect()
    }

    pub fn delete(&self, name: &str) -> EsoResult<()> {
        self.configs.remove(name).ok_or_else(|| EsoError::ProviderConfigNotFound(name.to_owned()))?;
        Ok(())
    }
}

impl Default for ProviderConfigStore {
    fn default() -> Self { Self::new() }
}
