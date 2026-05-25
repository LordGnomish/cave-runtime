// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Azure instance metadata lookup — additive parity-deepening helper.
//!
//! Mirrors `cloud-provider-azure/pkg/provider/azure_instances.go`. Same
//! four-method shape as the Hetzner sibling, but emits ARM-style
//! `azure:///subscriptions/<sub>/resourceGroups/<rg>/providers/Microsoft.Compute/virtualMachines/<name>`
//! provider IDs and uses Azure's `Region` + `AvailabilityZone` split.

use crate::providers::azure::PROVIDER_VERSION;
use crate::types::{Cite, CloudError, ProviderName};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Provider-id scheme — note the *triple* slash (ARM URI form).
pub const PROVIDER_ID_PREFIX: &str = "azure://";

/// Row of Azure-side metadata, mapping a Kubernetes node onto the VM it
/// is running on. Mirrors the fields of `*compute.VirtualMachine` that
/// the cloud-provider methods consume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AzureVmRow {
    pub vm_name: String,
    pub resource_group: String,
    pub subscription_id: String,
    pub vm_size: String,
    pub region: String,
    /// Empty for non-zonal regions (`"westus"`); `"1"`/`"2"`/`"3"` for
    /// zonal ones (`"westeurope"`).
    pub availability_zone: String,
    pub private_ipv4: Option<String>,
    pub public_ipv4: Option<String>,
}

impl AzureVmRow {
    pub fn provider_id(&self) -> String {
        format!(
            "{}/subscriptions/{}/resourceGroups/{}/providers/Microsoft.Compute/virtualMachines/{}",
            PROVIDER_ID_PREFIX, self.subscription_id, self.resource_group, self.vm_name
        )
    }
}

/// Subset of Kubernetes `NodeAddress` returned by Azure cloud-provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeAddress {
    pub kind: NodeAddressType,
    pub address: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeAddressType {
    Hostname,
    InternalIP,
    ExternalIP,
}

/// Pluggable Azure SDK client surface.
#[async_trait]
pub trait AzureClient: Send + Sync {
    async fn vm_by_name(&self, name: &str) -> Result<Option<AzureVmRow>, CloudError>;
}

/// Test/embedded implementation backed by an in-memory map.
#[derive(Debug, Default, Clone)]
pub struct InMemoryAzureClient {
    inner: Arc<RwLock<HashMap<String, AzureVmRow>>>,
}

impl InMemoryAzureClient {
    pub fn new() -> Self {
        Self::default()
    }
    pub async fn insert(&self, row: AzureVmRow) {
        let mut g = self.inner.write().await;
        g.insert(row.vm_name.clone(), row);
    }
    pub async fn len(&self) -> usize {
        self.inner.read().await.len()
    }
}

#[async_trait]
impl AzureClient for InMemoryAzureClient {
    async fn vm_by_name(&self, name: &str) -> Result<Option<AzureVmRow>, CloudError> {
        let g = self.inner.read().await;
        Ok(g.get(name).cloned())
    }
}

/// Owns an `AzureClient` and exposes the four cloud-provider methods.
pub struct AzureInstanceLookup<C: AzureClient> {
    client: C,
}

impl<C: AzureClient> AzureInstanceLookup<C> {
    pub fn new(client: C) -> Self {
        Self { client }
    }

    /// `InstanceID(ctx, name)` → ARM URI.
    pub async fn instance_id(&self, node_name: &str) -> Result<String, CloudError> {
        let row = self.row(node_name).await?;
        Ok(row.provider_id())
    }

    /// `NodeAddresses(ctx, name)` — hostname + optional internal + optional
    /// external IP, in upstream-canonical order.
    pub async fn node_addresses(&self, node_name: &str) -> Result<Vec<NodeAddress>, CloudError> {
        let row = self.row(node_name).await?;
        let mut out = Vec::with_capacity(3);
        out.push(NodeAddress {
            kind: NodeAddressType::Hostname,
            address: row.vm_name.clone(),
        });
        if let Some(ip) = &row.private_ipv4 {
            out.push(NodeAddress {
                kind: NodeAddressType::InternalIP,
                address: ip.clone(),
            });
        }
        if let Some(ip) = &row.public_ipv4 {
            out.push(NodeAddress {
                kind: NodeAddressType::ExternalIP,
                address: ip.clone(),
            });
        }
        Ok(out)
    }

    /// `InstanceType(ctx, name)` → VM size string.
    pub async fn instance_type(&self, node_name: &str) -> Result<String, CloudError> {
        Ok(self.row(node_name).await?.vm_size)
    }

