// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Hetzner instance metadata lookup — additive parity-deepening helper.
//!
//! Mirrors `hcloud-cloud-controller-manager/hcloud/instances.go` — the
//! four cloud-provider methods that translate a `node.metadata.name` into
//! the four pieces of cloud metadata Kubernetes reads back:
//!
//! * `InstanceID(node)`     → `"hcloud://<server-id>"`
//! * `NodeAddresses(node)`  → `Vec<NodeAddress>` (internal IPv4 + external IPv4/v6 + hostname)
//! * `InstanceType(node)`   → server type (`cpx21`, `cax11`, ...)
//! * `Zone(node)`           → failure-domain region/zone pair
//!
//! The real Hetzner SDK call is hidden behind an `HcloudClient` trait so
//! the metadata helper can be unit-tested without network. A trivial
//! `InMemoryHcloudClient` is provided for tests and embedded scenarios.

use crate::providers::hetzner::PROVIDER_VERSION;
use crate::types::{Cite, CloudError, ProviderName};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Provider-id scheme — identical to the legacy `hcloud://`.
pub const PROVIDER_ID_SCHEME: &str = "hcloud";

/// One row of Hetzner-side metadata for a Kubernetes node. Matches the
/// fields of `*hcloud.Server` that the cloud-provider methods consume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HcloudServerRow {
    pub id: u64,
    pub name: String,
    pub server_type: String,
    pub location: String,
    /// Hetzner exposes a single zone per location; mirror that here so the
    /// caller can return both pieces from one record.
    pub zone: String,
    pub private_ipv4: Option<String>,
    pub public_ipv4: Option<String>,
    pub public_ipv6: Option<String>,
}

impl HcloudServerRow {
    pub fn provider_id(&self) -> String {
        format!("{}://{}", PROVIDER_ID_SCHEME, self.id)
    }
}

/// Subset of `NodeAddress` the cloud-provider returns. Strings instead of an
/// enum because callers turn this straight into Kubernetes labels.
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

/// Pluggable client interface — `HcloudInstanceLookup` calls only this.
#[async_trait]
pub trait HcloudClient: Send + Sync {
    async fn server_by_name(&self, name: &str) -> Result<Option<HcloudServerRow>, CloudError>;
}

/// Test/embedded implementation — keeps an in-memory map of server rows.
#[derive(Debug, Default, Clone)]
pub struct InMemoryHcloudClient {
    inner: Arc<RwLock<HashMap<String, HcloudServerRow>>>,
}

impl InMemoryHcloudClient {
    pub fn new() -> Self {
        Self::default()
    }
    pub async fn insert(&self, row: HcloudServerRow) {
        let mut g = self.inner.write().await;
        g.insert(row.name.clone(), row);
    }
    pub async fn len(&self) -> usize {
        self.inner.read().await.len()
    }
}

#[async_trait]
impl HcloudClient for InMemoryHcloudClient {
    async fn server_by_name(&self, name: &str) -> Result<Option<HcloudServerRow>, CloudError> {
        let g = self.inner.read().await;
        Ok(g.get(name).cloned())
    }
}

/// Helper that owns an `HcloudClient` and exposes the four cloud-provider
/// methods. One per cluster.
pub struct HcloudInstanceLookup<C: HcloudClient> {
    client: C,
}

impl<C: HcloudClient> HcloudInstanceLookup<C> {
    pub fn new(client: C) -> Self {
        Self { client }
    }

    /// `InstanceID(ctx, name)` → `"hcloud://<id>"`. Returns
    /// `CloudError::Upstream` when the node is unknown to Hetzner so the
    /// upper controller can drive `node-lifecycle` cleanup.
    pub async fn instance_id(&self, node_name: &str) -> Result<String, CloudError> {
        let row = self
            .client
            .server_by_name(node_name)
            .await?
            .ok_or_else(|| CloudError::Upstream {
                provider: ProviderName::Hetzner,
                reason: format!("server {node_name} not found"),
            })?;
        Ok(row.provider_id())
    }

