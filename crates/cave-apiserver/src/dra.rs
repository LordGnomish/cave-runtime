// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! DynamicResourceAllocation (DRA) — KEP-4381, v1.32 beta.
//!
//! Mirrors `staging/src/k8s.io/api/resource/v1beta1/types.go` and the
//! associated registry under `pkg/apis/resource/`. Upstream models DRA
//! as three GVKs in the `resource.k8s.io/v1beta1` API group:
//!
//! * **ResourceClass** — cluster-scoped policy describing a class of
//!   shared, opaque devices a driver can provision.
//! * **ResourceClaim** — namespaced request for one or more devices
//!   of a particular class; lifecycle owner-bound to the consuming
//!   Pods that reference it.
//! * **PodSchedulingContext** — per-pod, per-namespace scratchpad the
//!   scheduler writes to while orchestrating multi-claim allocation.
//!
//! cave-apiserver previously rejected the `resource.k8s.io` group as
//! an unknown CRD. This module registers the type surface and
//! exposes the lifecycle hooks the scheduler and controller-manager
//! need. The reconciler itself is delivered by the kube-controller-
//! manager track; this file is the **apiserver-side** part of the
//! port.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::RwLock;

pub const GROUP: &str = "resource.k8s.io";
pub const VERSION: &str = "v1beta1";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ResourceClass {
    pub name: String,
    pub driver_name: String,
    pub parameters_ref: Option<ParametersReference>,
    pub suitable_nodes: Option<NodeSelector>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ParametersReference {
    pub api_group: String,
    pub kind: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct NodeSelector {
    pub match_labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum AllocationMode {
    #[default]
    WaitForFirstConsumer,
    Immediate,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResourceClaim {
    pub namespace: String,
    pub name: String,
    pub class_name: String,
    pub allocation_mode: AllocationMode,
    pub parameters_ref: Option<ParametersReference>,
    pub status: ResourceClaimStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResourceClaimStatus {
    pub allocation: Option<AllocationResult>,
    pub reserved_for: Vec<ResourceClaimConsumerRef>,
    pub deallocation_requested: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct AllocationResult {
    pub node_name: String,
    pub resource_handles: Vec<ResourceHandle>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ResourceHandle {
    pub driver_name: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ResourceClaimConsumerRef {
    pub api_group: String,
    pub resource: String,
    pub name: String,
    pub uid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PodSchedulingContext {
    pub namespace: String,
    pub name: String,
    pub selected_node: Option<String>,
    pub potential_nodes: Vec<String>,
    pub resource_claims: Vec<PodResourceClaimSchedulingStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PodResourceClaimSchedulingStatus {
    pub name: String,
    pub unsuitable_nodes: Vec<String>,
}

#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub enum DraError {
    #[error("DRA feature gate is disabled")]
    Disabled,
    #[error("resource class {0:?} not found")]
    ClassNotFound(String),
    #[error("resource claim {0:?}/{1:?} not found")]
    ClaimNotFound(String, String),
    #[error("resource class name must be non-empty")]
    EmptyClassName,
    #[error("driver name must be non-empty")]
    EmptyDriverName,
    #[error("claim already allocated")]
    AlreadyAllocated,
    #[error("claim still has {0} consumer(s) — cannot deallocate")]
    StillReserved(usize),
}

pub type DraResult<T> = Result<T, DraError>;

#[derive(Default)]
pub struct DraRegistry {
    enabled: std::sync::atomic::AtomicBool,
    classes: RwLock<BTreeMap<String, ResourceClass>>,
    claims: RwLock<BTreeMap<(String, String), ResourceClaim>>,
    contexts: RwLock<BTreeMap<(String, String), PodSchedulingContext>>,
}

impl DraRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enable(&self) {
        self.enabled
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn disable(&self) {
        self.enabled
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn require_enabled(&self) -> DraResult<()> {
        if self.is_enabled() {
            Ok(())
        } else {
            Err(DraError::Disabled)
        }
    }

    // ── ResourceClass ────────────────────────────────────────────────
    pub fn create_class(&self, c: ResourceClass) -> DraResult<ResourceClass> {
        self.require_enabled()?;
        if c.name.is_empty() {
            return Err(DraError::EmptyClassName);
        }
        if c.driver_name.is_empty() {
            return Err(DraError::EmptyDriverName);
        }
        self.classes
            .write()
            .unwrap()
            .insert(c.name.clone(), c.clone());
        Ok(c)
    }

    pub fn get_class(&self, name: &str) -> DraResult<ResourceClass> {
        self.require_enabled()?;
        self.classes
            .read()
            .unwrap()
            .get(name)
            .cloned()
            .ok_or_else(|| DraError::ClassNotFound(name.into()))
    }

    pub fn list_classes(&self) -> DraResult<Vec<ResourceClass>> {
        self.require_enabled()?;
        Ok(self.classes.read().unwrap().values().cloned().collect())
    }

    pub fn delete_class(&self, name: &str) -> DraResult<ResourceClass> {
        self.require_enabled()?;
        self.classes
            .write()
            .unwrap()
            .remove(name)
            .ok_or_else(|| DraError::ClassNotFound(name.into()))
    }

    // ── ResourceClaim ────────────────────────────────────────────────
    pub fn create_claim(&self, c: ResourceClaim) -> DraResult<ResourceClaim> {
        self.require_enabled()?;
        // Class must exist.
        if !self.classes.read().unwrap().contains_key(&c.class_name) {
            return Err(DraError::ClassNotFound(c.class_name.clone()));
        }
        self.claims
            .write()
            .unwrap()
            .insert((c.namespace.clone(), c.name.clone()), c.clone());
        Ok(c)
    }

    pub fn get_claim(&self, ns: &str, name: &str) -> DraResult<ResourceClaim> {
        self.require_enabled()?;
        self.claims
            .read()
            .unwrap()
            .get(&(ns.to_string(), name.to_string()))
            .cloned()
            .ok_or_else(|| DraError::ClaimNotFound(ns.into(), name.into()))
    }

    pub fn allocate(
        &self,
        ns: &str,
        name: &str,
        result: AllocationResult,
    ) -> DraResult<ResourceClaim> {
        self.require_enabled()?;
        let mut g = self.claims.write().unwrap();
        let key = (ns.to_string(), name.to_string());
        let claim = g
            .get_mut(&key)
            .ok_or_else(|| DraError::ClaimNotFound(ns.into(), name.into()))?;
        if claim.status.allocation.is_some() {
            return Err(DraError::AlreadyAllocated);
        }
        claim.status.allocation = Some(result);
        Ok(claim.clone())
    }

    pub fn reserve_for(
        &self,
        ns: &str,
        name: &str,
        consumer: ResourceClaimConsumerRef,
    ) -> DraResult<ResourceClaim> {
        self.require_enabled()?;
        let mut g = self.claims.write().unwrap();
        let claim = g
            .get_mut(&(ns.to_string(), name.to_string()))
            .ok_or_else(|| DraError::ClaimNotFound(ns.into(), name.into()))?;
        if !claim.status.reserved_for.contains(&consumer) {
            claim.status.reserved_for.push(consumer);
        }
        Ok(claim.clone())
    }

    pub fn unreserve(&self, ns: &str, name: &str, uid: &str) -> DraResult<ResourceClaim> {
        self.require_enabled()?;
        let mut g = self.claims.write().unwrap();
        let claim = g
            .get_mut(&(ns.to_string(), name.to_string()))
            .ok_or_else(|| DraError::ClaimNotFound(ns.into(), name.into()))?;
        claim.status.reserved_for.retain(|c| c.uid != uid);
        Ok(claim.clone())
    }

    pub fn deallocate(&self, ns: &str, name: &str) -> DraResult<ResourceClaim> {
        self.require_enabled()?;
        let mut g = self.claims.write().unwrap();
        let claim = g
            .get_mut(&(ns.to_string(), name.to_string()))
            .ok_or_else(|| DraError::ClaimNotFound(ns.into(), name.into()))?;
        if !claim.status.reserved_for.is_empty() {
            return Err(DraError::StillReserved(claim.status.reserved_for.len()));
        }
        claim.status.allocation = None;
        claim.status.deallocation_requested = false;
        Ok(claim.clone())
    }

    // ── PodSchedulingContext ────────────────────────────────────────
    pub fn upsert_context(&self, ctx: PodSchedulingContext) -> DraResult<PodSchedulingContext> {
        self.require_enabled()?;
        self.contexts
            .write()
            .unwrap()
            .insert((ctx.namespace.clone(), ctx.name.clone()), ctx.clone());
        Ok(ctx)
    }

    pub fn get_context(&self, ns: &str, name: &str) -> DraResult<PodSchedulingContext> {
        self.require_enabled()?;
        self.contexts
            .read()
            .unwrap()
            .get(&(ns.to_string(), name.to_string()))
            .cloned()
            .ok_or_else(|| DraError::ClaimNotFound(ns.into(), name.into()))
    }

    pub fn class_count(&self) -> usize {
        self.classes.read().unwrap().len()
    }
    pub fn claim_count(&self) -> usize {
        self.claims.read().unwrap().len()
    }
    pub fn context_count(&self) -> usize {
        self.contexts.read().unwrap().len()
    }
}

/// `apiserver.RegistryEntry` equivalent — what the discovery layer
/// publishes when DRA is enabled.
pub fn registry_entries() -> Vec<(&'static str, &'static str, &'static str, bool)> {
    vec![
        (GROUP, VERSION, "ResourceClass", false),
        (GROUP, VERSION, "ResourceClaim", true),
        (GROUP, VERSION, "PodSchedulingContext", true),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r() -> DraRegistry {
        let r = DraRegistry::new();
        r.enable();
        r
    }

    fn class(name: &str) -> ResourceClass {
        ResourceClass {
            name: name.into(),
            driver_name: "gpu.example.com".into(),
            ..Default::default()
        }
    }

    fn claim(ns: &str, name: &str, class: &str) -> ResourceClaim {
        ResourceClaim {
            namespace: ns.into(),
            name: name.into(),
            class_name: class.into(),
            ..Default::default()
        }
    }

    #[test]
    fn disabled_registry_rejects_all_calls() {
        let r = DraRegistry::new();
        assert_eq!(r.list_classes().unwrap_err(), DraError::Disabled);
        assert_eq!(r.get_claim("ns", "c").unwrap_err(), DraError::Disabled);
        assert!(!r.is_enabled());
    }

    #[test]
    fn class_lifecycle_create_get_list_delete() {
        let r = r();
        r.create_class(class("gpu-fast")).unwrap();
        assert_eq!(r.class_count(), 1);
        let g = r.get_class("gpu-fast").unwrap();
        assert_eq!(g.driver_name, "gpu.example.com");
        let all = r.list_classes().unwrap();
        assert_eq!(all.len(), 1);
        r.delete_class("gpu-fast").unwrap();
        assert_eq!(r.class_count(), 0);
    }

    #[test]
    fn class_create_rejects_empty_name_and_driver() {
        let r = r();
        let mut c = class("");
        c.driver_name = "d".into();
        assert_eq!(r.create_class(c).unwrap_err(), DraError::EmptyClassName);

        let mut c = class("ok");
        c.driver_name = "".into();
        assert_eq!(r.create_class(c).unwrap_err(), DraError::EmptyDriverName);
    }

    #[test]
    fn class_get_unknown_is_not_found() {
        let r = r();
        let err = r.get_class("missing").unwrap_err();
        assert_eq!(err, DraError::ClassNotFound("missing".into()));
    }

    #[test]
    fn claim_create_requires_existing_class() {
        let r = r();
        let err = r.create_claim(claim("default", "c1", "no-such")).unwrap_err();
        assert_eq!(err, DraError::ClassNotFound("no-such".into()));
    }

    #[test]
    fn allocate_then_double_allocate_fails() {
        let r = r();
        r.create_class(class("gpu")).unwrap();
        r.create_claim(claim("default", "c1", "gpu")).unwrap();
        let alloc = AllocationResult {
            node_name: "node-a".into(),
            resource_handles: vec![],
        };
        r.allocate("default", "c1", alloc.clone()).unwrap();
        let err = r.allocate("default", "c1", alloc).unwrap_err();
        assert_eq!(err, DraError::AlreadyAllocated);
    }

    #[test]
    fn deallocate_blocked_by_reservations() {
        let r = r();
        r.create_class(class("gpu")).unwrap();
        r.create_claim(claim("default", "c1", "gpu")).unwrap();
        let consumer = ResourceClaimConsumerRef {
            api_group: "".into(),
            resource: "pods".into(),
            name: "p1".into(),
            uid: "uid-1".into(),
        };
        r.reserve_for("default", "c1", consumer.clone()).unwrap();
        let err = r.deallocate("default", "c1").unwrap_err();
        assert_eq!(err, DraError::StillReserved(1));
        r.unreserve("default", "c1", "uid-1").unwrap();
        r.deallocate("default", "c1").unwrap();
    }

    #[test]
    fn reserve_for_is_idempotent() {
        let r = r();
        r.create_class(class("gpu")).unwrap();
        r.create_claim(claim("default", "c1", "gpu")).unwrap();
        let consumer = ResourceClaimConsumerRef {
            api_group: "".into(),
            resource: "pods".into(),
            name: "p1".into(),
            uid: "uid-1".into(),
        };
        r.reserve_for("default", "c1", consumer.clone()).unwrap();
        r.reserve_for("default", "c1", consumer.clone()).unwrap();
        let c = r.get_claim("default", "c1").unwrap();
        assert_eq!(c.status.reserved_for.len(), 1);
    }

    #[test]
    fn context_upsert_overwrites_previous() {
        let r = r();
        let mut ctx = PodSchedulingContext {
            namespace: "default".into(),
            name: "p1".into(),
            selected_node: None,
            potential_nodes: vec!["a".into(), "b".into()],
            resource_claims: vec![],
        };
        r.upsert_context(ctx.clone()).unwrap();
        ctx.selected_node = Some("b".into());
        r.upsert_context(ctx).unwrap();
        let got = r.get_context("default", "p1").unwrap();
        assert_eq!(got.selected_node.as_deref(), Some("b"));
    }

    #[test]
    fn registry_entries_describe_three_kinds() {
        let e = registry_entries();
        assert_eq!(e.len(), 3);
        assert!(e.iter().any(|(_, _, k, _)| *k == "ResourceClass"));
        assert!(e.iter().any(|(_, _, k, _)| *k == "ResourceClaim"));
        assert!(e.iter().any(|(_, _, k, _)| *k == "PodSchedulingContext"));
        // ResourceClass is cluster-scoped, the other two are namespaced.
        let class = e.iter().find(|(_, _, k, _)| *k == "ResourceClass").unwrap();
        assert!(!class.3);
        let claim = e.iter().find(|(_, _, k, _)| *k == "ResourceClaim").unwrap();
        assert!(claim.3);
    }
}
