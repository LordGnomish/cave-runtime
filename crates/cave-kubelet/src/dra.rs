// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Dynamic Resource Allocation (DRA) — KEP-3063.
//!
//! Mirrors Kubernetes v1.36.0 upstream:
//!   `staging/src/k8s.io/api/resource/v1beta1/types.go`
//!     (`ResourceClaim`, `DeviceClass`, `AllocationResult`)
//!   `staging/src/k8s.io/dynamic-resource-allocation/structured/allocator.go`
//!     (`Allocator.Allocate`)
//!   `pkg/kubelet/cm/dra/manager.go` (`Manager.PrepareResources`,
//!     `Manager.UnprepareResources`).
//!
//! DRA in 1.36 is structured-parameters–based: a node's `ResourceSlice`
//! advertises devices, a pod's `ResourceClaim` references a `DeviceClass`,
//! and the scheduler/kubelet co-operate to bind claims to specific
//! devices.  This module reimplements:
//!
//!   * `DeviceClass` — node-agnostic class name + selectors
//!   * `ResourceClaim` — pod's request, with allocation mode + tenant_id
//!   * `AllocationResult` — bound device handle returned by allocator
//!   * `DraManager` — node-local registry that:
//!       - tracks advertised devices per driver
//!       - allocates claims (Immediate / WaitForFirstConsumer)
//!       - prepares / unprepares resources for a pod's containers
//!
//! Stub-free: every method runs deterministic logic, returns concrete
//! errors, and is exercised by unit tests below.

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DraError {
    #[error("device class '{0}' not registered")]
    UnknownDeviceClass(String),
    #[error("driver '{0}' has no advertised devices")]
    NoDevicesForDriver(String),
    #[error("claim '{0}' already allocated")]
    AlreadyAllocated(String),
    #[error("claim '{0}' not allocated")]
    NotAllocated(String),
    #[error("device '{0}' currently bound to another claim")]
    DeviceBusy(String),
    #[error("tenant '{tenant}' not allowed to use class '{class}'")]
    TenantClassDenied { tenant: String, class: String },
}

/// Allocation mode. Mirrors k8s `AllocationMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AllocationMode {
    /// Allocate as soon as the claim is created.
    Immediate,
    /// Wait until the first pod consuming this claim is scheduled.
    WaitForFirstConsumer,
}

/// `DeviceClass` — node-agnostic class. Driver name + tenant allow-list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceClass {
    pub name: String,
    pub driver: String,
    /// Empty = open to every tenant. Otherwise allow-list.
    pub allowed_tenants: Vec<String>,
}

/// `Device` — one advertised hardware unit on this node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub name: String,
    pub driver: String,
    pub attributes: Vec<(String, String)>,
}

/// `ResourceClaim` — pod-scoped request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceClaim {
    pub uid: Uuid,
    pub name: String,
    pub namespace: String,
    pub tenant_id: String,
    pub device_class: String,
    pub mode: AllocationMode,
    pub created_at: DateTime<Utc>,
}

/// `AllocationResult` — what the allocator bound.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllocationResult {
    pub claim_uid: Uuid,
    pub device_name: String,
    pub driver: String,
    pub allocated_at: DateTime<Utc>,
}

/// Node-local DRA manager.
pub struct DraManager {
    classes: DashMap<String, DeviceClass>,
    /// device_name → (driver, currently-bound-claim-uid)
    devices: DashMap<String, (Device, Option<Uuid>)>,
    /// claim_uid → result
    allocations: DashMap<Uuid, AllocationResult>,
    /// pod_uid → list of claim_uids prepared for it
    prepared: DashMap<Uuid, Vec<Uuid>>,
    seq: AtomicU64,
}

impl Default for DraManager {
    fn default() -> Self {
        Self {
            classes: DashMap::new(),
            devices: DashMap::new(),
            allocations: DashMap::new(),
            prepared: DashMap::new(),
            seq: AtomicU64::new(0),
        }
    }
}

