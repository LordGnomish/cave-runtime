// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! KubeVirt v1 CRD models.
//!
//! Upstream: kubevirt/kubevirt v1.8.2
//!   staging/src/kubevirt.io/api/core/v1/types.go        → VirtualMachine, VirtualMachineInstance
//!   staging/src/kubevirt.io/containerized-data-importer/api/.../v1beta1/types.go → DataVolume

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VirtualMachine {
    pub name: String,
    pub namespace: Option<String>,
    pub spec: VirtualMachineSpec,
    pub status: Option<VirtualMachineStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VirtualMachineSpec {
    pub run_strategy: RunStrategy,
    pub instancetype: Option<InstancetypeRef>,
    pub preference: Option<PreferenceRef>,
    pub template: VirtualMachineInstanceTemplateSpec,
    pub data_volume_templates: Vec<DataVolumeTemplate>,
}

/// KubeVirt RunStrategy controls VM lifecycle. Upstream: api/core/v1/schema.go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RunStrategy {
    Always,
    #[default]
    Halted,
    Manual,
    RerunOnFailure,
    Once,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VirtualMachineStatus {
    pub printable_status: String,
    pub ready: bool,
    pub created: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstancetypeRef {
    pub name: String,
    pub kind: Option<String>,
    pub revision_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PreferenceRef {
    pub name: String,
    pub kind: Option<String>,
    pub revision_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VirtualMachineInstanceTemplateSpec {
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub spec: VirtualMachineInstanceSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VirtualMachineInstance {
    pub name: String,
    pub namespace: Option<String>,
    /// Set by the API server when a delete is issued; the VMI lingers while
    /// finalizers run. Mirrors `ObjectMeta.DeletionTimestamp` (Unix seconds).
    pub deletion_timestamp: Option<i64>,
    pub spec: VirtualMachineInstanceSpec,
    pub status: Option<VirtualMachineInstanceStatus>,
}

impl VirtualMachineInstance {
    /// Current phase string, defaulting to the upstream "unset" sentinel ("").
    fn phase(&self) -> &str {
        self.status.as_ref().map(|s| s.phase.as_str()).unwrap_or("")
    }

    /// `vmi.IsUnprocessed()` — no phase yet or still Pending.
    pub fn is_unprocessed(&self) -> bool {
        matches!(self.phase(), "" | "Pending")
    }

    /// `vmi.IsScheduling()`.
    pub fn is_scheduling(&self) -> bool {
        self.phase() == "Scheduling"
    }

    /// `vmi.IsScheduled()`.
    pub fn is_scheduled(&self) -> bool {
        self.phase() == "Scheduled"
    }

    /// `vmi.IsRunning()`.
    pub fn is_running(&self) -> bool {
        self.phase() == "Running"
    }

    /// `vmi.IsFinal()` — reached a terminal phase (Succeeded or Failed).
    pub fn is_final(&self) -> bool {
        matches!(self.phase(), "Succeeded" | "Failed")
    }

    /// `vmi.IsMarkedForDeletion()`.
    pub fn is_marked_for_deletion(&self) -> bool {
        self.deletion_timestamp.is_some()
    }

    /// True iff a condition of the given type is present with the given status.
    pub fn has_condition(&self, kind: &str, status: &str) -> bool {
        self.status
            .as_ref()
            .map(|s| {
                s.conditions
                    .iter()
                    .any(|c| c.kind == kind && c.status == status)
            })
            .unwrap_or(false)
    }

    /// `hasPausedCondition` — VMI carries a `Paused=True` condition.
    pub fn has_paused_condition(&self) -> bool {
        self.has_condition("Paused", "True")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VirtualMachineInstanceSpec {
    pub domain: Domain,
    pub volumes: Vec<Volume>,
    pub networks: Vec<Network>,
    pub termination_grace_period_seconds: Option<i64>,
    pub eviction_strategy: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VirtualMachineInstanceStatus {
    pub phase: String,
    pub node_name: Option<String>,
    pub interfaces: Vec<NetworkInterfaceStatus>,
    pub conditions: Vec<Condition>,
}

/// Domain shape from libvirt — kept opaque for the scaffold.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Domain {
    pub cpu: Option<DomainCpu>,
    pub memory: Option<DomainMemory>,
    pub devices: serde_json::Value,
    pub firmware: Option<Firmware>,
    pub features: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DomainCpu {
    pub cores: Option<u32>,
    pub sockets: Option<u32>,
    pub threads: Option<u32>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DomainMemory {
    pub guest: Option<String>,
    pub hugepages: Option<HugepagesSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HugepagesSpec {
    pub page_size: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Firmware {
    pub uuid: Option<String>,
    pub bootloader: Option<String>,
    pub serial: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Volume {
    pub name: String,
    pub source: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Network {
    pub name: String,
    pub source: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NetworkInterfaceStatus {
    pub name: String,
    pub mac: Option<String>,
    pub ip_address: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Condition {
    pub kind: String,
    pub status: String,
    pub reason: Option<String>,
    pub message: Option<String>,
}

/// CDI DataVolume — sidecar PVC populator.
/// Upstream: containerized-data-importer/.../v1beta1/types.go
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DataVolume {
    pub name: String,
    pub namespace: Option<String>,
    pub spec: DataVolumeSpec,
    pub status: Option<DataVolumeStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DataVolumeSpec {
    pub source: DataVolumeSource,
    pub pvc: PvcSpec,
    pub priority_class_name: Option<String>,
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DataVolumeSource {
    pub kind: String,
    pub spec: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PvcSpec {
    pub access_modes: Vec<String>,
    pub storage_class: Option<String>,
    pub size: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DataVolumeTemplate {
    pub metadata_name: String,
    pub spec: DataVolumeSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DataVolumeStatus {
    pub phase: String,
    pub progress: Option<String>,
}

/// VM phase enumeration (upstream's PrintableStatus values + VMI Phase values).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VmPhase {
    Stopped,
    Starting,
    Running,
    Migrating,
    Stopping,
    Terminating,
    Error,
}

#[cfg(test)]
mod vmi_predicate_tests {
    use super::*;

    fn vmi_with(phase: &str) -> VirtualMachineInstance {
        let mut v = VirtualMachineInstance::default();
        v.status = Some(VirtualMachineInstanceStatus {
            phase: phase.into(),
            ..Default::default()
        });
        v
    }

    #[test]
    fn unprocessed_covers_unset_and_pending() {
        assert!(VirtualMachineInstance::default().is_unprocessed());
        assert!(vmi_with("Pending").is_unprocessed());
        assert!(!vmi_with("Scheduling").is_unprocessed());
        assert!(!vmi_with("Running").is_unprocessed());
    }

    #[test]
    fn scheduling_and_scheduled_are_distinct() {
        assert!(vmi_with("Scheduling").is_scheduling());
        assert!(!vmi_with("Scheduling").is_scheduled());
        assert!(vmi_with("Scheduled").is_scheduled());
        assert!(!vmi_with("Scheduled").is_scheduling());
    }

    #[test]
    fn running_predicate() {
        assert!(vmi_with("Running").is_running());
        assert!(!vmi_with("Pending").is_running());
    }

    #[test]
    fn final_covers_succeeded_and_failed_only() {
        assert!(vmi_with("Succeeded").is_final());
        assert!(vmi_with("Failed").is_final());
        assert!(!vmi_with("Running").is_final());
        assert!(!vmi_with("Unknown").is_final());
    }

    #[test]
    fn marked_for_deletion_tracks_timestamp() {
        let mut v = vmi_with("Running");
        assert!(!v.is_marked_for_deletion());
        v.deletion_timestamp = Some(1_780_000_000);
        assert!(v.is_marked_for_deletion());
    }

    #[test]
    fn paused_condition_detection() {
        let mut v = vmi_with("Running");
        assert!(!v.has_paused_condition());
        v.status.as_mut().unwrap().conditions.push(Condition {
            kind: "Paused".into(),
            status: "True".into(),
            reason: None,
            message: None,
        });
        assert!(v.has_paused_condition());
        assert!(v.has_condition("Paused", "True"));
        // A Paused=False condition must NOT count as paused.
        assert!(!v.has_condition("Paused", "False"));
    }
}
