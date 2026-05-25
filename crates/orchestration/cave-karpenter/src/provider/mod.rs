// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CloudProvider abstraction + Hetzner/Azure NodeClass envelope specs.
//!
//! Upstream reference (Karpenter v1.4.0):
//!   pkg/cloudprovider/cloudprovider.go
//!   karpenter-provider-{aws,azure} :: pkg/apis/v1beta1/{ec2,aks}nodeclass_types.go
//!
//! Karpenter's upstream interface is provider-agnostic: each cloud has
//! a `CloudProvider` implementation that knows how to translate a
//! `NodeClass` envelope into a concrete instance create/delete call.
//! The Cave port exposes the same trait surface and ships two
//! provider-specific NodeClass spec structs (Hetzner + Azure) that
//! NodeClass.spec round-trips through. The real cloud-side dispatch
//! lands alongside cave-cloud-controller-manager.

use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("provider unavailable: {0}")]
    Unavailable(String),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("not found: {0}")]
    NotFound(String),
}

pub type ProviderResult<T> = Result<T, ProviderError>;

/// Cloud-provider trait — every NodeClaim lifecycle action goes through
/// this surface. Implementations live in cave-cloud-controller-manager;
/// the in-memory [`StaticProvider`] below is for tests and `cavectl`
/// demo flows.
pub trait CloudProvider {
    /// Allocate an instance of `instance_type` in `zone`. Returns the
    /// new instance's `provider_id` (e.g. `"hcloud://abcd1234"`).
    fn create(&self, instance_type: &str, zone: &str) -> ProviderResult<String>;

    /// Delete the instance with `provider_id`. Idempotent — repeating a
    /// delete on an already-gone instance must NOT error.
    fn delete(&self, provider_id: &str) -> ProviderResult<()>;

    /// True if `provider_id` still exists on the cloud side.
    fn exists(&self, provider_id: &str) -> ProviderResult<bool>;
}

/// In-memory provider for tests / cavectl demo. Tracks created IDs and
/// honours idempotent delete.
#[derive(Debug, Default)]
pub struct StaticProvider {
    counter: Mutex<u64>,
    live: Mutex<std::collections::BTreeSet<String>>,
}

impl StaticProvider {
    pub fn new() -> Self {
        Self::default()
    }
}

impl CloudProvider for StaticProvider {
    fn create(&self, instance_type: &str, zone: &str) -> ProviderResult<String> {
        let mut c = self.counter.lock().unwrap();
        *c += 1;
        let id = format!("static://{instance_type}/{zone}/{}", *c);
        self.live.lock().unwrap().insert(id.clone());
        Ok(id)
    }

    fn delete(&self, provider_id: &str) -> ProviderResult<()> {
        self.live.lock().unwrap().remove(provider_id);
        Ok(())
    }

    fn exists(&self, provider_id: &str) -> ProviderResult<bool> {
        Ok(self.live.lock().unwrap().contains(provider_id))
    }
}

/// Hetzner NodeClass envelope — fields mirror the spec at
/// `karpenter-provider-hetzner` (Cave first-party).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HetznerNodeClassSpec {
    /// `cx21`, `cx32`, `cx42`, etc.
    pub server_type: String,
    pub image: String,
    /// `hel1`, `nbg1`, `fsn1`, `ash`, `hil`.
    pub location: String,
    pub ssh_keys: Vec<String>,
    pub networks: Vec<String>,
}

/// Azure NodeClass envelope — fields mirror
/// `karpenter-provider-azure`/`v1beta1/aksnodeclass_types.go`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AzureNodeClassSpec {
    /// `Standard_D4s_v5`, etc.
    pub vm_size: String,
    pub image_sku: String,
    pub location: String,
    pub subnet_id: Option<String>,
    pub os_disk_size_gb: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_provider_create_then_exists_then_delete() {
        let p = StaticProvider::new();
        let id = p.create("m5.large", "us-east-1a").unwrap();
        assert!(p.exists(&id).unwrap());
        p.delete(&id).unwrap();
        assert!(!p.exists(&id).unwrap());
    }

    #[test]
    fn static_provider_delete_is_idempotent() {
        let p = StaticProvider::new();
        // Delete an unknown id — no error.
        p.delete("static://ghost").unwrap();
    }

    #[test]
    fn hetzner_spec_serde_roundtrip() {
        let s = HetznerNodeClassSpec {
            server_type: "cx21".into(),
            image: "ubuntu-22.04".into(),
            location: "hel1".into(),
            ssh_keys: vec!["root".into()],
            networks: vec!["10.0.0.0/16".into()],
        };
        let j = serde_json::to_value(&s).unwrap();
        let back: HetznerNodeClassSpec = serde_json::from_value(j).unwrap();
        assert_eq!(back.server_type, "cx21");
    }

    #[test]
    fn azure_spec_serde_roundtrip() {
        let s = AzureNodeClassSpec {
            vm_size: "Standard_D4s_v5".into(),
            image_sku: "ubuntu-22.04".into(),
            location: "westeurope".into(),
            subnet_id: None,
            os_disk_size_gb: Some(60),
        };
        let j = serde_json::to_value(&s).unwrap();
        let back: AzureNodeClassSpec = serde_json::from_value(j).unwrap();
        assert_eq!(back.vm_size, "Standard_D4s_v5");
    }
}
