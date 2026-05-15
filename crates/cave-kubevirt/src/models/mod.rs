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
    pub spec: VirtualMachineInstanceSpec,
    pub status: Option<VirtualMachineInstanceStatus>,
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