    /// `Zone(ctx, name)` → `(failure_domain, region)`. For non-zonal
    /// regions the failure-domain is empty, matching upstream.
    pub async fn zone(&self, node_name: &str) -> Result<(String, String), CloudError> {
        let row = self.row(node_name).await?;
        Ok((row.availability_zone, row.region))
    }

    async fn row(&self, node_name: &str) -> Result<AzureVmRow, CloudError> {
        self.client
            .vm_by_name(node_name)
            .await?
            .ok_or_else(|| CloudError::Upstream {
                provider: ProviderName::Azure,
                reason: format!("VM {node_name} not found"),
            })
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::ext(
    "kubernetes-sigs/cloud-provider-azure",
    "pkg/provider/azure_instances.go",
    "Instances.InstanceID",
    PROVIDER_VERSION,
);

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_row(name: &str) -> AzureVmRow {
        AzureVmRow {
            vm_name: name.to_string(),
            resource_group: "rg-prod".to_string(),
            subscription_id: "00000000-0000-0000-0000-000000000001".to_string(),
            vm_size: "Standard_D4s_v5".to_string(),
            region: "westeurope".to_string(),
            availability_zone: "2".to_string(),
            private_ipv4: Some("10.10.0.5".into()),
            public_ipv4: Some("52.50.0.7".into()),
        }
    }

    #[tokio::test]
    async fn instance_id_renders_full_arm_uri() {
        let client = InMemoryAzureClient::new();
        client.insert(sample_row("aks-node-0")).await;
        let l = AzureInstanceLookup::new(client);
        let id = l.instance_id("aks-node-0").await.unwrap();
        assert_eq!(
            id,
            "azure:///subscriptions/00000000-0000-0000-0000-000000000001/\
             resourceGroups/rg-prod/providers/Microsoft.Compute/virtualMachines/aks-node-0"
        );
    }

    #[tokio::test]
    async fn unknown_vm_returns_upstream_error() {
        let client = InMemoryAzureClient::new();
        let l = AzureInstanceLookup::new(client);
        let err = l.instance_type("ghost").await.unwrap_err();
        assert!(matches!(err, CloudError::Upstream { .. }));
    }

    #[tokio::test]
    async fn node_addresses_emit_in_canonical_order() {
        let client = InMemoryAzureClient::new();
        client.insert(sample_row("aks-node-0")).await;
        let l = AzureInstanceLookup::new(client);
        let addrs = l.node_addresses("aks-node-0").await.unwrap();
        assert_eq!(addrs.len(), 3);
        assert_eq!(addrs[0].kind, NodeAddressType::Hostname);
        assert_eq!(addrs[1].kind, NodeAddressType::InternalIP);
        assert_eq!(addrs[2].kind, NodeAddressType::ExternalIP);
    }

    #[tokio::test]
    async fn node_addresses_omit_missing_public_ip() {
        let client = InMemoryAzureClient::new();
        let mut row = sample_row("aks-node-1");
        row.public_ipv4 = None;
        client.insert(row).await;
        let l = AzureInstanceLookup::new(client);
        let addrs = l.node_addresses("aks-node-1").await.unwrap();
        assert_eq!(addrs.len(), 2);
    }

    #[tokio::test]
    async fn instance_type_returns_vm_size_slug() {
        let client = InMemoryAzureClient::new();
        client.insert(sample_row("aks-node-0")).await;
        let l = AzureInstanceLookup::new(client);
        assert_eq!(l.instance_type("aks-node-0").await.unwrap(), "Standard_D4s_v5");
    }

    #[tokio::test]
    async fn zone_emits_empty_az_for_non_zonal_region() {
        let client = InMemoryAzureClient::new();
        let mut row = sample_row("aks-node-2");
        row.region = "westus".into();
        row.availability_zone = "".into();
        client.insert(row).await;
        let l = AzureInstanceLookup::new(client);
        let (az, region) = l.zone("aks-node-2").await.unwrap();
        assert!(az.is_empty(), "non-zonal regions have no AZ");
        assert_eq!(region, "westus");
    }

    #[tokio::test]
    async fn zone_emits_arm_az_for_zonal_region() {
        let client = InMemoryAzureClient::new();
        client.insert(sample_row("aks-node-0")).await;
        let l = AzureInstanceLookup::new(client);
        let (az, region) = l.zone("aks-node-0").await.unwrap();
        assert_eq!(az, "2");
        assert_eq!(region, "westeurope");
    }
}
