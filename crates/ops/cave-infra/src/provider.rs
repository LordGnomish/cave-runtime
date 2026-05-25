// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Provider abstraction — bare metal and cloud-like providers.

use crate::error::{InfraError, InfraResult};
use crate::resource::{ResourceKind, ResourceSpec, ResourceState};
use async_trait::async_trait;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Provider trait ────────────────────────────────────────────────────────────

#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    fn supported_kinds(&self) -> Vec<ResourceKind>;

    async fn create(&self, spec: &ResourceSpec) -> InfraResult<ProvisionResult>;
    async fn read(
        &self,
        provider_id: &str,
        kind: &ResourceKind,
    ) -> InfraResult<HashMap<String, serde_json::Value>>;
    async fn update(&self, provider_id: &str, spec: &ResourceSpec) -> InfraResult<ProvisionResult>;
    async fn delete(&self, provider_id: &str, kind: &ResourceKind) -> InfraResult<()>;

    fn validate(&self, spec: &ResourceSpec) -> InfraResult<()> {
        if spec.name.is_empty() {
            return Err(InfraError::InvalidSpec("name is required".into()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisionResult {
    pub provider_id: String,
    pub actual: HashMap<String, serde_json::Value>,
    pub outputs: HashMap<String, serde_json::Value>,
}

// ── Bare-metal provider ───────────────────────────────────────────────────────

pub struct BareMetalProvider {
    name: String,
    /// Simulated inventory of available hardware
    inventory: DashMap<String, ServerTemplate>,
}

#[derive(Debug, Clone)]
pub struct ServerTemplate {
    pub cpu: i32,
    pub memory_gb: i32,
    pub disk_gb: i32,
    pub available: bool,
}

impl BareMetalProvider {
    pub fn new(name: String) -> Self {
        let p = Self {
            name,
            inventory: DashMap::new(),
        };
        // Populate some inventory
        for i in 1..=20 {
            p.inventory.insert(
                format!("baremetal-{i:03}"),
                ServerTemplate {
                    cpu: 32,
                    memory_gb: 128,
                    disk_gb: 1000,
                    available: true,
                },
            );
        }
        p
    }
}

#[async_trait]
impl Provider for BareMetalProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn supported_kinds(&self) -> Vec<ResourceKind> {
        vec![
            ResourceKind::Server,
            ResourceKind::IpAddress,
            ResourceKind::SshKey,
        ]
    }

    async fn create(&self, spec: &ResourceSpec) -> InfraResult<ProvisionResult> {
        self.validate(spec)?;

        match spec.kind {
            ResourceKind::Server => {
                let cpu = spec
                    .properties
                    .get("cpu")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(2) as i32;
                let memory_gb = spec
                    .properties
                    .get("memory_gb")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(4) as i32;

                // Find an available server in inventory
                let provider_id = self
                    .inventory
                    .iter()
                    .find(|e| e.available && e.cpu >= cpu && e.memory_gb >= memory_gb)
                    .map(|e| e.key().clone())
                    .ok_or_else(|| InfraError::ProviderError {
                        provider: self.name.clone(),
                        message: "no available hardware matching spec".into(),
                    })?;

                // Mark as allocated
                if let Some(mut host) = self.inventory.get_mut(&provider_id) {
                    host.available = false;
                }

                let mut actual = spec.properties.clone();
                actual.insert("provider_id".into(), serde_json::json!(&provider_id));
                actual.insert(
                    "ip_address".into(),
                    serde_json::json!(format!(
                        "10.0.1.{}",
                        provider_id
                            .chars()
                            .filter(|c| c.is_numeric())
                            .collect::<String>()
                            .chars()
                            .take(3)
                            .collect::<String>()
                    )),
                );
                actual.insert("status".into(), serde_json::json!("running"));

                let mut outputs = HashMap::new();
                outputs.insert("provider_id".into(), serde_json::json!(&provider_id));
                outputs.insert("private_ip".into(), actual["ip_address"].clone());

                Ok(ProvisionResult {
                    provider_id,
                    actual,
                    outputs,
                })
            }
            ResourceKind::IpAddress => {
                let provider_id = format!(
                    "ip-{}",
                    uuid::Uuid::new_v4()
                        .to_string()
                        .split('-')
                        .next()
                        .unwrap_or("xxxx")
                );
                let ip = format!("185.{}.{}.{}", rand_byte(), rand_byte(), rand_byte());
                let mut actual = HashMap::new();
                actual.insert("ip".into(), serde_json::json!(&ip));
                actual.insert("type".into(), serde_json::json!("public"));
                let mut outputs = HashMap::new();
                outputs.insert("ip".into(), serde_json::json!(&ip));
                Ok(ProvisionResult {
                    provider_id,
                    actual,
                    outputs,
                })
            }
            ResourceKind::SshKey => {
                let provider_id = format!(
                    "key-{}",
                    uuid::Uuid::new_v4()
                        .to_string()
                        .split('-')
                        .next()
                        .unwrap_or("xxxx")
                );
                let fingerprint = format!("SHA256:{}", uuid::Uuid::new_v4());
                let mut actual = HashMap::new();
                actual.insert("fingerprint".into(), serde_json::json!(&fingerprint));
                let outputs = actual.clone();
                Ok(ProvisionResult {
                    provider_id,
                    actual,
                    outputs,
                })
            }
            ref other => Err(InfraError::ProviderError {
                provider: self.name.clone(),
                message: format!("unsupported kind: {}", other.as_str()),
            }),
        }
    }

    async fn read(
        &self,
        provider_id: &str,
        kind: &ResourceKind,
    ) -> InfraResult<HashMap<String, serde_json::Value>> {
        let mut actual = HashMap::new();
        match kind {
            ResourceKind::Server => {
                let available = !self
                    .inventory
                    .get(provider_id)
                    .map(|h| h.available)
                    .unwrap_or(true);
                actual.insert(
                    "status".into(),
                    serde_json::json!(if available { "running" } else { "deleted" }),
                );
                actual.insert("provider_id".into(), serde_json::json!(provider_id));
            }
            _ => {
                actual.insert("provider_id".into(), serde_json::json!(provider_id));
            }
        }
        Ok(actual)
    }

    async fn update(&self, provider_id: &str, spec: &ResourceSpec) -> InfraResult<ProvisionResult> {
        let actual = spec.properties.clone();
        Ok(ProvisionResult {
            provider_id: provider_id.to_string(),
            actual,
            outputs: HashMap::new(),
        })
    }

    async fn delete(&self, provider_id: &str, kind: &ResourceKind) -> InfraResult<()> {
        if let ResourceKind::Server = kind {
            if let Some(mut host) = self.inventory.get_mut(provider_id) {
                host.available = true;
            }
        }
        Ok(())
    }
}

fn rand_byte() -> u8 {
    // Simple deterministic "random" based on current time nanos
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(42);
    (t % 256) as u8
}

// ── No-op provider (for testing) ──────────────────────────────────────────────

pub struct NoopProvider {
    name: String,
}

impl NoopProvider {
    pub fn new(name: String) -> Self {
        Self { name }
    }
}

#[async_trait]
impl Provider for NoopProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn supported_kinds(&self) -> Vec<ResourceKind> {
        vec![
            ResourceKind::Server,
            ResourceKind::Network,
            ResourceKind::Subnet,
            ResourceKind::LoadBalancer,
            ResourceKind::BlockStorage,
            ResourceKind::ObjectStorage,
            ResourceKind::Database,
            ResourceKind::Cache,
            ResourceKind::Dns,
            ResourceKind::Firewall,
            ResourceKind::IpAddress,
            ResourceKind::SshKey,
            ResourceKind::KubernetesCluster,
        ]
    }

    async fn create(&self, spec: &ResourceSpec) -> InfraResult<ProvisionResult> {
        self.validate(spec)?;
        let provider_id = format!(
            "{}-{}",
            self.name,
            uuid::Uuid::new_v4()
                .to_string()
                .split('-')
                .next()
                .unwrap_or("xxxx")
        );
        Ok(ProvisionResult {
            provider_id,
            actual: spec.properties.clone(),
            outputs: HashMap::new(),
        })
    }

    async fn read(
        &self,
        provider_id: &str,
        _kind: &ResourceKind,
    ) -> InfraResult<HashMap<String, serde_json::Value>> {
        let mut actual = HashMap::new();
        actual.insert("provider_id".into(), serde_json::json!(provider_id));
        actual.insert("status".into(), serde_json::json!("running"));
        Ok(actual)
    }

    async fn update(&self, provider_id: &str, spec: &ResourceSpec) -> InfraResult<ProvisionResult> {
        Ok(ProvisionResult {
            provider_id: provider_id.to_string(),
            actual: spec.properties.clone(),
            outputs: HashMap::new(),
        })
    }

    async fn delete(&self, _provider_id: &str, _kind: &ResourceKind) -> InfraResult<()> {
        Ok(())
    }
}

// ── Provider registry ─────────────────────────────────────────────────────────

pub struct ProviderRegistry {
    providers: DashMap<String, Box<dyn Provider>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        let r = Self {
            providers: DashMap::new(),
        };
        // Register default providers
        r.register(Box::new(BareMetalProvider::new("bare-metal".into())));
        r.register(Box::new(NoopProvider::new("noop".into())));
        r
    }

    pub fn register(&self, provider: Box<dyn Provider>) {
        self.providers.insert(provider.name().to_string(), provider);
    }

    pub fn get(&self, name: &str) -> InfraResult<&dyn Provider> {
        // Safety: DashMap guarantees the reference is valid for the lifetime of the guard.
        // We need a different approach since we can't return a reference through the DashMap guard.
        // Return the provider name and use it for operations instead.
        // This is a design limitation - in a real impl we'd use Arc<dyn Provider>.
        Err(InfraError::ProviderNotFound(name.to_string()))
    }

    pub fn list_names(&self) -> Vec<String> {
        self.providers.iter().map(|e| e.key().clone()).collect()
    }

    /// Execute a create operation via the named provider.
    pub async fn create(
        &self,
        provider_name: &str,
        spec: &ResourceSpec,
    ) -> InfraResult<ProvisionResult> {
        // We need to hold the ref within the scope of this function
        let guard = self
            .providers
            .get(provider_name)
            .ok_or_else(|| InfraError::ProviderNotFound(provider_name.to_string()))?;
        guard.create(spec).await
    }

    pub async fn read(
        &self,
        provider_name: &str,
        provider_id: &str,
        kind: &ResourceKind,
    ) -> InfraResult<HashMap<String, serde_json::Value>> {
        let guard = self
            .providers
            .get(provider_name)
            .ok_or_else(|| InfraError::ProviderNotFound(provider_name.to_string()))?;
        guard.read(provider_id, kind).await
    }

    pub async fn update(
        &self,
        provider_name: &str,
        provider_id: &str,
        spec: &ResourceSpec,
    ) -> InfraResult<ProvisionResult> {
        let guard = self
            .providers
            .get(provider_name)
            .ok_or_else(|| InfraError::ProviderNotFound(provider_name.to_string()))?;
        guard.update(provider_id, spec).await
    }

    pub async fn delete(
        &self,
        provider_name: &str,
        provider_id: &str,
        kind: &ResourceKind,
    ) -> InfraResult<()> {
        let guard = self
            .providers
            .get(provider_name)
            .ok_or_else(|| InfraError::ProviderNotFound(provider_name.to_string()))?;
        guard.delete(provider_id, kind).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bare_metal_provision_server() {
        let provider = BareMetalProvider::new("bm".into());
        let mut props = HashMap::new();
        props.insert("cpu".into(), serde_json::json!(4));
        props.insert("memory_gb".into(), serde_json::json!(16));
        let spec = ResourceSpec {
            kind: ResourceKind::Server,
            name: "test-server".into(),
            provider: "bare-metal".into(),
            properties: props,
            depends_on: vec![],
            tags: HashMap::new(),
        };
        let result = provider.create(&spec).await.unwrap();
        assert!(!result.provider_id.is_empty());
        assert!(result.actual.contains_key("ip_address"));
    }

    #[tokio::test]
    async fn noop_provider_succeeds_for_any_kind() {
        let provider = NoopProvider::new("noop".into());
        let spec = ResourceSpec {
            kind: ResourceKind::Database,
            name: "my-db".into(),
            provider: "noop".into(),
            properties: HashMap::new(),
            depends_on: vec![],
            tags: HashMap::new(),
        };
        let result = provider.create(&spec).await.unwrap();
        assert!(!result.provider_id.is_empty());
    }

    #[tokio::test]
    async fn provider_registry_create() {
        let registry = ProviderRegistry::new();
        let mut props = HashMap::new();
        props.insert("cpu".into(), serde_json::json!(2));
        props.insert("memory_gb".into(), serde_json::json!(8));
        let spec = ResourceSpec {
            kind: ResourceKind::Server,
            name: "registry-test".into(),
            provider: "bare-metal".into(),
            properties: props,
            depends_on: vec![],
            tags: HashMap::new(),
        };
        let result = registry.create("bare-metal", &spec).await.unwrap();
        assert!(!result.provider_id.is_empty());
    }
}
