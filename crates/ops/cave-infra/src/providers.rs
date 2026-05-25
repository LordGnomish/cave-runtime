// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Resource providers — abstractions over cloud/infra APIs.

use serde::{Deserialize, Serialize};

/// All supported resource types across providers.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceType {
    Vm,
    Container,
    Vpc,
    Subnet,
    LoadBalancer,
    BlockStorage,
    ObjectStorage,
    DnsRecord,
    SecurityGroup,
    IpAddress,
}

impl std::fmt::Display for ResourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| format!("{self:?}").to_lowercase());
        write!(f, "{s}")
    }
}

// ── Specs ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmSpec {
    pub cpu_cores: u32,
    pub memory_gb: u32,
    pub disk_gb: u32,
    pub image: String,
    pub region: String,
    pub ssh_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VpcSpec {
    pub cidr: String,
    pub region: String,
    pub enable_dns: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubnetSpec {
    pub cidr: String,
    pub vpc_id: String,
    pub availability_zone: String,
    pub public: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageSpec {
    pub size_gb: u32,
    pub storage_class: String,
    pub iops: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectStorageSpec {
    pub bucket_name: String,
    pub region: String,
    pub versioning: bool,
    pub public: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsRecordSpec {
    pub zone: String,
    pub record_type: String,
    pub name: String,
    pub value: String,
    pub ttl: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadBalancerSpec {
    pub algorithm: String,
    pub backends: Vec<String>,
    pub port: u16,
    pub health_check_path: String,
}

// ── ProvisionResult ───────────────────────────────────────────────────────────

/// Result of a provision operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisionResult {
    pub resource_id: String,
    /// Cloud provider's own ID for the resource.
    pub provider_id: String,
    pub provider: String,
    pub actual_state: serde_json::Value,
    pub success: bool,
    pub error: Option<String>,
}

// ── ResourceProvider trait ───────────────────────────────────────────────────

/// Trait that every cloud/infra provider must implement.
#[async_trait::async_trait]
pub trait ResourceProvider: Send + Sync {
    fn name(&self) -> &str;
    fn supports(&self, resource_type: &ResourceType) -> bool;
    async fn provision(&self, resource: &crate::state::InfraResource) -> ProvisionResult;
    async fn deprovision(
        &self,
        provider_id: &str,
        resource_type: &ResourceType,
    ) -> Result<(), String>;
    async fn describe(
        &self,
        provider_id: &str,
        resource_type: &ResourceType,
    ) -> Result<serde_json::Value, String>;
}

// ── MockProvider ─────────────────────────────────────────────────────────────

/// Mock provider for testing.
pub struct MockProvider {
    pub provider_name: String,
    pub supported_types: Vec<ResourceType>,
    pub should_fail: bool,
}

impl MockProvider {
    pub fn new(name: &str, types: Vec<ResourceType>) -> Self {
        Self {
            provider_name: name.to_string(),
            supported_types: types,
            should_fail: false,
        }
    }

    pub fn failing(name: &str, types: Vec<ResourceType>) -> Self {
        Self {
            provider_name: name.to_string(),
            supported_types: types,
            should_fail: true,
        }
    }
}

#[async_trait::async_trait]
impl ResourceProvider for MockProvider {
    fn name(&self) -> &str {
        &self.provider_name
    }

    fn supports(&self, rt: &ResourceType) -> bool {
        self.supported_types.contains(rt)
    }

    async fn provision(&self, resource: &crate::state::InfraResource) -> ProvisionResult {
        if self.should_fail {
            ProvisionResult {
                resource_id: resource.id.clone(),
                provider_id: String::new(),
                provider: self.provider_name.clone(),
                actual_state: serde_json::Value::Null,
                success: false,
                error: Some("mock failure".to_string()),
            }
        } else {
            ProvisionResult {
                resource_id: resource.id.clone(),
                provider_id: format!("prov-{}", uuid::Uuid::new_v4()),
                provider: self.provider_name.clone(),
                actual_state: resource.spec.clone(),
                success: true,
                error: None,
            }
        }
    }

    async fn deprovision(&self, _id: &str, _rt: &ResourceType) -> Result<(), String> {
        if self.should_fail {
            Err("mock failure".to_string())
        } else {
            Ok(())
        }
    }

    async fn describe(
        &self,
        id: &str,
        _rt: &ResourceType,
    ) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({ "id": id, "status": "running" }))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::InfraResource;

    fn make_vm_resource() -> InfraResource {
        InfraResource::new(
            "vm-001",
            ResourceType::Vm,
            "mock",
            "test-vm",
            "tenant-abc",
            serde_json::json!({ "cpu_cores": 2, "memory_gb": 4 }),
        )
    }

    #[tokio::test]
    async fn test_mock_provider_provisions_vm() {
        let provider = MockProvider::new("mock", vec![ResourceType::Vm]);
        let resource = make_vm_resource();

        assert!(provider.supports(&ResourceType::Vm));
        assert!(!provider.supports(&ResourceType::Vpc));

        let result = provider.provision(&resource).await;
        assert!(result.success);
        assert!(result.error.is_none());
        assert_eq!(result.resource_id, "vm-001");
        assert!(!result.provider_id.is_empty());
        assert_eq!(result.provider, "mock");
    }

    #[tokio::test]
    async fn test_mock_failing_provider() {
        let provider = MockProvider::failing("fail-cloud", vec![ResourceType::Vm, ResourceType::Vpc]);
        let resource = make_vm_resource();

        let result = provider.provision(&resource).await;
        assert!(!result.success);
        assert_eq!(result.error.as_deref(), Some("mock failure"));

        let deprov = provider
            .deprovision("prov-xyz", &ResourceType::Vm)
            .await;
        assert!(deprov.is_err());
    }
}
