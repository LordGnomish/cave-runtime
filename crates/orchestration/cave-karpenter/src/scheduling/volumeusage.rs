// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Volume-limit tracking — port of the portable core of
//! `pkg/scheduling/volumeusage.go` from kubernetes-sigs/karpenter v1.12.1
//! (sha ed490e8).
//!
//! Upstream tracks how many CSI volumes (keyed by storage-driver /
//! provisioner name) a candidate node would mount, so the scheduler can
//! reject pods that would push a node past its per-driver attachment limit.
//!
//! Ported here: the two pure data structures the scheduler relies on —
//! [`Volumes`] (a `driver -> set<pvcID>` map with set-union semantics) and
//! [`VolumeUsage`] (the per-node limit tracker). The k8s-client-bound
//! resolvers `GetVolumes` / `ResolveDriver` / `driverFromSC` /
//! `driverFromVolume` are scope-cut per
//! ADR-RUNTIME-KARPENTER-CLOUD-AGNOSTIC-001 — they need a live
//! controller-runtime client plus the CSI translation library and carry no
//! cloud-agnostic behaviour.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// `Volumes` maps a storage-driver / provisioner name to the set of PVC
/// identifiers that resolve to it. Mirrors upstream `Volumes
/// map[string]sets.Set[string]`; sets give us automatic de-duplication of
/// volume names that appear across pods.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Volumes(BTreeMap<String, BTreeSet<String>>);

impl Volumes {
    /// Empty map.
    pub fn new() -> Self {
        Volumes(BTreeMap::new())
    }

    /// Insert `pvc_id` into the set tracked for `provisioner`, creating the
    /// set on first use. Upstream `Add`.
    pub fn add(&mut self, provisioner: &str, pvc_id: &str) {
        self.0
            .entry(provisioner.to_string())
            .or_default()
            .insert(pvc_id.to_string());
    }

    /// Return a NEW `Volumes` containing the per-driver set-union of `self`
    /// and `other`, leaving both operands untouched. Upstream `Union`.
    pub fn union(&self, other: &Volumes) -> Volumes {
        let mut cp = self.clone();
        cp.insert(other);
        cp
    }

    /// Merge `other` into `self` in place (per-driver set-union). Upstream
    /// `Insert`.
    pub fn insert(&mut self, other: &Volumes) {
        for (driver, set) in &other.0 {
            let existing = self.0.entry(driver.clone()).or_default();
            for pvc in set {
                existing.insert(pvc.clone());
            }
        }
    }

    /// Number of distinct PVCs tracked for `provisioner` (0 if unknown).
    pub fn count(&self, provisioner: &str) -> usize {
        self.0.get(provisioner).map_or(0, BTreeSet::len)
    }

    /// Whether `pvc_id` is tracked under `provisioner`.
    pub fn contains(&self, provisioner: &str, pvc_id: &str) -> bool {
        self.0
            .get(provisioner)
            .is_some_and(|set| set.contains(pvc_id))
    }

    /// Iterate `(driver, &set)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &BTreeSet<String>)> {
        self.0.iter()
    }
}

/// Raised when a candidate set of volumes would push a driver past its
/// registered attachment limit. Mirrors the `serrors.Wrap("would exceed
/// volume limit", ...)` fields upstream returns from `ExceedsLimits`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeLimitExceeded {
    pub provisioner: String,
    pub volume_count: usize,
    pub volume_limit: usize,
}

impl fmt::Display for VolumeLimitExceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "would exceed volume limit provisioner={} volume-count={} volume-limit={}",
            self.provisioner, self.volume_count, self.volume_limit
        )
    }
}

impl std::error::Error for VolumeLimitExceeded {}

/// Per-node volume-limit tracker. Mirrors upstream `VolumeUsage`:
///   * `volumes`     — aggregated usage across all tracked pods
///   * `pod_volumes` — per-pod contribution (keyed by namespaced name)
///   * `limits`      — per-driver attachment ceiling
#[derive(Debug, Clone, Default)]
pub struct VolumeUsage {
    volumes: Volumes,
    pod_volumes: BTreeMap<String, Volumes>,
    limits: BTreeMap<String, usize>,
}

impl VolumeUsage {
    /// Empty tracker. Upstream `NewVolumeUsage`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register the attachment ceiling for a storage driver. Upstream
    /// `AddLimit`.
    pub fn add_limit(&mut self, storage_driver: &str, value: usize) {
        self.limits.insert(storage_driver.to_string(), value);
    }

    /// Record a pod's volumes and fold them into the aggregate. Upstream
    /// `Add`.
    pub fn add(&mut self, pod_key: &str, volumes: Volumes) {
        self.volumes = self.volumes.union(&volumes);
        self.pod_volumes.insert(pod_key.to_string(), volumes);
    }

    /// Return `Err` if unioning `vols` with the already-tracked usage would
    /// push any driver strictly past its limit. Drivers without a registered
    /// limit are unbounded. Upstream `ExceedsLimits` (guard is `len > limit`,
    /// so being exactly at the limit is permitted).
    pub fn exceeds_limits(&self, vols: &Volumes) -> Result<(), VolumeLimitExceeded> {
        let combined = self.volumes.union(vols);
        for (driver, set) in combined.iter() {
            if let Some(&limit) = self.limits.get(driver) {
                if set.len() > limit {
                    return Err(VolumeLimitExceeded {
                        provisioner: driver.clone(),
                        volume_count: set.len(),
                        volume_limit: limit,
                    });
                }
            }
        }
        Ok(())
    }

    /// Drop a pod's contribution and rebuild the aggregate from the survivors.
    /// Volume names can be shared across pods, so upstream re-creates the
    /// aggregate from scratch rather than subtracting. Upstream `DeletePod`.
    pub fn delete_pod(&mut self, pod_key: &str) {
        self.pod_volumes.remove(pod_key);
        let mut rebuilt = Volumes::new();
        for contribution in self.pod_volumes.values() {
            rebuilt.insert(contribution);
        }
        self.volumes = rebuilt;
    }

    /// Read-only view of the aggregated usage.
    pub fn volumes(&self) -> &Volumes {
        &self.volumes
    }
}
