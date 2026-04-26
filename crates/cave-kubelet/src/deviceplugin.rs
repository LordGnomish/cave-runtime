//! Device plugin manager — vendor-extension hardware advertisement.
//!
//! Mirrors `pkg/kubelet/cm/devicemanager` and the device plugin gRPC
//! API (`/var/lib/kubelet/device-plugins/<sock>`): plugins register a
//! resource name + endpoint, then stream `ListAndWatch` updates with
//! per-device health. The kubelet calls `Allocate` to claim devices for a
//! container and `PreStartContainer` for runtime hooks. This module
//! captures the full state machine plus topology metadata that flows
//! into the topology manager and the pod resources API.

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Device plugin API version. K8s currently expects `v1beta1`.
pub const API_VERSION: &str = "v1beta1";
pub const KUBELET_SOCKET: &str = "kubelet.sock";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceHealth {
    Healthy,
    Unhealthy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub health: DeviceHealth,
    /// NUMA node IDs the device is local to (empty == no preference).
    pub topology_numa: Vec<i64>,
}

impl Device {
    pub fn healthy(id: &str) -> Self {
        Self {
            id: id.into(),
            health: DeviceHealth::Healthy,
            topology_numa: Vec::new(),
        }
    }

    pub fn with_numa(mut self, numa_nodes: Vec<i64>) -> Self {
        self.topology_numa = numa_nodes;
        self
    }
}

/// Plugin registration request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub version: String,
    pub endpoint: String,
    pub resource_name: String,
    pub options: PluginOptions,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginOptions {
    pub pre_start_required: bool,
    pub get_preferred_allocation_available: bool,
}

/// Resource name validation per upstream `kubernetes.io/resourcenames`:
/// must contain a domain prefix, alphanumeric (+ `.`, `-`, `_`), no
/// `kubernetes.io/` prefix unless on an allowlist.
pub fn validate_resource_name(name: &str) -> Result<(), DevicePluginError> {
    if name.is_empty() {
        return Err(DevicePluginError::Invalid("resource name empty".into()));
    }
    if !name.contains('/') {
        return Err(DevicePluginError::Invalid(format!(
            "resource name {} must include a domain prefix (e.g. 'vendor.com/gpu')",
            name
        )));
    }
    let (prefix, suffix) = name.split_once('/').unwrap();
    if prefix.is_empty() || suffix.is_empty() {
        return Err(DevicePluginError::Invalid(
            "resource name domain and path must both be non-empty".into(),
        ));
    }
    // K8s reserved domains (excluding hugepages which is handled elsewhere).
    if prefix == "kubernetes.io" || prefix == "kubernetes" {
        return Err(DevicePluginError::Invalid(format!(
            "resource name {} uses reserved kubernetes.io prefix",
            name
        )));
    }
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '/');
    if !name.chars().all(allowed) {
        return Err(DevicePluginError::Invalid(format!(
            "resource name {} contains invalid characters",
            name
        )));
    }
    Ok(())
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DevicePluginError {
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("insufficient: requested {request}, available {available}")]
    Insufficient { request: usize, available: usize },
    #[error("unhealthy: {0}")]
    Unhealthy(String),
    #[error("version mismatch: expected {expected}, got {got}")]
    VersionMismatch { expected: String, got: String },
}

pub type DpResult<T> = Result<T, DevicePluginError>;

/// Mount entry the plugin asks the kubelet to inject.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceMount {
    pub container_path: String,
    pub host_path: String,
    pub read_only: bool,
}

/// Device node entry (e.g. /dev/nvidia0).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceNode {
    pub container_path: String,
    pub host_path: String,
    pub permissions: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllocateResponse {
    pub envs: BTreeMap<String, String>,
    pub mounts: Vec<DeviceMount>,
    pub devices: Vec<DeviceNode>,
    pub annotations: BTreeMap<String, String>,
}