    /// `NodeAddresses(ctx, name)`. Returns hostname + at most one internal
    /// + at most one external IPv4 + at most one external IPv6, in the
    /// upstream-canonical order.
    pub async fn node_addresses(&self, node_name: &str) -> Result<Vec<NodeAddress>, CloudError> {
        let row = self
            .client
            .server_by_name(node_name)
            .await?
            .ok_or_else(|| CloudError::Upstream {
                provider: ProviderName::Hetzner,
                reason: format!("server {node_name} not found"),
            })?;
        let mut out = Vec::with_capacity(4);
        out.push(NodeAddress {
            kind: NodeAddressType::Hostname,
            address: row.name.clone(),
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
        if let Some(ip) = &row.public_ipv6 {
            out.push(NodeAddress {
                kind: NodeAddressType::ExternalIP,
                address: ip.clone(),
            });
        }
        Ok(out)
    }

    /// `InstanceType(ctx, name)` → server-type slug.
    pub async fn instance_type(&self, node_name: &str) -> Result<String, CloudError> {
        let row = self
            .client
            .server_by_name(node_name)
            .await?
            .ok_or_else(|| CloudError::Upstream {
                provider: ProviderName::Hetzner,
                reason: format!("server {node_name} not found"),
            })?;
        Ok(row.server_type)
    }

    /// `Zone(ctx, name)` → `(failure_domain, region)` — Hetzner location
    /// maps to *both* (zone == location, since each location has 1 zone).
    pub async fn zone(&self, node_name: &str) -> Result<(String, String), CloudError> {
        let row = self
            .client
            .server_by_name(node_name)
            .await?
            .ok_or_else(|| CloudError::Upstream {
                provider: ProviderName::Hetzner,
                reason: format!("server {node_name} not found"),
            })?;
        Ok((row.zone, row.location))
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::ext(
    "hetznercloud/hcloud-cloud-controller-manager",
    "hcloud/instances.go",
    "Instances.InstanceID",
    PROVIDER_VERSION,
);

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_row(name: &str, id: u64) -> HcloudServerRow {
        HcloudServerRow {
            id,
            name: name.to_string(),
            server_type: "cpx21".to_string(),
            location: "fsn1".to_string(),
            zone: "fsn1-dc14".to_string(),
            private_ipv4: Some("10.0.0.5".into()),
            public_ipv4: Some("203.0.113.7".into()),
            public_ipv6: Some("2a01:4f8::7".into()),
        }
    }

    #[tokio::test]
    async fn provider_id_uses_hcloud_scheme() {
        let client = InMemoryHcloudClient::new();
        client.insert(sample_row("node-a", 42)).await;
        let l = HcloudInstanceLookup::new(client);
        assert_eq!(l.instance_id("node-a").await.unwrap(), "hcloud://42");
    }

    #[tokio::test]
    async fn unknown_node_returns_upstream_error() {
        let client = InMemoryHcloudClient::new();
        let l = HcloudInstanceLookup::new(client);
        let err = l.instance_id("ghost").await.unwrap_err();
        assert!(matches!(err, CloudError::Upstream { .. }));
    }

    #[tokio::test]
    async fn node_addresses_emits_hostname_internal_external_in_order() {
        let client = InMemoryHcloudClient::new();
        client.insert(sample_row("node-a", 42)).await;
        let l = HcloudInstanceLookup::new(client);
        let addrs = l.node_addresses("node-a").await.unwrap();
        assert_eq!(addrs[0].kind, NodeAddressType::Hostname);
        assert_eq!(addrs[1].kind, NodeAddressType::InternalIP);
        assert_eq!(addrs[1].address, "10.0.0.5");
        assert_eq!(addrs[2].kind, NodeAddressType::ExternalIP);
        assert_eq!(addrs[2].address, "203.0.113.7");
        assert_eq!(addrs[3].kind, NodeAddressType::ExternalIP);
        assert_eq!(addrs[3].address, "2a01:4f8::7");
    }

    #[tokio::test]
    async fn node_addresses_omits_missing_ip_fields() {
        let client = InMemoryHcloudClient::new();
        let mut row = sample_row("node-a", 1);
        row.public_ipv6 = None;
        row.private_ipv4 = None;
        client.insert(row).await;
        let l = HcloudInstanceLookup::new(client);
        let addrs = l.node_addresses("node-a").await.unwrap();
        // hostname + one external IPv4 only.
        assert_eq!(addrs.len(), 2);
        assert_eq!(addrs[0].kind, NodeAddressType::Hostname);
        assert_eq!(addrs[1].kind, NodeAddressType::ExternalIP);
    }

    #[tokio::test]
    async fn instance_type_returns_server_slug() {
        let client = InMemoryHcloudClient::new();
        client.insert(sample_row("node-a", 1)).await;
        let l = HcloudInstanceLookup::new(client);
        assert_eq!(l.instance_type("node-a").await.unwrap(), "cpx21");
    }

    #[tokio::test]
    async fn zone_returns_zone_and_region_in_that_order() {
        let client = InMemoryHcloudClient::new();
        client.insert(sample_row("node-a", 1)).await;
        let l = HcloudInstanceLookup::new(client);
        let (zone, region) = l.zone("node-a").await.unwrap();
        assert_eq!(zone, "fsn1-dc14");
        assert_eq!(region, "fsn1");
    }

    #[tokio::test]
    async fn in_memory_client_len_reflects_inserts() {
        let client = InMemoryHcloudClient::new();
        assert_eq!(client.len().await, 0);
        client.insert(sample_row("a", 1)).await;
        client.insert(sample_row("b", 2)).await;
        assert_eq!(client.len().await, 2);
    }
}
