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

use crate::models::NodeClass;
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

impl HetznerNodeClassSpec {
    /// API group of the first-party Hetzner NodeClass.
    pub const GROUP: &'static str = "karpenter.hetzner.cloud";
    pub const KIND: &'static str = "HetznerNodeClass";
}

impl AzureNodeClassSpec {
    /// API group of `karpenter-provider-azure`.
    pub const GROUP: &'static str = "karpenter.azure.com";
    pub const KIND: &'static str = "AKSNodeClass";
}

/// AWS NodeClass envelope — fields mirror
/// `karpenter-provider-aws`/`pkg/apis/v1/ec2nodeclass_types.go`. The
/// concrete EC2 fleet dispatch lands in cave-cloud-controller-manager's
/// AWS track; only the envelope shape lives here.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Ec2NodeClassSpec {
    /// IAM instance profile bound to the launched node.
    pub instance_profile: String,
    /// `AL2`, `AL2023`, `Bottlerocket`, `Ubuntu`, `Custom`.
    pub ami_family: String,
    /// Selector terms resolving the subnets the node may launch into.
    pub subnet_selector_terms: Vec<String>,
    /// Selector terms resolving the security groups attached to the node.
    pub security_group_selector_terms: Vec<String>,
    /// Selector terms resolving the AMI(s) (empty → AMIFamily default).
    #[serde(default)]
    pub ami_selector_terms: Vec<String>,
    #[serde(default)]
    pub tags: std::collections::BTreeMap<String, String>,
}

impl Ec2NodeClassSpec {
    /// API group of `karpenter-provider-aws`.
    pub const GROUP: &'static str = "karpenter.k8s.aws";
    pub const KIND: &'static str = "EC2NodeClass";
}

/// GCP NodeClass envelope — fields mirror the community
/// `karpenter-provider-gcp`/`GCENodeClass`. GCE instance dispatch lands
/// alongside cave-cloud-controller-manager's GCP track.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GceNodeClassSpec {
    /// `n2`, `e2`, `c3`, etc.
    pub machine_family: String,
    /// `cos-stable`, `ubuntu-2204-lts`, etc.
    pub image_family: String,
    pub region: String,
    pub service_account: Option<String>,
    #[serde(default)]
    pub network_tags: Vec<String>,
    #[serde(default)]
    pub labels: std::collections::BTreeMap<String, String>,
}

impl GceNodeClassSpec {
    /// API group of the GCP provider.
    pub const GROUP: &'static str = "karpenter.k8s.gcp";
    pub const KIND: &'static str = "GCENodeClass";
}

/// The `(group, kind)` pair the registry keys on — mirrors upstream
/// `CloudProvider.GetSupportedNodeClasses()` returning the status.Object
/// GroupVersionKind a provider owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeClassKind {
    pub group: String,
    pub kind: String,
}

/// One provider entry in the [`ProviderRegistry`]. `name` is the
/// CloudProvider implementation name (`Name()` upstream); `kind` is the
/// NodeClass GVK it owns; `provider` is the dispatch target.
pub struct ProviderRegistration {
    pub name: String,
    pub kind: NodeClassKind,
    pub provider: Box<dyn CloudProvider + Send + Sync>,
}

/// Routes a `NodeClass` to the cloud provider that owns its GVK. This is
/// the cave-karpenter analogue of Karpenter's multi-cloud build: each
/// concrete provider (AWS, GCP, Azure, Hetzner) registers the NodeClass
/// kind it understands, and the core dispatches lifecycle calls through
/// the [`CloudProvider`] trait without favouring any one cloud.
#[derive(Default)]
pub struct ProviderRegistry {
    regs: Vec<ProviderRegistration>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self { regs: Vec::new() }
    }

    /// Register a provider for its NodeClass GVK. A later registration for
    /// the same GVK shadows an earlier one (no aliases — pre-OSS breaking
    /// changes are acceptable per the golden rules).
    pub fn register(&mut self, reg: ProviderRegistration) {
        self.regs
            .retain(|r| r.kind != reg.kind || r.name != reg.name);
        self.regs.push(reg);
    }

    /// Provider names, sorted — equal-level listing with no privileged
    /// first-party (memory runtime_oss_no_hetzner_branding).
    pub fn names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.regs.iter().map(|r| r.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    /// Resolve the registration owning `nc`'s group/kind, or `None`.
    pub fn for_node_class(&self, nc: &NodeClass) -> Option<&ProviderRegistration> {
        self.regs
            .iter()
            .find(|r| r.kind.group == nc.group && r.kind.kind == nc.kind)
    }

    /// Dispatch a create to the provider owning `nc`. Mirrors upstream
    /// `CloudProvider.Create` selected by the NodeClaim's NodeClassRef.
    pub fn create_for(
        &self,
        nc: &NodeClass,
        instance_type: &str,
        zone: &str,
    ) -> ProviderResult<String> {
        self.dispatch(nc)?.provider.create(instance_type, zone)
    }

    /// Dispatch an idempotent delete to the provider owning `nc`.
    pub fn delete_for(&self, nc: &NodeClass, provider_id: &str) -> ProviderResult<()> {
        self.dispatch(nc)?.provider.delete(provider_id)
    }

    /// Dispatch an existence check to the provider owning `nc`.
    pub fn exists_for(&self, nc: &NodeClass, provider_id: &str) -> ProviderResult<bool> {
        self.dispatch(nc)?.provider.exists(provider_id)
    }

    fn dispatch(&self, nc: &NodeClass) -> ProviderResult<&ProviderRegistration> {
        self.for_node_class(nc).ok_or_else(|| {
            ProviderError::NotFound(format!(
                "no provider registered for NodeClass {}/{}",
                nc.group, nc.kind
            ))
        })
    }
}

/// Build the default registry with AWS / GCP / Azure / Hetzner wired to
/// in-memory [`StaticProvider`]s. The concrete cloud-side clients replace
/// these in cave-cloud-controller-manager; the GVK routing is identical.
pub fn default_registry() -> ProviderRegistry {
    let mut r = ProviderRegistry::new();
    for (name, group, kind) in [
        ("aws", Ec2NodeClassSpec::GROUP, Ec2NodeClassSpec::KIND),
        ("azure", AzureNodeClassSpec::GROUP, AzureNodeClassSpec::KIND),
        ("gcp", GceNodeClassSpec::GROUP, GceNodeClassSpec::KIND),
        (
            "hetzner",
            HetznerNodeClassSpec::GROUP,
            HetznerNodeClassSpec::KIND,
        ),
    ] {
        r.register(ProviderRegistration {
            name: name.to_string(),
            kind: NodeClassKind {
                group: group.to_string(),
                kind: kind.to_string(),
            },
            provider: Box::new(StaticProvider::new()),
        });
    }
    r
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