/// Per-resource plugin state.
#[derive(Debug, Clone)]
pub struct RegisteredPlugin {
    pub resource_name: String,
    pub endpoint: String,
    pub version: String,
    pub options: PluginOptions,
    pub registered_at: DateTime<Utc>,
    pub devices: BTreeMap<String, Device>,
    /// Allocation state: device_id → owning (pod_uid, container).
    pub allocations: BTreeMap<String, (String, String)>,
}

impl RegisteredPlugin {
    pub fn healthy_count(&self) -> usize {
        self.devices
            .values()
            .filter(|d| d.health == DeviceHealth::Healthy)
            .count()
    }

    pub fn unhealthy_count(&self) -> usize {
        self.devices
            .values()
            .filter(|d| d.health == DeviceHealth::Unhealthy)
            .count()
    }

    /// Currently free (healthy AND not allocated) devices.
    pub fn available_devices(&self) -> Vec<&Device> {
        self.devices
            .values()
            .filter(|d| d.health == DeviceHealth::Healthy && !self.allocations.contains_key(&d.id))
            .collect()
    }

    pub fn allocated_count(&self) -> usize {
        self.allocations.len()
    }
}

#[derive(Debug, Default)]
pub struct DeviceManager {
    plugins: DashMap<String, RegisteredPlugin>,
}