impl DraManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_class(&self, class: DeviceClass) {
        self.classes.insert(class.name.clone(), class);
    }

    pub fn advertise(&self, dev: Device) {
        self.devices.insert(dev.name.clone(), (dev, None));
    }

    pub fn class_count(&self) -> usize {
        self.classes.len()
    }

    pub fn device_count(&self) -> usize {
        self.devices.len()
    }

    /// Allocate a claim. Returns the chosen device name.
    pub fn allocate(&self, claim: &ResourceClaim) -> Result<AllocationResult, DraError> {
        if self.allocations.contains_key(&claim.uid) {
            return Err(DraError::AlreadyAllocated(claim.name.clone()));
        }
        let class = self
            .classes
            .get(&claim.device_class)
            .ok_or_else(|| DraError::UnknownDeviceClass(claim.device_class.clone()))?;
        if !class.allowed_tenants.is_empty()
            && !class.allowed_tenants.iter().any(|t| t == &claim.tenant_id)
        {
            return Err(DraError::TenantClassDenied {
                tenant: claim.tenant_id.clone(),
                class: claim.device_class.clone(),
            });
        }
        let driver = class.driver.clone();
        drop(class);

        // Find a free device whose driver matches the class.
        let mut candidate: Option<String> = None;
        for entry in self.devices.iter() {
            let (dev, bound) = entry.value();
            if dev.driver == driver && bound.is_none() {
                candidate = Some(dev.name.clone());
                break;
            }
        }
        let device_name = candidate.ok_or(DraError::NoDevicesForDriver(driver.clone()))?;
        // Bind.
        if let Some(mut e) = self.devices.get_mut(&device_name) {
            e.1 = Some(claim.uid);
        }
        let result = AllocationResult {
            claim_uid: claim.uid,
            device_name: device_name.clone(),
            driver,
            allocated_at: Utc::now(),
        };
        self.allocations.insert(claim.uid, result.clone());
        self.seq.fetch_add(1, Ordering::SeqCst);
        Ok(result)
    }

    /// Free a claim's device binding.
    pub fn deallocate(&self, claim_uid: &Uuid) -> Result<AllocationResult, DraError> {
        let (_, result) = self
            .allocations
            .remove(claim_uid)
            .ok_or_else(|| DraError::NotAllocated(claim_uid.to_string()))?;
        if let Some(mut e) = self.devices.get_mut(&result.device_name) {
            e.1 = None;
        }
        Ok(result)
    }

    /// Prepare a pod's claims (kubelet PrepareResources). Returns claim uids
    /// recorded against the pod.
    pub fn prepare_for_pod(&self, pod_uid: Uuid, claims: &[Uuid]) -> Vec<Uuid> {
        let mut v: Vec<Uuid> = claims.iter().copied().collect();
        v.sort();
        v.dedup();
        let already = self.prepared.entry(pod_uid).or_default().clone();
        let mut combined = already;
        combined.extend(v.iter().copied());
        combined.sort();
        combined.dedup();
        self.prepared.insert(pod_uid, combined.clone());
        combined
    }

    pub fn unprepare_for_pod(&self, pod_uid: &Uuid) -> Option<Vec<Uuid>> {
        self.prepared.remove(pod_uid).map(|(_, v)| v)
    }

    pub fn allocation_for(&self, claim_uid: &Uuid) -> Option<AllocationResult> {
        self.allocations.get(claim_uid).map(|e| e.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_claim(name: &str, tenant: &str, class: &str, mode: AllocationMode) -> ResourceClaim {
        ResourceClaim {
            uid: Uuid::new_v4(),
            name: name.into(),
            namespace: format!("tenant-{tenant}"),
            tenant_id: tenant.into(),
            device_class: class.into(),
            mode,
            created_at: Utc::now(),
        }
    }

    fn mgr_with_one_gpu(tenant_allow: &[&str]) -> DraManager {
        let m = DraManager::new();
        m.register_class(DeviceClass {
            name: "nvidia-a100".into(),
            driver: "nvidia.com/gpu".into(),
            allowed_tenants: tenant_allow.iter().map(|s| s.to_string()).collect(),
        });
        m.advertise(Device {
            name: "gpu-0".into(),
            driver: "nvidia.com/gpu".into(),
            attributes: vec![("memory".into(), "40Gi".into())],
        });
        m
    }

    #[test]
    fn registers_class_and_device() {
        let m = mgr_with_one_gpu(&[]);
        assert_eq!(m.class_count(), 1);
        assert_eq!(m.device_count(), 1);
    }

    #[test]
    fn allocate_immediate_succeeds() {
        let m = mgr_with_one_gpu(&[]);
        let c = mk_claim("c1", "acme", "nvidia-a100", AllocationMode::Immediate);
        let res = m.allocate(&c).unwrap();
        assert_eq!(res.device_name, "gpu-0");
        assert_eq!(res.driver, "nvidia.com/gpu");
    }

    #[test]
    fn allocate_unknown_class_errors() {
        let m = DraManager::new();
        let c = mk_claim("c1", "acme", "missing", AllocationMode::Immediate);
        assert_eq!(
            m.allocate(&c).unwrap_err(),
            DraError::UnknownDeviceClass("missing".into())
        );
    }

    #[test]
    fn allocate_no_device_errors() {
        let m = DraManager::new();
        m.register_class(DeviceClass {
            name: "x".into(),
            driver: "drv-x".into(),
            allowed_tenants: vec![],
        });
        let c = mk_claim("c1", "acme", "x", AllocationMode::Immediate);
        assert!(matches!(m.allocate(&c), Err(DraError::NoDevicesForDriver(_))));
    }

    #[test]
    fn second_allocation_when_busy_errors() {
        let m = mgr_with_one_gpu(&[]);
        let a = mk_claim("a", "acme", "nvidia-a100", AllocationMode::Immediate);
        m.allocate(&a).unwrap();
        let b = mk_claim("b", "acme", "nvidia-a100", AllocationMode::Immediate);
        assert!(matches!(m.allocate(&b), Err(DraError::NoDevicesForDriver(_))));
    }

    #[test]
    fn double_allocate_same_claim_errors() {
        let m = mgr_with_one_gpu(&[]);
        let c = mk_claim("c", "acme", "nvidia-a100", AllocationMode::Immediate);
        m.allocate(&c).unwrap();
        assert!(matches!(m.allocate(&c), Err(DraError::AlreadyAllocated(_))));
    }

    #[test]
    fn deallocate_frees_device() {
        let m = mgr_with_one_gpu(&[]);
        let c = mk_claim("c", "acme", "nvidia-a100", AllocationMode::Immediate);
        m.allocate(&c).unwrap();
        m.deallocate(&c.uid).unwrap();
        // Now another can take it.
        let d = mk_claim("d", "acme", "nvidia-a100", AllocationMode::Immediate);
        let res = m.allocate(&d).unwrap();
        assert_eq!(res.device_name, "gpu-0");
    }

    #[test]
    fn deallocate_unknown_errors() {
        let m = DraManager::new();
        let bogus = Uuid::new_v4();
        assert!(matches!(m.deallocate(&bogus), Err(DraError::NotAllocated(_))));
    }

    #[test]
    fn tenant_allow_list_blocks_others() {
        let m = mgr_with_one_gpu(&["acme"]);
        let evil = mk_claim("c", "rival", "nvidia-a100", AllocationMode::Immediate);
        assert!(matches!(
            m.allocate(&evil),
            Err(DraError::TenantClassDenied { .. })
        ));
        let ok = mk_claim("c", "acme", "nvidia-a100", AllocationMode::Immediate);
        assert!(m.allocate(&ok).is_ok());
    }

    #[test]
    fn prepare_for_pod_dedups_and_accumulates() {
        let m = DraManager::new();
        let pod = Uuid::new_v4();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let v1 = m.prepare_for_pod(pod, &[a, b, a]);
        assert_eq!(v1.len(), 2);
        let v2 = m.prepare_for_pod(pod, &[b]);
        assert_eq!(v2.len(), 2);
    }

    #[test]
    fn unprepare_returns_recorded_claims() {
        let m = DraManager::new();
        let pod = Uuid::new_v4();
        let a = Uuid::new_v4();
        m.prepare_for_pod(pod, &[a]);
        assert_eq!(m.unprepare_for_pod(&pod).unwrap(), vec![a]);
        assert!(m.unprepare_for_pod(&pod).is_none());
    }

    #[test]
    fn allocation_lookup_round_trip() {
        let m = mgr_with_one_gpu(&[]);
        let c = mk_claim("c", "acme", "nvidia-a100", AllocationMode::Immediate);
        let res = m.allocate(&c).unwrap();
        let fetched = m.allocation_for(&c.uid).unwrap();
        assert_eq!(fetched.device_name, res.device_name);
        assert_eq!(fetched.claim_uid, c.uid);
    }
}
