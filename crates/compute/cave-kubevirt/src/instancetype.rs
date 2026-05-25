// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! VirtualMachineInstancetype + VirtualMachinePreference CRDs.
//!
//! Upstream: kubevirt/kubevirt v1.8.2
//!   staging/src/kubevirt.io/api/instancetype/v1beta1/types.go
//!   pkg/instancetype/instancetype.go (resolver)
//!
//! Instancetypes pin CPU/memory + machine-type sizing; preferences pin
//! soft policy (default network model, RNG, IOThreads). When a VM
//! references an instancetype/preference the controller resolves the pair
//! into a concrete `VirtualMachineInstanceSpec` overlay.

use crate::models::{
    DomainCpu, DomainMemory, InstancetypeRef, PreferenceRef, VirtualMachine,
    VirtualMachineInstanceSpec,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;

/// VirtualMachineInstancetype — pins hardware sizing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VirtualMachineInstancetype {
    pub name: String,
    pub namespace: Option<String>,
    pub spec: InstancetypeSpec,
}

impl Default for VirtualMachineInstancetype {
    fn default() -> Self {
        Self {
            name: String::new(),
            namespace: None,
            spec: InstancetypeSpec::default(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct InstancetypeSpec {
    pub cpu: CpuInstancetype,
    pub memory: MemoryInstancetype,
    pub machine_type: Option<String>,
    pub gpus: Vec<GpuRef>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CpuInstancetype {
    pub guest: u32,
    pub model: Option<String>,
    pub dedicated_cpu_placement: bool,
    pub isolate_emulator_thread: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MemoryInstancetype {
    pub guest: String,
    pub hugepages: Option<String>,
    pub overcommit_guest_overhead: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GpuRef {
    pub name: String,
    pub device_name: String,
}

/// VirtualMachinePreference — soft hardware preferences. Applied as
/// defaults the VM template can still override.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VirtualMachinePreference {
    pub name: String,
    pub namespace: Option<String>,
    pub spec: PreferenceSpec,
}

impl Default for VirtualMachinePreference {
    fn default() -> Self {
        Self {
            name: String::new(),
            namespace: None,
            spec: PreferenceSpec::default(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PreferenceSpec {
    pub preferred_cpu_topology: Option<PreferredCpuTopology>,
    pub preferred_network_interface_model: Option<String>,
    pub preferred_disk_bus: Option<String>,
    pub preferred_rng: bool,
    pub preferred_io_threads: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PreferredCpuTopology {
    Cores,
    Sockets,
    Threads,
}

/// In-memory store for both CRDs.
#[derive(Default)]
pub struct InstancetypeStore {
    instancetypes: RwLock<HashMap<String, VirtualMachineInstancetype>>,
    preferences: RwLock<HashMap<String, VirtualMachinePreference>>,
}

impl InstancetypeStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put_instancetype(&self, it: VirtualMachineInstancetype) {
        let key = format!(
            "{}/{}",
            it.namespace.clone().unwrap_or_else(|| "default".into()),
            it.name
        );
        self.instancetypes.write().unwrap().insert(key, it);
    }

    pub fn get_instancetype(&self, ns: &str, name: &str) -> Option<VirtualMachineInstancetype> {
        self.instancetypes
            .read()
            .unwrap()
            .get(&format!("{ns}/{name}"))
            .cloned()
    }

    pub fn put_preference(&self, p: VirtualMachinePreference) {
        let key = format!(
            "{}/{}",
            p.namespace.clone().unwrap_or_else(|| "default".into()),
            p.name
        );
        self.preferences.write().unwrap().insert(key, p);
    }

    pub fn get_preference(&self, ns: &str, name: &str) -> Option<VirtualMachinePreference> {
        self.preferences
            .read()
            .unwrap()
            .get(&format!("{ns}/{name}"))
            .cloned()
    }
}

/// Resolve a VM's instancetype + preference into a concrete VMI spec
/// overlay. Applies instancetype (hard) over preference (soft) over the
/// VM template. Returns the resolved spec.
pub fn resolve_vmi_spec(
    vm: &VirtualMachine,
    store: &InstancetypeStore,
) -> VirtualMachineInstanceSpec {
    let mut spec = vm.spec.template.spec.clone();
    let ns = vm.namespace.as_deref().unwrap_or("default");

    if let Some(pref) = &vm.spec.preference {
        if let Some(p) = store.get_preference(ns, &pref.name) {
            apply_preference(&p.spec, &mut spec);
        }
    }

    if let Some(it) = &vm.spec.instancetype {
        if let Some(t) = store.get_instancetype(ns, &it.name) {
            apply_instancetype(&t.spec, &mut spec);
        }
    }

    spec
}

fn apply_instancetype(it: &InstancetypeSpec, spec: &mut VirtualMachineInstanceSpec) {
    // Apply CPU (hard).
    let topology_cores = it.cpu.guest;
    spec.domain.cpu = Some(DomainCpu {
        cores: Some(topology_cores),
        sockets: Some(1),
        threads: Some(1),
        model: it.cpu.model.clone(),
    });
    // Apply memory (hard).
    spec.domain.memory = Some(DomainMemory {
        guest: Some(it.memory.guest.clone()),
        hugepages: it
            .memory
            .hugepages
            .as_ref()
            .map(|p| crate::models::HugepagesSpec {
                page_size: p.clone(),
            }),
    });
}

fn apply_preference(p: &PreferenceSpec, spec: &mut VirtualMachineInstanceSpec) {
    // Preferences only fill in *missing* values; they do not override.
    let cpu = spec.domain.cpu.get_or_insert_with(DomainCpu::default);
    if let Some(topology) = p.preferred_cpu_topology {
        // Redistribute the existing core count across the preferred axis.
        let total = cpu.cores.unwrap_or(1).max(1) as u32;
        match topology {
            PreferredCpuTopology::Cores => {
                cpu.cores = Some(total);
                cpu.sockets = Some(1);
                cpu.threads = Some(1);
            }
            PreferredCpuTopology::Sockets => {
                cpu.cores = Some(1);
                cpu.sockets = Some(total);
                cpu.threads = Some(1);
            }
            PreferredCpuTopology::Threads => {
                cpu.cores = Some(1);
                cpu.sockets = Some(1);
                cpu.threads = Some(total);
            }
        }
    }
    // Network model preference: applied to networks that have no `model`.
    if let Some(model) = &p.preferred_network_interface_model {
        for net in &mut spec.networks {
            let map = net.source.as_object_mut();
            if let Some(map) = map {
                map.entry("model")
                    .or_insert_with(|| serde_json::Value::String(model.clone()));
            }
        }
    }
}

/// Convenience: build a reference for use on a `VirtualMachineSpec`.
pub fn instancetype_ref(name: &str) -> InstancetypeRef {
    InstancetypeRef {
        name: name.into(),
        kind: Some("VirtualMachineInstancetype".into()),
        revision_name: None,
    }
}

/// Convenience: build a reference for use on a `VirtualMachineSpec`.
pub fn preference_ref(name: &str) -> PreferenceRef {
    PreferenceRef {
        name: name.into(),
        kind: Some("VirtualMachinePreference".into()),
        revision_name: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        Domain, RunStrategy, VirtualMachine, VirtualMachineInstanceTemplateSpec,
    };

    fn vm_with_refs(it: Option<&str>, pref: Option<&str>) -> VirtualMachine {
        let mut v = VirtualMachine::default();
        v.name = "vm-1".into();
        v.namespace = Some("default".into());
        v.spec.run_strategy = RunStrategy::Always;
        v.spec.instancetype = it.map(instancetype_ref);
        v.spec.preference = pref.map(preference_ref);
        v.spec.template = VirtualMachineInstanceTemplateSpec::default();
        v
    }

    #[test]
    fn store_put_get_instancetype() {
        let s = InstancetypeStore::new();
        let it = VirtualMachineInstancetype {
            name: "u1.medium".into(),
            namespace: Some("default".into()),
            spec: InstancetypeSpec {
                cpu: CpuInstancetype {
                    guest: 4,
                    ..Default::default()
                },
                memory: MemoryInstancetype {
                    guest: "4Gi".into(),
                    ..Default::default()
                },
                ..Default::default()
            },
        };
        s.put_instancetype(it.clone());
        let got = s.get_instancetype("default", "u1.medium").unwrap();
        assert_eq!(got, it);
    }

    #[test]
    fn store_put_get_preference() {
        let s = InstancetypeStore::new();
        let p = VirtualMachinePreference {
            name: "fedora".into(),
            namespace: Some("default".into()),
            spec: PreferenceSpec {
                preferred_cpu_topology: Some(PreferredCpuTopology::Sockets),
                preferred_network_interface_model: Some("virtio".into()),
                ..Default::default()
            },
        };
        s.put_preference(p.clone());
        let got = s.get_preference("default", "fedora").unwrap();
        assert_eq!(got, p);
    }

    #[test]
    fn resolve_applies_instancetype_cpu() {
        let s = InstancetypeStore::new();
        s.put_instancetype(VirtualMachineInstancetype {
            name: "u1.medium".into(),
            namespace: Some("default".into()),
            spec: InstancetypeSpec {
                cpu: CpuInstancetype {
                    guest: 4,
                    ..Default::default()
                },
                memory: MemoryInstancetype {
                    guest: "4Gi".into(),
                    ..Default::default()
                },
                ..Default::default()
            },
        });
        let vm = vm_with_refs(Some("u1.medium"), None);
        let spec = resolve_vmi_spec(&vm, &s);
        let cpu = spec.domain.cpu.unwrap();
        assert_eq!(cpu.cores, Some(4));
        assert_eq!(cpu.sockets, Some(1));
        assert_eq!(cpu.threads, Some(1));
        assert_eq!(spec.domain.memory.unwrap().guest, Some("4Gi".into()));
    }

    #[test]
    fn resolve_applies_preference_topology_sockets() {
        let s = InstancetypeStore::new();
        s.put_preference(VirtualMachinePreference {
            name: "wide".into(),
            namespace: Some("default".into()),
            spec: PreferenceSpec {
                preferred_cpu_topology: Some(PreferredCpuTopology::Sockets),
                ..Default::default()
            },
        });
        let mut vm = vm_with_refs(None, Some("wide"));
        vm.spec.template.spec.domain.cpu = Some(DomainCpu {
            cores: Some(8),
            sockets: Some(1),
            threads: Some(1),
            model: None,
        });
        let spec = resolve_vmi_spec(&vm, &s);
        let cpu = spec.domain.cpu.unwrap();
        assert_eq!(cpu.cores, Some(1));
        assert_eq!(cpu.sockets, Some(8));
    }

    #[test]
    fn resolve_preference_under_instancetype() {
        let s = InstancetypeStore::new();
        s.put_instancetype(VirtualMachineInstancetype {
            name: "u1.medium".into(),
            namespace: Some("default".into()),
            spec: InstancetypeSpec {
                cpu: CpuInstancetype {
                    guest: 4,
                    ..Default::default()
                },
                memory: MemoryInstancetype {
                    guest: "4Gi".into(),
                    ..Default::default()
                },
                ..Default::default()
            },
        });
        s.put_preference(VirtualMachinePreference {
            name: "wide".into(),
            namespace: Some("default".into()),
            spec: PreferenceSpec {
                preferred_cpu_topology: Some(PreferredCpuTopology::Sockets),
                ..Default::default()
            },
        });
        let vm = vm_with_refs(Some("u1.medium"), Some("wide"));
        let spec = resolve_vmi_spec(&vm, &s);
        // Instancetype hard-overrides preference; final cores=4, sockets=1.
        let cpu = spec.domain.cpu.unwrap();
        assert_eq!(cpu.cores, Some(4));
        assert_eq!(cpu.sockets, Some(1));
    }

    #[test]
    fn resolve_with_no_refs_returns_template() {
        let s = InstancetypeStore::new();
        let mut vm = vm_with_refs(None, None);
        vm.spec.template.spec.domain = Domain {
            cpu: Some(DomainCpu {
                cores: Some(2),
                sockets: Some(1),
                threads: Some(1),
                model: None,
            }),
            ..Default::default()
        };
        let spec = resolve_vmi_spec(&vm, &s);
        assert_eq!(spec.domain.cpu.unwrap().cores, Some(2));
    }

    #[test]
    fn resolve_with_missing_instancetype_falls_through() {
        let s = InstancetypeStore::new();
        let vm = vm_with_refs(Some("nonexistent"), None);
        let spec = resolve_vmi_spec(&vm, &s);
        assert!(spec.domain.cpu.is_none());
    }

    #[test]
    fn preference_threads_topology() {
        let s = InstancetypeStore::new();
        s.put_preference(VirtualMachinePreference {
            name: "smt".into(),
            namespace: Some("default".into()),
            spec: PreferenceSpec {
                preferred_cpu_topology: Some(PreferredCpuTopology::Threads),
                ..Default::default()
            },
        });
        let mut vm = vm_with_refs(None, Some("smt"));
        vm.spec.template.spec.domain.cpu = Some(DomainCpu {
            cores: Some(4),
            sockets: Some(1),
            threads: Some(1),
            model: None,
        });
        let spec = resolve_vmi_spec(&vm, &s);
        let cpu = spec.domain.cpu.unwrap();
        assert_eq!(cpu.threads, Some(4));
        assert_eq!(cpu.cores, Some(1));
    }

    #[test]
    fn instancetype_ref_helper() {
        let r = instancetype_ref("u1.medium");
        assert_eq!(r.name, "u1.medium");
        assert_eq!(r.kind.as_deref(), Some("VirtualMachineInstancetype"));
    }

    #[test]
    fn preference_ref_helper() {
        let r = preference_ref("fedora");
        assert_eq!(r.name, "fedora");
        assert_eq!(r.kind.as_deref(), Some("VirtualMachinePreference"));
    }

    #[test]
    fn serde_round_trip_instancetype() {
        let it = VirtualMachineInstancetype {
            name: "u1.large".into(),
            namespace: Some("test".into()),
            spec: InstancetypeSpec {
                cpu: CpuInstancetype {
                    guest: 8,
                    dedicated_cpu_placement: true,
                    ..Default::default()
                },
                memory: MemoryInstancetype {
                    guest: "16Gi".into(),
                    ..Default::default()
                },
                ..Default::default()
            },
        };
        let s = serde_json::to_string(&it).unwrap();
        let back: VirtualMachineInstancetype = serde_json::from_str(&s).unwrap();
        assert_eq!(back, it);
    }
}