impl DeviceManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Process a Register RPC. Idempotent on (resource_name, endpoint).
    pub fn register(&self, req: RegisterRequest) -> DpResult<()> {
        if req.version != API_VERSION {
            return Err(DevicePluginError::VersionMismatch {
                expected: API_VERSION.into(),
                got: req.version,
            });
        }
        validate_resource_name(&req.resource_name)?;
        if req.endpoint.is_empty() {
            return Err(DevicePluginError::Invalid("endpoint empty".into()));
        }
        if let Some(existing) = self.plugins.get(&req.resource_name) {
            if existing.endpoint != req.endpoint {
                // Re-registration with a different endpoint replaces the old plugin
                // (a fresh plugin process took over). Allocations carry over only
                // if the new plugin still advertises the same device IDs.
                drop(existing);
                self.replace_plugin(&req);
                return Ok(());
            }
            return Ok(());
        }
        self.plugins.insert(
            req.resource_name.clone(),
            RegisteredPlugin {
                resource_name: req.resource_name.clone(),
                endpoint: req.endpoint,
                version: req.version,
                options: req.options,
                registered_at: Utc::now(),
                devices: BTreeMap::new(),
                allocations: BTreeMap::new(),
            },
        );
        Ok(())
    }

    fn replace_plugin(&self, req: &RegisterRequest) {
        // Preserve allocations to enable graceful re-registration.
        let prev_allocations = self
            .plugins
            .get(&req.resource_name)
            .map(|p| p.allocations.clone())
            .unwrap_or_default();
        self.plugins.insert(
            req.resource_name.clone(),
            RegisteredPlugin {
                resource_name: req.resource_name.clone(),
                endpoint: req.endpoint.clone(),
                version: req.version.clone(),
                options: req.options.clone(),
                registered_at: Utc::now(),
                devices: BTreeMap::new(),
                allocations: prev_allocations,
            },
        );
    }

    pub fn deregister(&self, resource_name: &str) -> DpResult<()> {
        self.plugins.remove(resource_name);
        Ok(())
    }

    pub fn is_registered(&self, resource_name: &str) -> bool {
        self.plugins.contains_key(resource_name)
    }

    pub fn registered_resources(&self) -> Vec<String> {
        let mut v: Vec<String> = self.plugins.iter().map(|r| r.key().clone()).collect();
        v.sort();
        v
    }

    /// Apply a `ListAndWatch` update — the plugin reports the *full* current
    /// device list each tick. Devices not in the update become stale → removed
    /// (unless allocated, in which case kubelet keeps them but marks unhealthy
    /// and lets normal cleanup release on container exit).
    pub fn list_and_watch_update(
        &self,
        resource_name: &str,
        devices: Vec<Device>,
    ) -> DpResult<()> {
        let mut plugin = self
            .plugins
            .get_mut(resource_name)
            .ok_or_else(|| DevicePluginError::NotFound(resource_name.into()))?;
        let new_ids: BTreeSet<String> = devices.iter().map(|d| d.id.clone()).collect();
        let allocated_ids: BTreeSet<String> = plugin.allocations.keys().cloned().collect();
        // Drop devices that vanished AND are not allocated.
        plugin
            .devices
            .retain(|id, _| new_ids.contains(id) || allocated_ids.contains(id));
        // Mark allocated-but-vanished devices unhealthy.
        for id in &allocated_ids {
            if !new_ids.contains(id) {
                if let Some(d) = plugin.devices.get_mut(id) {
                    d.health = DeviceHealth::Unhealthy;
                }
            }
        }
        // Upsert new/updated devices.
        for d in devices {
            plugin.devices.insert(d.id.clone(), d);
        }
        Ok(())
    }

    pub fn devices(&self, resource_name: &str) -> Vec<Device> {
        self.plugins
            .get(resource_name)
            .map(|p| p.devices.values().cloned().collect())
            .unwrap_or_default()
    }

    pub fn capacity(&self, resource_name: &str) -> usize {
        self.plugins.get(resource_name).map(|p| p.devices.len()).unwrap_or(0)
    }

    pub fn allocatable(&self, resource_name: &str) -> usize {
        self.plugins.get(resource_name).map(|p| p.healthy_count()).unwrap_or(0)
    }

    pub fn allocated(&self, resource_name: &str) -> usize {
        self.plugins.get(resource_name).map(|p| p.allocated_count()).unwrap_or(0)
    }

    /// Allocate `request` healthy, unallocated devices. Optional `prefer_numa`
    /// directs the allocator to first take devices local to the requested
    /// NUMA node set when topology hints are honoured.
    pub fn allocate(
        &self,
        resource_name: &str,
        pod_uid: &str,
        container: &str,
        request: usize,
        prefer_numa: Option<&[i64]>,
    ) -> DpResult<Vec<String>> {
        if request == 0 {
            return Ok(Vec::new());
        }
        let mut plugin = self
            .plugins
            .get_mut(resource_name)
            .ok_or_else(|| DevicePluginError::NotFound(resource_name.into()))?;

        // Idempotency: if (pod, container) already holds `request` devices, return them.
        let existing: Vec<String> = plugin
            .allocations
            .iter()
            .filter(|(_, owner)| owner.0 == pod_uid && owner.1 == container)
            .map(|(id, _)| id.clone())
            .collect();
        if !existing.is_empty() {
            if existing.len() == request {
                let mut out = existing;
                out.sort();
                return Ok(out);
            }
            return Err(DevicePluginError::Conflict(format!(
                "container {}/{} already holds {} devices (requested {})",
                pod_uid,
                container,
                existing.len(),
                request
            )));
        }

        let available: Vec<&Device> = plugin.available_devices();
        if available.len() < request {
            return Err(DevicePluginError::Insufficient {
                request,
                available: available.len(),
            });
        }

        // Pick devices: NUMA-preferred first, then anything.
        let mut chosen: Vec<String> = Vec::with_capacity(request);
        if let Some(prefer) = prefer_numa {
            let prefer: BTreeSet<i64> = prefer.iter().copied().collect();
            for d in &available {
                if chosen.len() == request {
                    break;
                }
                if d.topology_numa.iter().any(|n| prefer.contains(n)) {
                    chosen.push(d.id.clone());
                }
            }
        }
        if chosen.len() < request {
            for d in &available {
                if chosen.len() == request {
                    break;
                }
                if !chosen.contains(&d.id) {
                    chosen.push(d.id.clone());
                }
            }
        }
        chosen.sort();

        for id in &chosen {
            plugin
                .allocations
                .insert(id.clone(), (pod_uid.to_string(), container.to_string()));
        }
        Ok(chosen)
    }

    pub fn deallocate_container(&self, resource_name: &str, pod_uid: &str, container: &str) {
        if let Some(mut plugin) = self.plugins.get_mut(resource_name) {
            plugin
                .allocations
                .retain(|_, owner| !(owner.0 == pod_uid && owner.1 == container));
        }
    }

    pub fn deallocate_pod(&self, resource_name: &str, pod_uid: &str) {
        if let Some(mut plugin) = self.plugins.get_mut(resource_name) {
            plugin.allocations.retain(|_, owner| owner.0 != pod_uid);
        }
    }

    pub fn allocations_for(
        &self,
        resource_name: &str,
        pod_uid: &str,
        container: &str,
    ) -> Vec<String> {
        self.plugins
            .get(resource_name)
            .map(|p| {
                let mut v: Vec<String> = p
                    .allocations
                    .iter()
                    .filter(|(_, owner)| owner.0 == pod_uid && owner.1 == container)
                    .map(|(id, _)| id.clone())
                    .collect();
                v.sort();
                v
            })
            .unwrap_or_default()
    }

    /// Generate the topology hint set for a given resource and request count
    /// (used by the topology manager). Each healthy free device contributes
    /// a hint mask; the merged hints are de-duplicated.
    pub fn topology_hints(
        &self,
        resource_name: &str,
        request: usize,
    ) -> Vec<crate::topology::TopologyHint> {
        let plugin = match self.plugins.get(resource_name) {
            None => return Vec::new(),
            Some(p) => p,
        };
        if request == 0 {
            return Vec::new();
        }
        let avail = plugin.available_devices();
        if avail.len() < request {
            return Vec::new();
        }
        // For each subset of NUMA nodes that has at least `request` devices,
        // emit a (mask, preferred=is-single-node) hint.
        let mut by_numa: BTreeMap<i64, Vec<&Device>> = BTreeMap::new();
        let mut numa_less: Vec<&Device> = Vec::new();
        for d in &avail {
            if d.topology_numa.is_empty() {
                numa_less.push(d);
            } else {
                for n in &d.topology_numa {
                    by_numa.entry(*n).or_default().push(d);
                }
            }
        }
        let nodes: Vec<i64> = by_numa.keys().copied().collect();
        let mut hints: Vec<crate::topology::TopologyHint> = Vec::new();
        // Single-node hints first.
        for n in &nodes {
            if by_numa.get(n).unwrap().len() >= request {
                hints.push(crate::topology::TopologyHint {
                    mask: crate::topology::NumaMask::from_nodes(&[*n as u8]),
                    preferred: true,
                });
            }
        }
        // Two-node combinations (kept simple; full powerset deferred).
        for i in 0..nodes.len() {
            for j in (i + 1)..nodes.len() {
                let a = nodes[i];
                let b = nodes[j];
                let mut union: BTreeSet<&str> = BTreeSet::new();
                for d in by_numa.get(&a).unwrap() {
                    union.insert(&d.id);
                }
                for d in by_numa.get(&b).unwrap() {
                    union.insert(&d.id);
                }
                if union.len() >= request {
                    hints.push(crate::topology::TopologyHint {
                        mask: crate::topology::NumaMask::from_nodes(&[a as u8, b as u8]),
                        preferred: false,
                    });
                }
            }
        }
        // If we have NUMA-less devices and overall enough total available, add a no-pref hint.
        if !numa_less.is_empty() && avail.len() >= request {
            hints.push(crate::topology::TopologyHint {
                mask: crate::topology::NumaMask(u64::MAX),
                preferred: false,
            });
        }
        hints.sort_by_key(|h| (h.mask.0, !h.preferred));
        hints.dedup_by_key(|h| (h.mask, h.preferred));
        hints
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(name: &str, endpoint: &str) -> RegisterRequest {
        RegisterRequest {
            version: API_VERSION.into(),
            endpoint: endpoint.into(),
            resource_name: name.into(),
            options: PluginOptions::default(),
        }
    }

    #[test]
    fn validate_resource_name_requires_prefix() {
        assert!(validate_resource_name("gpu").is_err());
        assert!(validate_resource_name("nvidia.com/gpu").is_ok());
    }

    #[test]
    fn validate_resource_name_rejects_kubernetes_io() {
        assert!(validate_resource_name("kubernetes.io/cpu").is_err());
        assert!(validate_resource_name("kubernetes/foo").is_err());
    }

    #[test]
    fn validate_resource_name_rejects_empty_components() {
        assert!(validate_resource_name("/gpu").is_err());
        assert!(validate_resource_name("nvidia.com/").is_err());
    }

    #[test]
    fn validate_resource_name_rejects_invalid_chars() {
        assert!(validate_resource_name("vendor.com/gpu space").is_err());
        assert!(validate_resource_name("vendor.com/gpu#1").is_err());
    }

    #[test]
    fn validate_resource_name_accepts_complex_legit() {
        validate_resource_name("nvidia.com/gpu").unwrap();
        validate_resource_name("intel.com/sgx_epc").unwrap();
        validate_resource_name("vendor-x.example/dev-1.0").unwrap();
    }

    #[test]
    fn register_records_plugin() {
        let m = DeviceManager::new();
        m.register(req("nvidia.com/gpu", "/dev/sock1")).unwrap();
        assert!(m.is_registered("nvidia.com/gpu"));
        assert_eq!(m.registered_resources(), vec!["nvidia.com/gpu".to_string()]);
    }

    #[test]
    fn register_rejects_wrong_version() {
        let m = DeviceManager::new();
        let mut r = req("nvidia.com/gpu", "/sock");
        r.version = "v1alpha1".into();
        let err = m.register(r).unwrap_err();
        assert!(matches!(err, DevicePluginError::VersionMismatch { .. }));
    }

    #[test]
    fn register_rejects_invalid_resource_name() {
        let m = DeviceManager::new();
        let r = req("badname", "/sock");
        assert!(matches!(m.register(r), Err(DevicePluginError::Invalid(_))));
    }

    #[test]
    fn register_rejects_empty_endpoint() {
        let m = DeviceManager::new();
        let r = req("vendor.com/gpu", "");
        assert!(matches!(m.register(r), Err(DevicePluginError::Invalid(_))));
    }

    #[test]
    fn register_idempotent_for_same_endpoint() {
        let m = DeviceManager::new();
        m.register(req("vendor.com/gpu", "/sock")).unwrap();
        m.register(req("vendor.com/gpu", "/sock")).unwrap();
        assert_eq!(m.registered_resources().len(), 1);
    }

    #[test]
    fn register_replaces_when_endpoint_changes() {
        let m = DeviceManager::new();
        m.register(req("vendor.com/gpu", "/sock1")).unwrap();
        m.register(req("vendor.com/gpu", "/sock2")).unwrap();
        // Only one plugin remains.
        assert_eq!(m.registered_resources().len(), 1);
    }

    #[test]
    fn register_replacement_preserves_existing_allocations() {
        let m = DeviceManager::new();
        m.register(req("vendor.com/gpu", "/sock1")).unwrap();
        m.list_and_watch_update("vendor.com/gpu", vec![Device::healthy("g1"), Device::healthy("g2")]).unwrap();
        m.allocate("vendor.com/gpu", "p", "c", 1, None).unwrap();
        assert_eq!(m.allocated("vendor.com/gpu"), 1);
        m.register(req("vendor.com/gpu", "/sock2")).unwrap();
        assert_eq!(m.allocated("vendor.com/gpu"), 1);
    }

    #[test]
    fn deregister_removes_plugin() {
        let m = DeviceManager::new();
        m.register(req("vendor.com/gpu", "/sock")).unwrap();
        m.deregister("vendor.com/gpu").unwrap();
        assert!(!m.is_registered("vendor.com/gpu"));
    }

    #[test]
    fn list_and_watch_update_unknown_resource_errors() {
        let m = DeviceManager::new();
        let err = m.list_and_watch_update("ghost", vec![]).unwrap_err();
        assert!(matches!(err, DevicePluginError::NotFound(_)));
    }

    #[test]
    fn list_and_watch_update_records_devices() {
        let m = DeviceManager::new();
        m.register(req("vendor.com/gpu", "/s")).unwrap();
        m.list_and_watch_update(
            "vendor.com/gpu",
            vec![Device::healthy("g1"), Device::healthy("g2")],
        )
        .unwrap();
        assert_eq!(m.capacity("vendor.com/gpu"), 2);
        assert_eq!(m.allocatable("vendor.com/gpu"), 2);
    }

    #[test]
    fn list_and_watch_update_marks_unhealthy() {
        let m = DeviceManager::new();
        m.register(req("vendor.com/gpu", "/s")).unwrap();
        m.list_and_watch_update(
            "vendor.com/gpu",
            vec![Device::healthy("g1"), Device {
                id: "g2".into(),
                health: DeviceHealth::Unhealthy,
                topology_numa: vec![],
            }],
        )
        .unwrap();
        assert_eq!(m.allocatable("vendor.com/gpu"), 1);
    }

    #[test]
    fn list_and_watch_update_drops_vanished_unallocated_devices() {
        let m = DeviceManager::new();
        m.register(req("vendor.com/gpu", "/s")).unwrap();
        m.list_and_watch_update("vendor.com/gpu", vec![Device::healthy("g1"), Device::healthy("g2")])
            .unwrap();
        m.list_and_watch_update("vendor.com/gpu", vec![Device::healthy("g1")]).unwrap();
        assert_eq!(m.capacity("vendor.com/gpu"), 1);
    }

    #[test]
    fn list_and_watch_update_keeps_vanished_allocated_marks_unhealthy() {
        let m = DeviceManager::new();
        m.register(req("vendor.com/gpu", "/s")).unwrap();
        m.list_and_watch_update(
            "vendor.com/gpu",
            vec![Device::healthy("g1"), Device::healthy("g2")],
        )
        .unwrap();
        m.allocate("vendor.com/gpu", "p", "c", 1, None).unwrap();
        // Now the plugin removes both devices in its update.
        m.list_and_watch_update("vendor.com/gpu", vec![]).unwrap();
        // Allocated device kept.
        assert_eq!(m.capacity("vendor.com/gpu"), 1);
        // It's unhealthy now → no longer allocatable.
        assert_eq!(m.allocatable("vendor.com/gpu"), 0);
    }

    #[test]
    fn allocate_zero_returns_empty() {
        let m = DeviceManager::new();
        m.register(req("vendor.com/gpu", "/s")).unwrap();
        let v = m.allocate("vendor.com/gpu", "p", "c", 0, None).unwrap();
        assert!(v.is_empty());
    }

    #[test]
    fn allocate_unknown_resource_errors() {
        let m = DeviceManager::new();
        let err = m.allocate("ghost", "p", "c", 1, None).unwrap_err();
        assert!(matches!(err, DevicePluginError::NotFound(_)));
    }

    #[test]
    fn allocate_insufficient_returns_err() {
        let m = DeviceManager::new();
        m.register(req("vendor.com/gpu", "/s")).unwrap();
        m.list_and_watch_update("vendor.com/gpu", vec![Device::healthy("g1")]).unwrap();
        let err = m.allocate("vendor.com/gpu", "p", "c", 2, None).unwrap_err();
        assert!(matches!(err, DevicePluginError::Insufficient { .. }));
    }

    #[test]
    fn allocate_skips_unhealthy_devices() {
        let m = DeviceManager::new();
        m.register(req("vendor.com/gpu", "/s")).unwrap();
        m.list_and_watch_update(
            "vendor.com/gpu",
            vec![
                Device {
                    id: "g1".into(),
                    health: DeviceHealth::Unhealthy,
                    topology_numa: vec![],
                },
                Device::healthy("g2"),
            ],
        )
        .unwrap();
        let chosen = m.allocate("vendor.com/gpu", "p", "c", 1, None).unwrap();
        assert_eq!(chosen, vec!["g2".to_string()]);
    }

    #[test]
    fn allocate_idempotent_for_same_request_count() {
        let m = DeviceManager::new();
        m.register(req("vendor.com/gpu", "/s")).unwrap();
        m.list_and_watch_update("vendor.com/gpu", vec![Device::healthy("g1"), Device::healthy("g2")]).unwrap();
        let a = m.allocate("vendor.com/gpu", "p", "c", 2, None).unwrap();
        let b = m.allocate("vendor.com/gpu", "p", "c", 2, None).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn allocate_conflict_when_request_changes() {
        let m = DeviceManager::new();
        m.register(req("vendor.com/gpu", "/s")).unwrap();
        m.list_and_watch_update("vendor.com/gpu", vec![Device::healthy("g1"), Device::healthy("g2")]).unwrap();
        m.allocate("vendor.com/gpu", "p", "c", 1, None).unwrap();
        let err = m.allocate("vendor.com/gpu", "p", "c", 2, None).unwrap_err();
        assert!(matches!(err, DevicePluginError::Conflict(_)));
    }

    #[test]
    fn allocate_prefers_numa_local_devices() {
        let m = DeviceManager::new();
        m.register(req("vendor.com/gpu", "/s")).unwrap();
        m.list_and_watch_update(
            "vendor.com/gpu",
            vec![
                Device::healthy("g0").with_numa(vec![0]),
                Device::healthy("g1").with_numa(vec![1]),
            ],
        )
        .unwrap();
        let chosen = m.allocate("vendor.com/gpu", "p", "c", 1, Some(&[1])).unwrap();
        assert_eq!(chosen, vec!["g1".to_string()]);
    }

    #[test]
    fn allocate_falls_back_when_numa_pref_unmet() {
        let m = DeviceManager::new();
        m.register(req("vendor.com/gpu", "/s")).unwrap();
        m.list_and_watch_update(
            "vendor.com/gpu",
            vec![Device::healthy("g0").with_numa(vec![0])],
        )
        .unwrap();
        // Prefer NUMA 1, but there's no NUMA-1 device → fall back to g0.
        let chosen = m.allocate("vendor.com/gpu", "p", "c", 1, Some(&[1])).unwrap();
        assert_eq!(chosen, vec!["g0".to_string()]);
    }

    #[test]
    fn deallocate_container_releases_devices() {
        let m = DeviceManager::new();
        m.register(req("vendor.com/gpu", "/s")).unwrap();
        m.list_and_watch_update("vendor.com/gpu", vec![Device::healthy("g1"), Device::healthy("g2")]).unwrap();
        m.allocate("vendor.com/gpu", "p", "c", 2, None).unwrap();
        m.deallocate_container("vendor.com/gpu", "p", "c");
        assert_eq!(m.allocated("vendor.com/gpu"), 0);
    }

    #[test]
    fn deallocate_pod_releases_all_containers() {
        let m = DeviceManager::new();
        m.register(req("vendor.com/gpu", "/s")).unwrap();
        m.list_and_watch_update("vendor.com/gpu", vec![Device::healthy("g1"), Device::healthy("g2"), Device::healthy("g3")]).unwrap();
        m.allocate("vendor.com/gpu", "p", "c1", 1, None).unwrap();
        m.allocate("vendor.com/gpu", "p", "c2", 2, None).unwrap();
        m.deallocate_pod("vendor.com/gpu", "p");
        assert_eq!(m.allocated("vendor.com/gpu"), 0);
    }

    #[test]
    fn deallocate_unknown_is_noop() {
        let m = DeviceManager::new();
        m.deallocate_container("ghost", "p", "c");
        m.deallocate_pod("ghost", "p");
    }

    #[test]
    fn allocations_for_returns_sorted_ids() {
        let m = DeviceManager::new();
        m.register(req("vendor.com/gpu", "/s")).unwrap();
        m.list_and_watch_update("vendor.com/gpu", vec![Device::healthy("g3"), Device::healthy("g1"), Device::healthy("g2")]).unwrap();
        m.allocate("vendor.com/gpu", "p", "c", 3, None).unwrap();
        let v = m.allocations_for("vendor.com/gpu", "p", "c");
        assert_eq!(v, vec!["g1".to_string(), "g2".into(), "g3".into()]);
    }

    #[test]
    fn capacity_allocatable_allocated_for_unknown_resource_zero() {
        let m = DeviceManager::new();
        assert_eq!(m.capacity("ghost"), 0);
        assert_eq!(m.allocatable("ghost"), 0);
        assert_eq!(m.allocated("ghost"), 0);
    }

    #[test]
    fn topology_hints_emits_single_node_when_request_fits() {
        let m = DeviceManager::new();
        m.register(req("vendor.com/gpu", "/s")).unwrap();
        m.list_and_watch_update(
            "vendor.com/gpu",
            vec![
                Device::healthy("g0").with_numa(vec![0]),
                Device::healthy("g1").with_numa(vec![0]),
            ],
        )
        .unwrap();
        let hints = m.topology_hints("vendor.com/gpu", 1);
        assert!(hints.iter().any(|h| h.preferred && h.mask == crate::topology::NumaMask::from_nodes(&[0])));
    }

    #[test]
    fn topology_hints_emits_multi_node_when_no_single_fits() {
        let m = DeviceManager::new();
        m.register(req("vendor.com/gpu", "/s")).unwrap();
        m.list_and_watch_update(
            "vendor.com/gpu",
            vec![
                Device::healthy("g0").with_numa(vec![0]),
                Device::healthy("g1").with_numa(vec![1]),
            ],
        )
        .unwrap();
        let hints = m.topology_hints("vendor.com/gpu", 2);
        assert!(hints.iter().any(|h| !h.preferred && h.mask == crate::topology::NumaMask::from_nodes(&[0, 1])));
        assert!(!hints.iter().any(|h| h.preferred));
    }

    #[test]
    fn topology_hints_returns_empty_when_insufficient() {
        let m = DeviceManager::new();
        m.register(req("vendor.com/gpu", "/s")).unwrap();
        m.list_and_watch_update("vendor.com/gpu", vec![Device::healthy("g0")]).unwrap();
        assert!(m.topology_hints("vendor.com/gpu", 5).is_empty());
    }

    #[test]
    fn topology_hints_zero_request_empty() {
        let m = DeviceManager::new();
        m.register(req("vendor.com/gpu", "/s")).unwrap();
        m.list_and_watch_update("vendor.com/gpu", vec![Device::healthy("g0")]).unwrap();
        assert!(m.topology_hints("vendor.com/gpu", 0).is_empty());
    }

    #[test]
    fn topology_hints_unknown_resource_empty() {
        let m = DeviceManager::new();
        assert!(m.topology_hints("ghost", 1).is_empty());
    }

    #[test]
    fn topology_hints_numa_less_devices_yield_no_pref_hint() {
        let m = DeviceManager::new();
        m.register(req("vendor.com/gpu", "/s")).unwrap();
        m.list_and_watch_update(
            "vendor.com/gpu",
            vec![Device::healthy("g0"), Device::healthy("g1")],
        )
        .unwrap();
        let hints = m.topology_hints("vendor.com/gpu", 1);
        assert!(hints.iter().any(|h| h.mask == crate::topology::NumaMask(u64::MAX)));
    }

    #[test]
    fn allocate_subsequent_pod_gets_separate_devices() {
        let m = DeviceManager::new();
        m.register(req("vendor.com/gpu", "/s")).unwrap();
        m.list_and_watch_update(
            "vendor.com/gpu",
            vec![Device::healthy("g1"), Device::healthy("g2")],
        )
        .unwrap();
        let a = m.allocate("vendor.com/gpu", "p1", "c", 1, None).unwrap();
        let b = m.allocate("vendor.com/gpu", "p2", "c", 1, None).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn devices_returns_clone() {
        let m = DeviceManager::new();
        m.register(req("vendor.com/gpu", "/s")).unwrap();
        m.list_and_watch_update("vendor.com/gpu", vec![Device::healthy("g1")]).unwrap();
        let v = m.devices("vendor.com/gpu");
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn registered_resources_sorted() {
        let m = DeviceManager::new();
        m.register(req("z.example/q", "/s")).unwrap();
        m.register(req("a.example/q", "/s")).unwrap();
        let v = m.registered_resources();
        assert_eq!(v, vec!["a.example/q".to_string(), "z.example/q".into()]);
    }

    #[test]
    fn plugin_options_default_false() {
        let o = PluginOptions::default();
        assert!(!o.pre_start_required);
        assert!(!o.get_preferred_allocation_available);
    }
}
